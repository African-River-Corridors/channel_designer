//! The QA overlay (spec 0013, ticket T5): the bank lines, the centreline and the low-confidence
//! stretches drawn over the Sentinel-2 true-colour image of the scene(s) used, so a person can check a
//! reach by eye. Pure: it takes the lines and the scene windows and gives a PNG and a KMZ as bytes; the
//! CLI and the app write the files.
//!
//! - **The image.** True colour (red B04, green B03, blue B02), each band stretched from its 2nd to its
//!   98th percentile over the window, on the scene's 10 m grid. Where the first scene has no data, the
//!   next fills in. Cloud is shown as it is: the picture is for checking, not for measuring.
//! - **The lines.** Left bank vermilion, right bank teal, the centreline cream, each cased in ink, and
//!   each low-confidence stretch as a wide band along the centreline: repairs in the fail colour, the
//!   rest in the warn colour (the style guide's tokens, dark-theme values for legibility on imagery).
//! - **The legend,** under the image: the swatches, the scene ids and every notice.
//! - **The KMZ:** `doc.kml` with a ground overlay placed by its four corners (`gx:LatLonQuad`, exact
//!   for a UTM image, which a north-up `LatLonBox` is not), the lines as KML line strings with the
//!   same colours, and the notices in the description; and the image, `overlay.png`.

use crate::banks::StretchKind;
use crate::error::ReachError;
use crate::sentinel2::Imagery;
use owd_format::design::crs::Utm;
use owd_format::design::midline::{arc, point_at};

type P2 = [f64; 2];

/// Colours, RGBA.
pub const LEFT: [u8; 4] = [0xee, 0xa5, 0x94, 255];
pub const RIGHT: [u8; 4] = [0x9f, 0xca, 0xd1, 255];
pub const CENTRE: [u8; 4] = [0xf2, 0xe8, 0xcf, 255];
pub const CASING: [u8; 4] = [0x2b, 0x29, 0x25, 255];
pub const REPAIRED: [u8; 4] = [0xff, 0x5a, 0x6a, 150];
pub const LOW: [u8; 4] = [0xfb, 0xd8, 0x6e, 150];
const PAPER: [u8; 4] = [0xf2, 0xe8, 0xcf, 255];
const INK: [u8; 4] = [0x2b, 0x29, 0x25, 255];

/// A low-confidence stretch, by chainage along the centreline.
#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    pub from_m: f64,
    pub to_m: f64,
    /// The document's name of the kind (`repaired`, `no-edge`, ...).
    pub kind: String,
}

impl Flag {
    pub fn from_stretch(s: &crate::banks::Stretch) -> Flag {
        Flag {
            from_m: s.from_m,
            to_m: s.to_m,
            kind: s.kind.name().to_string(),
        }
    }

    fn colour(&self) -> [u8; 4] {
        if self.kind == StretchKind::Repaired(Default::default()).name() {
            REPAIRED
        } else {
            LOW
        }
    }
}

/// What is drawn, in the scene's UTM zone.
#[derive(Debug, Clone, Default)]
pub struct Lines {
    pub left: Vec<P2>,
    pub right: Vec<P2>,
    pub centre: Vec<P2>,
    pub flags: Vec<Flag>,
}

impl Lines {
    /// The part between two chainages of the centreline: the centreline cut there, the bank vertices
    /// whose nearest centreline point lies between them, and the flags that overlap, moved to start
    /// at the cut.
    pub fn crop(&self, from_m: f64, to_m: f64) -> Lines {
        let s = arc(&self.centre);
        let (a, b) = (from_m.max(0.0), to_m.min(*s.last().unwrap_or(&0.0)));
        let mut centre = vec![point_at(&self.centre, &s, a)];
        centre.extend(
            self.centre
                .iter()
                .zip(&s)
                .filter(|(_, c)| **c > a && **c < b)
                .map(|(p, _)| *p),
        );
        centre.push(point_at(&self.centre, &s, b));
        let keep = |bank: &[P2]| -> Vec<P2> {
            bank.iter()
                .filter(|p| {
                    let c = crate::bankcheck::chainage_on(&self.centre, &s, **p);
                    c >= a && c <= b
                })
                .copied()
                .collect()
        };
        Lines {
            left: keep(&self.left),
            right: keep(&self.right),
            centre,
            flags: self
                .flags
                .iter()
                .filter(|f| f.to_m > a && f.from_m < b)
                .map(|f| Flag {
                    from_m: f.from_m.max(a) - a,
                    to_m: f.to_m.min(b) - a,
                    kind: f.kind.clone(),
                })
                .collect(),
        }
    }

