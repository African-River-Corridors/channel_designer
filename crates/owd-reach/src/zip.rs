//! Members of a big remote zip, read by range requests over [`Fetch`]: the end record, the central
//! directory, then only the members asked for. This is how the SWORD release on Zenodo (one 3.8 GB zip
//! of shapefiles) gives up the one basin a reach needs. What is read: stored and DEFLATE members of a
//! zip that is not zip64 (under 4 GiB, as the SWORD release is). Anything else is refused by name.

use crate::error::ReachError;
use crate::fetch::Fetch;

/// One member, from the central directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    /// 0 stored, 8 DEFLATE.
    pub method: u16,
    pub compressed: u64,
    pub size: u64,
    /// Offset of the member's local header.
    pub offset: u64,
}

/// A remote zip, its directory read.
#[derive(Debug, Clone)]
pub struct RemoteZip {
    pub url: String,
    pub members: Vec<Member>,
}

fn bad(url: &str, what: &str) -> ReachError {
    ReachError::Parse(format!("{url}: {what}"))
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The longest end record: 22 bytes and a comment of up to 65535.
const TAIL: u64 = 22 + 65_535;

impl RemoteZip {
    /// Read the directory of the zip at `url`, which is `size` bytes long.
    pub fn open(fetch: &dyn Fetch, url: &str, size: u64) -> Result<RemoteZip, ReachError> {
        let from = size.saturating_sub(TAIL);
        let tail = fetch.get_range(url, from, size - from)?;
        let at = (0..tail.len().saturating_sub(21))
            .rev()
            .find(|i| tail[*i..*i + 4] == *b"PK\x05\x06")
            .ok_or_else(|| bad(url, "no zip end record (not a zip?)"))?;
        let (count, cd_size, cd_off) = (
            u16_at(&tail, at + 10),
            u32_at(&tail, at + 12),
            u32_at(&tail, at + 16),
        );
        if count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF {
            return Err(bad(url, "a zip64 file is not read"));
        }
        let cd = fetch.get_range(url, u64::from(cd_off), u64::from(cd_size))?;
        let mut members = Vec::with_capacity(usize::from(count));
        let mut p = 0;
        while p + 46 <= cd.len() && cd[p..p + 4] == *b"PK\x01\x02" {
            let (nl, el, cl) = (
                usize::from(u16_at(&cd, p + 28)),
                usize::from(u16_at(&cd, p + 30)),
                usize::from(u16_at(&cd, p + 32)),
            );
            let name = cd
                .get(p + 46..p + 46 + nl)
                .ok_or_else(|| bad(url, "the central directory is cut short"))?;
            members.push(Member {
                name: String::from_utf8_lossy(name).into_owned(),
                method: u16_at(&cd, p + 10),
                compressed: u64::from(u32_at(&cd, p + 20)),
                size: u64::from(u32_at(&cd, p + 24)),
                offset: u64::from(u32_at(&cd, p + 42)),
            });
            p += 46 + nl + el + cl;
        }
        if members.len() != usize::from(count) {
            return Err(bad(
                url,
                "the central directory does not hold the members it counts",
            ));
        }
        Ok(RemoteZip {
            url: url.to_string(),
            members,
        })
    }

    /// A member by name.
    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.name == name)
    }

    /// Where a member's data starts: after its local header.
    fn data_offset(&self, fetch: &dyn Fetch, m: &Member) -> Result<u64, ReachError> {
        let h = fetch.get_range(&self.url, m.offset, 30)?;
        if h.len() < 30 || h[..4] != *b"PK\x03\x04" {
            return Err(bad(&self.url, &format!("no local header for {}", m.name)));
        }
        Ok(m.offset + 30 + u64::from(u16_at(&h, 26)) + u64::from(u16_at(&h, 28)))
    }

    /// The whole of a member, uncompressed.
    pub fn read(&self, fetch: &dyn Fetch, m: &Member) -> Result<Vec<u8>, ReachError> {
        let start = self.data_offset(fetch, m)?;
        let raw = fetch.get_range(&self.url, start, m.compressed)?;
        let out = match m.method {
            0 => raw,
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(&raw, m.size as usize)
                .map_err(|e| {
                    bad(
                        &self.url,
                        &format!("{}: DEFLATE data is damaged ({e:?})", m.name),
                    )
                })?,
            k => {
                return Err(bad(
                    &self.url,
                    &format!("{}: compression {k} is not read", m.name),
                ));
            }
        };
        if out.len() as u64 != m.size {
            return Err(bad(
                &self.url,
                &format!("{}: {} bytes, not {}", m.name, out.len(), m.size),
            ));
        }
        Ok(out)
    }

    /// The first `want` bytes of a member, uncompressed, reading only as much of it as they need (a
    /// shapefile's header gives its bounding box in its first 100 bytes). The local header and the
    /// first compressed bytes come in one request.
    pub fn read_prefix(
        &self,
        fetch: &dyn Fetch,
        m: &Member,
        want: usize,
    ) -> Result<Vec<u8>, ReachError> {
        use miniz_oxide::inflate::stream::{InflateState, inflate};
        use miniz_oxide::{DataFormat, MZFlush};
        const CHUNK: u64 = 4096;
        // The local header's name is the central one's; its extra field may differ, so allow for one.
        let head_len = 30 + m.name.len() as u64 + 512;
        let first = fetch.get_range(&self.url, m.offset, head_len + CHUNK)?;
        if first.len() < 30 || first[..4] != *b"PK\x03\x04" {
            return Err(bad(&self.url, &format!("no local header for {}", m.name)));
        }
        let skip = 30 + usize::from(u16_at(&first, 26)) + usize::from(u16_at(&first, 28));
        let start = m.offset + skip as u64;
        let end = start + m.compressed;
        let mut input: Vec<u8> = first.get(skip..).unwrap_or_default().to_vec();
        input.truncate(m.compressed as usize);
        let mut read_to = start + input.len() as u64;
        if m.method == 0 {
            input.truncate(want.min(m.size as usize));
            return Ok(input);
        }
        if m.method != 8 {
            return Err(bad(
                &self.url,
                &format!("{}: compression {} is not read", m.name, m.method),
            ));
        }
        let mut state = InflateState::new_boxed(DataFormat::Raw);
        let mut out = vec![0u8; want];
        let mut got = 0;
        loop {
            let mut used = 0;
            while used < input.len() && got < want {
                let r = inflate(&mut state, &input[used..], &mut out[got..], MZFlush::None);
                if r.status.is_err() {
                    return Err(bad(
                        &self.url,
                        &format!("{}: DEFLATE data is damaged", m.name),
                    ));
                }
                used += r.bytes_consumed;
                got += r.bytes_written;
                if r.bytes_consumed == 0 && r.bytes_written == 0 {
                    break;
                }
            }
            if got >= want || read_to >= end {
                break;
            }
            let len = CHUNK.min(end - read_to);
            input = fetch.get_range(&self.url, read_to, len)?;
            read_to += len;
        }
        out.truncate(got);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::fetch::ReplayFetch;

    const SUBSET: &[u8] = include_bytes!("../../../examples/reach/sword_v17c_shp_subset.zip");

    #[test]
    fn the_members_of_a_zip_read_back_whole_and_by_prefix() {
        let url = "https://example.org/sword.zip";
        let f = ReplayFetch::new().with_response(url, SUBSET.to_vec());
        let z = RemoteZip::open(&f, url, SUBSET.len() as u64).unwrap();
        let names: Vec<&str> = z.members.iter().map(|m| m.name.as_str()).collect();
        assert!(
            names.contains(&"shp/af_sword_reaches_hb14_v17c.shp"),
            "{names:?}"
        );
        let m = z.member("shp/af_sword_reaches_hb14_v17c.shp").unwrap();
        assert_eq!(m.method, 8);
        let whole = z.read(&f, m).unwrap();
        assert_eq!(whole.len() as u64, m.size);
        // A shapefile starts with the file code 9994, big-endian.
        assert_eq!(whole[..4], 9994i32.to_be_bytes());
        let head = z.read_prefix(&f, m, 100).unwrap();
        assert_eq!(head, whole[..100]);
        // Not a zip: refused by name.
        let g = ReplayFetch::new().with_response(url, vec![0u8; 200]);
        assert!(
            RemoteZip::open(&g, url, 200)
                .unwrap_err()
                .to_string()
                .contains("not a zip")
        );
    }
}