    /// The box of every line, `(min, max)`.
    pub fn bounds(&self) -> Option<(P2, P2)> {
        let mut it = self.left.iter().chain(&self.right).chain(&self.centre);
        let first = *it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        }))
    }
}

/// An RGBA picture on a UTM grid.
#[derive(Debug, Clone)]
pub struct Picture {
    pub utm: Utm,
    pub x0: f64,
    pub y1: f64,
    pub px: f64,
    pub width: usize,
    /// The rows of the map; in a picture with a legend, the legend rows follow them.
    pub map_rows: usize,
    /// The columns of the map; a legend may be wider.
    pub map_cols: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// A scene window and its blue band, for the picture.
pub struct Shot<'a> {
    pub imagery: &'a Imagery,
    pub blue: &'a [f32],
}

fn percentile(mut v: Vec<f32>, q: f64) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// The true-colour picture of the windows (the first window's grid; later ones fill its no-data
/// pixels and must share the grid), with no lines yet.
pub fn true_colour(shots: &[Shot]) -> Result<Picture, ReachError> {
    let first = shots
        .first()
        .ok_or_else(|| ReachError::Input("no scene to draw".into()))?
        .imagery;
    let (w, h) = (first.cols, first.rows);
    for s in shots {
        let im = s.imagery;
        if im.utm != first.utm || im.cols != w || im.rows != h || s.blue.len() != w * h {
            return Err(ReachError::Input(
                "the scenes of one picture must share a window".into(),
            ));
        }
    }
    // Per pixel, the first scene with data there (a band at zero is no data).
    let mut from: Vec<Option<usize>> = vec![None; w * h];
    for (i, f) in from.iter_mut().enumerate() {
        *f = shots.iter().position(|s| {
            s.imagery.bands[0][i] > 0.0 && s.imagery.bands[1][i] > 0.0 && s.blue[i] > 0.0
        });
    }
    let band = |i: usize, k: usize| -> Option<f32> {
        from[i].map(|s| {
            if k == 2 {
                shots[s].blue[i]
            } else {
                shots[s].imagery.bands[k][i]
            }
        })
    };
    // Red, green, blue: bands 1, 0 and the blue band.
    let order = [1usize, 0, 2];
    let mut lohi = [[0f32; 2]; 3];
    for (c, k) in order.iter().enumerate() {
        let v: Vec<f32> = (0..w * h).filter_map(|i| band(i, *k)).collect();
        lohi[c] = [percentile(v.clone(), 0.02), percentile(v, 0.98)];
    }
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..w * h {
        for (c, k) in order.iter().enumerate() {
            let v = band(i, *k).map_or(0.0, |v| {
                let [lo, hi] = lohi[c];
                ((v - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0)
            });
            rgba[i * 4 + c] = (v * 255.0).round() as u8;
        }
        rgba[i * 4 + 3] = 255;
    }
    Ok(Picture {
        utm: first.utm,
        x0: first.x0,
        y1: first.y1,
        px: first.px,
        width: w,
        map_rows: h,
        map_cols: w,
        height: h,
        rgba,
    })
}

impl Picture {
    /// The map at `k` times the pixels each way (each pixel repeated), so lines drawn on a small
    /// window stay thin against the river.
    pub fn upscale(&self, k: usize) -> Picture {
        let k = k.max(1);
        let (w, h) = (self.map_cols * k, self.map_rows * k);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = ((y / k) * self.width + x / k) * 4;
                let j = (y * w + x) * 4;
                rgba[j..j + 4].copy_from_slice(&self.rgba[i..i + 4]);
            }
        }
        Picture {
            px: self.px / k as f64,
            width: w,
            map_rows: h,
            map_cols: w,
            height: h,
            rgba,
            ..self.clone()
        }
    }

    fn to_px(&self, p: P2) -> [f64; 2] {
        [(p[0] - self.x0) / self.px, (self.y1 - p[1]) / self.px]
    }

    fn blend(&mut self, x: i64, y: i64, c: [u8; 4], rows: usize) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= rows {
            return;
        }
        let i = (y as usize * self.width + x as usize) * 4;
        let a = f32::from(c[3]) / 255.0;
        for (k, ck) in c.iter().take(3).enumerate() {
            let v = f32::from(self.rgba[i + k]) * (1.0 - a) + f32::from(*ck) * a;
            self.rgba[i + k] = v.round() as u8;
        }
    }

    /// A line `radius` pixels wide either side, on the map rows. Translucent colours are laid once
    /// per pixel, so a band does not darken where its segments overlap.
    fn stroke(&mut self, line: &[P2], radius: f64, c: [u8; 4]) {
        let rows = self.map_rows;
        let mut hit = vec![false; self.width * rows];
        for w in line.windows(2) {
            let (a, b) = (self.to_px(w[0]), self.to_px(w[1]));
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let n = (len * 2.0).ceil().max(1.0) as usize;
            for k in 0..=n {
                let t = k as f64 / n as f64;
                let (cx, cy) = (a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]));
                let r = radius.ceil() as i64;
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (x, y) = (cx.floor() as i64 + dx, cy.floor() as i64 + dy);
                        let (ex, ey) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                        if ex * ex + ey * ey > radius * radius + 0.25 {
                            continue;
                        }
                        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= rows {
                            continue;
                        }
                        let j = y as usize * self.width + x as usize;
                        if !hit[j] {
                            hit[j] = true;
                            self.blend(x, y, c, rows);
                        }
                    }
                }
            }
        }
    }

    /// Draw the stretches, then the lines, each cased.
    pub fn draw(&mut self, lines: &Lines) {
        let s = arc(&lines.centre);
        for f in &lines.flags {
            let mut part = vec![point_at(&lines.centre, &s, f.from_m)];
            part.extend(
                lines
                    .centre
                    .iter()
                    .zip(&s)
                    .filter(|(_, c)| **c > f.from_m && **c < f.to_m)
                    .map(|(p, _)| *p),
            );
            part.push(point_at(&lines.centre, &s, f.to_m));
            self.stroke(&part, 7.0, f.colour());
        }
        for (line, c) in [
            (&lines.left, LEFT),
            (&lines.right, RIGHT),
            (&lines.centre, CENTRE),
        ] {
            self.stroke(line, 2.0, CASING);
            self.stroke(line, 1.0, c);
        }
    }

    /// The picture with the legend under the map: a swatch and a name per line and stretch kind,
    /// then `text` (scene ids, notices). The canvas widens if the text needs it.
    pub fn with_legend(&self, text: &[String]) -> Picture {
        let line_h = 10;
        let items: [(&str, [u8; 4]); 5] = [
            ("left bank", LEFT),
            ("right bank", RIGHT),
            ("centreline", CENTRE),
            ("repaired (low confidence)", REPAIRED),
            ("other low confidence", LOW),
        ];
        // At least wide enough for the swatch names; longer text wraps at word breaks.
        let width = self.width.max(8 * 30 + 16);
        let per_line = (width - 16) / 8;
        let text: Vec<String> = text.iter().flat_map(|t| wrap(t, per_line)).collect();
        let height = self.map_rows + (items.len() + text.len()) * line_h + 12;
        let mut rgba = vec![0u8; width * height * 4];
        for px in rgba.chunks_mut(4) {
            px.copy_from_slice(&PAPER);
        }
        for y in 0..self.map_rows {
            let (a, b) = (y * self.width * 4, y * width * 4);
            rgba[b..b + self.width * 4].copy_from_slice(&self.rgba[a..a + self.width * 4]);
        }
        let mut out = Picture {
            width,
            height,
            rgba,
            ..self.clone()
        };
        let mut y = self.map_rows + 6;
        for (t, c) in items {
            for dy in 0..8 {
                for dx in 0..18 {
                    let edge = dy == 0 || dy == 7 || dx == 0 || dx == 17;
                    out.put(
                        7 + dx,
                        y + dy,
                        if edge { INK } else { [c[0], c[1], c[2], 255] },
                    );
                }
            }
            out.text(30, y, t);
            y += line_h;
        }
        for t in &text {
            out.text(8, y, t);
            y += line_h;
        }
        out
    }

    fn put(&mut self, x: usize, y: usize, c: [u8; 4]) {
        if x < self.width && y < self.height {
            let i = (y * self.width + x) * 4;
            self.rgba[i..i + 4].copy_from_slice(&c);
        }
    }

    fn text(&mut self, x: usize, y: usize, s: &str) {
        use font8x8::UnicodeFonts;
        for (k, ch) in s.chars().enumerate() {
            let g = font8x8::BASIC_FONTS
                .get(ch)
                .or_else(|| font8x8::LATIN_FONTS.get(ch))
                .unwrap_or([0; 8]);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..8 {
                    if bits & (1 << col) != 0 {
                        self.put(x + k * 8 + col, y + row, INK);
                    }
                }
            }
        }
    }

    /// The picture as a PNG.
    pub fn png(&self) -> Result<Vec<u8>, ReachError> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width as u32, self.height as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc
                .write_header()
                .map_err(|e| ReachError::Input(format!("PNG: {e}")))?;
            w.write_image_data(&self.rgba)
                .map_err(|e| ReachError::Input(format!("PNG: {e}")))?;
        }
        Ok(out)
    }

    /// The four corners of the map (not the legend) in longitude and latitude: bottom left, bottom
    /// right, top right, top left, as `gx:LatLonQuad` lists them.
    pub fn corners(&self) -> Result<[P2; 4], ReachError> {
        let (x1, y0) = (
            self.x0 + self.map_cols as f64 * self.px,
            self.y1 - self.map_rows as f64 * self.px,
        );
        let c = self
            .utm
            .inverse_all(&[[self.x0, y0], [x1, y0], [x1, self.y1], [self.x0, self.y1]])
            .map_err(ReachError::Input)?;
        Ok([c[0], c[1], c[2], c[3]])
    }
}

/// Lines of at most `n` characters, broken at spaces where it can.
fn wrap(t: &str, n: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in t.split(' ') {
        let mut word = word.to_string();
        while word.chars().count() > n {
            let head: String = word.chars().take(n).collect();
            word = word.chars().skip(n).collect();
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            out.push(head);
        }
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > n {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// CRC-32 (IEEE), for the zip.
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, t) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c = table[((c ^ u32::from(*b)) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// A zip of stored (uncompressed) members: a KMZ needs no more.
pub fn zip_stored(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in members {
        let (crc, size, at) = (crc32(data), data.len() as u32, out.len() as u32);
        let head = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central {
                h.extend_from_slice(&20u16.to_le_bytes()); // made by
            }
            h.extend_from_slice(&20u16.to_le_bytes()); // needed to extract
            h.extend_from_slice(&0u16.to_le_bytes()); // flags
            h.extend_from_slice(&0u16.to_le_bytes()); // stored
            h.extend_from_slice(&0u16.to_le_bytes()); // time
            h.extend_from_slice(&0x21u16.to_le_bytes()); // date 1980-01-01
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&size.to_le_bytes());
            h.extend_from_slice(&size.to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // extra
            if central {
                h.extend_from_slice(&0u16.to_le_bytes()); // comment
                h.extend_from_slice(&0u16.to_le_bytes()); // disk
                h.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
                h.extend_from_slice(&0u32.to_le_bytes()); // external attributes
                h.extend_from_slice(&at.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(head(0x0403_4b50, false));
        out.extend_from_slice(data);
        central.extend(head(0x0201_4b50, true));
    }
    let (start, len) = (out.len() as u32, central.len() as u32);
    out.extend(central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(members.len() as u16).to_le_bytes());
    out.extend_from_slice(&(members.len() as u16).to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// KML colour: aabbggrr.
fn kml_colour(c: [u8; 4]) -> String {
    format!("ff{:02x}{:02x}{:02x}", c[2], c[1], c[0])
}

/// The KMZ of a picture (the map, no legend) and its lines: `doc.kml` and `overlay.png`.
pub fn kmz(
    map: &Picture,
    lines: &Lines,
    name: &str,
    notices: &[String],
) -> Result<Vec<u8>, ReachError> {
    let corners = map.corners()?;
    let ll = |pts: &[P2]| -> Result<String, ReachError> {
        let g = map.utm.inverse_all(pts).map_err(ReachError::Input)?;
        Ok(g.iter()
            .map(|p| format!("{:.7},{:.7},0", p[0], p[1]))
            .collect::<Vec<_>>()
            .join(" "))
    };
    let s = arc(&lines.centre);
    let mut marks = String::new();
    for (title, line, c) in [
        ("Left bank", &lines.left, LEFT),
        ("Right bank", &lines.right, RIGHT),
        ("Centreline", &lines.centre, CENTRE),
    ] {
        if line.len() >= 2 {
            marks.push_str(&format!(
                "<Placemark><name>{title}</name><Style><LineStyle><color>{}</color><width>2</width></LineStyle></Style><LineString><tessellate>1</tessellate><coordinates>{}</coordinates></LineString></Placemark>\n",
                kml_colour(c),
                ll(line)?
            ));
        }
    }
    for f in &lines.flags {
        let mut part = vec![point_at(&lines.centre, &s, f.from_m)];
        part.extend(
            lines
                .centre
                .iter()
                .zip(&s)
                .filter(|(_, c)| **c > f.from_m && **c < f.to_m)
                .map(|(p, _)| *p),
        );
        part.push(point_at(&lines.centre, &s, f.to_m));
        // Translucent: alpha 0x96.
        let colour = format!("96{}", &kml_colour(f.colour())[2..]);
        marks.push_str(&format!(
            "<Placemark><name>{} {:.0} to {:.0} m</name><Style><LineStyle><color>{colour}</color><width>8</width></LineStyle></Style><LineString><tessellate>1</tessellate><coordinates>{}</coordinates></LineString></Placemark>\n",
            xml(&f.kind), f.from_m, f.to_m,
            ll(&part)?
        ));
    }
    let quad = corners
        .iter()
        .map(|p| format!("{:.9},{:.9}", p[0], p[1]))
        .collect::<Vec<_>>()
        .join(" ");
    let notice = xml(&notices.join("; "));
    let doc = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<kml xmlns=\"http://www.opengis.net/kml/2.2\" xmlns:gx=\"http://www.google.com/kml/ext/2.2\">\n<Document><name>{}</name><description>{notice}</description>\n<GroundOverlay><name>Sentinel-2 true colour</name><description>{notice}</description><Icon><href>overlay.png</href></Icon><gx:LatLonQuad><coordinates>{quad}</coordinates></gx:LatLonQuad></GroundOverlay>\n{marks}</Document>\n</kml>\n",
        xml(name)
    );
    let png = map.png()?;
    Ok(zip_stored(&[
        ("doc.kml", doc.as_bytes()),
        ("overlay.png", &png),
    ]))
}

/// The lines of a document, in its UTM zone: the banks, the centreline and the low-confidence
/// stretches. A study in WGS84 is refused (the picture is drawn on a UTM grid).
pub fn lines_from_document(doc: &serde_json::Value) -> Result<(Utm, Lines), ReachError> {
    use owd_format::design::crs::Crs;
    let epsg = doc["project"]["crs"]["epsg"].as_u64().unwrap_or(0) as u32;
    let utm = match Crs::from_epsg(epsg) {
        Ok(Crs::Utm(u)) => u,
        _ => {
            return Err(ReachError::Input(
                "the study is not in a UTM zone: the overlay is drawn on the scene's UTM grid"
                    .into(),
            ));
        }
    };
    let line = |role: &str| -> Vec<P2> {
        doc["inputs"][role]["coordinates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
            .collect()
    };
    let lines = Lines {
        left: line("left_bank"),
        right: line("right_bank"),
        centre: line("centreline"),
        flags: doc["low_confidence_width"]["stretches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                Some(Flag {
                    from_m: s["from_m"].as_f64()?,
                    to_m: s["to_m"].as_f64()?,
                    kind: s["kind"].as_str()?.to_string(),
                })
            })
            .collect(),
    };
    if lines.centre.len() < 2 {
        return Err(ReachError::Input(
            "the document has no centreline to draw".into(),
        ));
    }
    Ok((utm, lines))
}

/// The Sentinel-2 scenes a document's banks were made from, in the order recorded (the scene of each
/// part, then the first scene), without repeats.
pub fn scene_ids(doc: &serde_json::Value) -> Vec<String> {
    let set = &doc["provenance"]["left_bank"]["settings"];
    let mut out: Vec<String> = Vec::new();
    let parts = set["parts"].as_array().into_iter().flatten();
    let scenes = set["scenes"].as_array().into_iter().flatten();
    for id in parts
        .chain(scenes)
        .filter_map(|p| p["scene_id"].as_str().or_else(|| p["id"].as_str()))
        .chain(set["scene_id"].as_str())
    {
        if !out.iter().any(|o| o == id) {
            out.push(id.to_string());
        }
    }
    out
}
