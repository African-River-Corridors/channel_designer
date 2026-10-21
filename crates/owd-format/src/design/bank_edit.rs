//! Edits to a bank line (spec 0012, ticket #236): splice a redrawn stretch, and move, add and
//! delete a vertex. The geometry is pure planar code in project metres. [`Session::edit_bank`]
//! applies an edit given in longitude and latitude, marks the input `edited`, and rebuilds the river
//! polygon and centreline from the two banks. A refused edit changes nothing.
//!
//! Every edit is refused when the line would cross itself or the other bank, so a bank always
//! stays one simple line.
//!
//! Label: property

use super::crs::Crs;
use super::geojson::{Role, all_positions};
use super::provenance::{Mark, record};
use super::session::{DesignError, Session};
use serde_json::{Value, json};

/// Metres between the stations of a rebuilt centreline.
pub const CENTRELINE_STATION_M: f64 = 20.0;

/// One edit to a bank, with its points in longitude and latitude.
#[derive(Clone, Debug, PartialEq)]
pub enum BankEdit {
    /// Redraw a stretch: it replaces the bank between the vertices nearest its two ends.
    Splice(Vec<[f64; 2]>),
    Move {
        index: usize,
        to: [f64; 2],
    },
    /// Insert a vertex after vertex `after`.
    Add {
        after: usize,
        at: [f64; 2],
    },
    Delete {
        index: usize,
    },
}

impl Session {
    fn project_crs(&self) -> Result<Crs, DesignError> {
        let epsg = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .ok_or_else(|| DesignError::Input("the design has no bank lines to edit yet".into()))?;
        Crs::from_epsg(epsg as u32).map_err(DesignError::Input)
    }

    fn bank(&self, role: Role) -> Vec<Pt> {
        let g = &self.doc["inputs"][role.name()];
        if g["type"] == "LineString" {
            all_positions(g)
        } else {
            Vec::new()
        }
    }

    /// Edit one bank. `date` (`YYYY-MM-DD`) is stamped on a record made here.
    pub fn edit_bank(&mut self, side: Role, edit: BankEdit, date: &str) -> Result<(), DesignError> {
        if !side.is_bank() {
            return Err(DesignError::Input(
                "only a bank line can be edited here".into(),
            ));
        }
        let crs = self.project_crs()?;
        let other_role = if side == Role::LeftBank {
            Role::RightBank
        } else {
            Role::LeftBank
        };
        let (bank, other) = (self.bank(side), self.bank(other_role));
        if bank.len() < 2 {
            return Err(DesignError::Input(format!(
                "there is no {} to edit",
                side.name().replace('_', " ")
            )));
        }
        let to_project = |pts: &[Pt]| -> Result<Vec<Pt>, DesignError> {
            match crs {
                Crs::Wgs84 => Ok(pts.to_vec()),
                Crs::Utm(u) => u.forward_all(pts).map_err(DesignError::Input),
            }
        };
        let one = |p: Pt| -> Result<Pt, DesignError> { Ok(to_project(&[p])?[0]) };
        let line = match edit {
            BankEdit::Splice(pts) => splice(&bank, &other, &to_project(&pts)?),
            BankEdit::Move { index, to } => move_vertex(&bank, &other, index, one(to)?),
            BankEdit::Add { after, at } => add_vertex(&bank, &other, after, one(at)?),
            BankEdit::Delete { index } => delete_vertex(&bank, &other, index),
        }
        .map_err(DesignError::Input)?;
        self.doc["inputs"][side.name()] = json!({"type": "LineString", "coordinates": line});
        let base = self.record_base(side, date);
        self.set_provenance(side.name(), edited(&base))?;
        let (left, right) = (self.bank(Role::LeftBank), self.bank(Role::RightBank));
        if left.len() >= 2 && right.len() >= 2 {
            self.doc["inputs"]["river_polygon"] =
                json!({"type": "Polygon", "coordinates": [polygon(&left, &right)]});
            self.doc["inputs"]["centreline"] = json!({"type": "LineString", "coordinates": centreline(&left, &right, CENTRELINE_STATION_M)});
            for derived in ["river_polygon", "centreline"] {
                self.set_provenance(derived, edited(&base))?;
            }
        }
        Ok(())
    }

    /// The record of `input` if there is one, else one for the user's own file.
    fn record_base(&self, input: Role, date: &str) -> Value {
        self.provenance(input.name())
            .cloned()
            .unwrap_or_else(|| record("user-file", date, "User-supplied data", Mark::User, false))
    }

    /// Mark inputs just loaded from the user's files as `user`.
    pub fn mark_user(&mut self, roles: &[Role], date: &str) -> Result<(), DesignError> {
        for role in roles {
            self.set_provenance(
                role.name(),
                record("user-file", date, "User-supplied data", Mark::User, false),
            )?;
        }
        Ok(())
    }
}

fn edited(base: &Value) -> Value {
    let mut r = base.clone();
    r["mark"] = json!(Mark::Edited.name());
    r
}

type Pt = [f64; 2];

fn cross(o: Pt, a: Pt, b: Pt) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn on_segment(a: Pt, b: Pt, p: Pt) -> bool {
    p[0] >= a[0].min(b[0])
        && p[0] <= a[0].max(b[0])
        && p[1] >= a[1].min(b[1])
        && p[1] <= a[1].max(b[1])
}

/// True when the closed segments `a-b` and `c-d` share a point (touching counts).
fn segments_meet(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let (d1, d2, d3, d4) = (
        cross(c, d, a),
        cross(c, d, b),
        cross(a, b, c),
        cross(a, b, d),
    );
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on_segment(c, d, a))
        || (d2 == 0.0 && on_segment(c, d, b))
        || (d3 == 0.0 && on_segment(a, b, c))
        || (d4 == 0.0 && on_segment(a, b, d))
}

fn bbox_apart(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    a[0].max(b[0]) < c[0].min(d[0])
        || c[0].max(d[0]) < a[0].min(b[0])
        || a[1].max(b[1]) < c[1].min(d[1])
        || c[1].max(d[1]) < a[1].min(b[1])
}

/// Check the segments `first..=last` of `line` (segment k joins vertex k and k+1) against the rest
/// of `line` and against all of `other`.
fn check(line: &[Pt], first: usize, last: usize, other: &[Pt]) -> Result<(), String> {
    let nseg = line.len().saturating_sub(1);
    let last = last.min(nseg.saturating_sub(1));
    for k in first..=last.max(first) {
        if k >= nseg {
            break;
        }
        let (a, b) = (line[k], line[k + 1]);
        for m in 0..nseg {
            // A neighbour shares a vertex with this segment: that is not a crossing.
            if m + 1 >= k && m <= k + 1 {
                continue;
            }
            let (c, d) = (line[m], line[m + 1]);
            if !bbox_apart(a, b, c, d) && segments_meet(a, b, c, d) {
                return Err("this edit would make the bank cross itself, so it was refused".into());
            }
        }
        for m in 0..other.len().saturating_sub(1) {
            let (c, d) = (other[m], other[m + 1]);
            if !bbox_apart(a, b, c, d) && segments_meet(a, b, c, d) {
                return Err("this edit would cross the other bank, so it was refused".into());
            }
        }
    }
    Ok(())
}

fn nearest_vertex(line: &[Pt], p: Pt) -> usize {
    let d = |q: &Pt| {
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        dx * dx + dy * dy
    };
    (0..line.len())
        .min_by(|&i, &j| d(&line[i]).total_cmp(&d(&line[j])))
        .unwrap_or(0)
}

fn finite(points: &[Pt]) -> Result<(), String> {
    if points
        .iter()
        .any(|p| !p[0].is_finite() || !p[1].is_finite())
    {
        Err("a point is not a number".into())
    } else {
        Ok(())
    }
}

/// Replace the stretch of `bank` between the vertices nearest the first and the last point of
/// `redraw` with `redraw` (turned round if it runs against the bank).
pub fn splice(bank: &[Pt], other: &[Pt], redraw: &[Pt]) -> Result<Vec<Pt>, String> {
    finite(redraw)?;
    if redraw.len() < 2 {
        return Err("draw at least two points to redraw a stretch".into());
    }
    let (mut i, mut j) = (
        nearest_vertex(bank, redraw[0]),
        nearest_vertex(bank, redraw[redraw.len() - 1]),
    );
    let mut draw = redraw.to_vec();
    if i > j {
        std::mem::swap(&mut i, &mut j);
        draw.reverse();
    }
    if i == j {
        return Err(
            "the redrawn stretch must start and end at different places on the bank".into(),
        );
    }
    let mut out = bank[..i].to_vec();
    out.extend_from_slice(&draw);
    out.extend_from_slice(&bank[j + 1..]);
    let first = i.saturating_sub(1);
    check(&out, first, i + draw.len(), other)?;
    Ok(out)
}

/// Move vertex `index` to `to`.
pub fn move_vertex(bank: &[Pt], other: &[Pt], index: usize, to: Pt) -> Result<Vec<Pt>, String> {
    finite(&[to])?;
    if index >= bank.len() {
        return Err(format!("the bank has no vertex {index}"));
    }
    let mut out = bank.to_vec();
    out[index] = to;
    check(&out, index.saturating_sub(1), index, other)?;
    Ok(out)
}

/// Insert a vertex at `at` between vertex `after` and the next one.
pub fn add_vertex(bank: &[Pt], other: &[Pt], after: usize, at: Pt) -> Result<Vec<Pt>, String> {
    finite(&[at])?;
    if after + 1 >= bank.len() {
        return Err(format!("the bank has no segment after vertex {after}"));
    }
    let mut out = bank.to_vec();
    out.insert(after + 1, at);
    check(&out, after, after + 1, other)?;
    Ok(out)
}

/// Delete vertex `index`. A bank keeps at least two vertices.
pub fn delete_vertex(bank: &[Pt], other: &[Pt], index: usize) -> Result<Vec<Pt>, String> {
    if index >= bank.len() {
        return Err(format!("the bank has no vertex {index}"));
    }
    if bank.len() <= 2 {
        return Err("a bank needs at least two vertices".into());
    }
    let mut out = bank.to_vec();
    out.remove(index);
    check(&out, index.saturating_sub(1), index, other)?;
    Ok(out)
}

/// The river polygon from two banks: the left bank, then the right bank backwards, closed.
pub fn polygon(left: &[Pt], right: &[Pt]) -> Vec<Pt> {
    let mut ring = left.to_vec();
    ring.extend(right.iter().rev());
    if let Some(first) = ring.first().copied() {
        ring.push(first);
    }
    ring
}

fn along(line: &[Pt], frac: f64) -> Pt {
    let seg: Vec<f64> = line
        .windows(2)
        .map(|w| libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]))
        .collect();
    let total: f64 = seg.iter().sum();
    let mut want = frac * total;
    for (k, len) in seg.iter().enumerate() {
        if want <= *len || k + 1 == seg.len() {
            let t = if *len > 0.0 {
                (want / len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            return [
                line[k][0] + t * (line[k + 1][0] - line[k][0]),
                line[k][1] + t * (line[k + 1][1] - line[k][1]),
            ];
        }
        want -= len;
    }
    line[0]
}

/// A centreline between two banks: the midpoints of points at equal fractions of each bank's
/// length, one per station metre of the longer bank (at least 2 points).
pub fn centreline(left: &[Pt], right: &[Pt], station_m: f64) -> Vec<Pt> {
    let length = |l: &[Pt]| {
        l.windows(2)
            .map(|w| libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]))
            .sum::<f64>()
    };
    let n = ((length(left).max(length(right)) / station_m.max(1.0)).ceil() as usize).max(1);
    (0..=n)
        .map(|k| {
            let f = k as f64 / n as f64;
            let (a, b) = (along(left, f), along(right, f));
            [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn left() -> Vec<Pt> {
        (0..=10).map(|i| [i as f64 * 10.0, 50.0]).collect()
    }
    fn right() -> Vec<Pt> {
        (0..=10).map(|i| [i as f64 * 10.0, 0.0]).collect()
    }

    #[test]
    fn a_redrawn_stretch_replaces_the_bank_between_the_nearest_vertices() {
        let out = splice(
            &left(),
            &right(),
            &[[31.0, 52.0], [40.0, 58.0], [52.0, 51.0]],
        )
        .unwrap();
        // Vertices 3 and 5 are the nearest ones; 4 is gone, the three drawn points are in.
        assert_eq!(out.len(), 11 - 3 + 3);
        assert_eq!(out[3], [31.0, 52.0]);
        assert_eq!(out[5], [52.0, 51.0]);
        assert_eq!(out[6], [60.0, 50.0]);
    }

    #[test]
    fn a_redraw_that_runs_against_the_bank_is_turned_round() {
        let out = splice(
            &left(),
            &right(),
            &[[52.0, 51.0], [40.0, 58.0], [31.0, 52.0]],
        )
        .unwrap();
        assert_eq!(out[3], [31.0, 52.0]);
        assert_eq!(out[5], [52.0, 51.0]);
    }

    #[test]
    fn a_splice_that_crosses_the_other_bank_is_refused_with_a_message() {
        let e = splice(
            &left(),
            &right(),
            &[[30.0, 50.0], [40.0, -20.0], [60.0, 50.0]],
        )
        .unwrap_err();
        assert!(e.contains("other bank"), "{e}");
    }

    #[test]
    fn a_splice_that_crosses_its_own_bank_is_refused() {
        // A figure of eight: the first and third drawn legs cross.
        let e = splice(
            &left(),
            &right(),
            &[[30.0, 50.0], [60.0, 80.0], [40.0, 80.0], [70.0, 50.0]],
        )
        .unwrap_err();
        assert!(e.contains("itself"), "{e}");
    }

    #[test]
    fn a_splice_must_span_two_places() {
        assert!(splice(&left(), &right(), &[[31.0, 52.0], [32.0, 53.0]]).is_err());
        assert!(splice(&left(), &right(), &[[31.0, 52.0]]).is_err());
    }

    #[test]
    fn move_add_and_delete_keep_the_bank_one_line() {
        let moved = move_vertex(&left(), &right(), 5, [50.0, 30.0]).unwrap();
        assert_eq!(moved[5], [50.0, 30.0]);
        let added = add_vertex(&left(), &right(), 2, [25.0, 55.0]).unwrap();
        assert_eq!(added.len(), 12);
        assert_eq!(added[3], [25.0, 55.0]);
        let deleted = delete_vertex(&left(), &right(), 4).unwrap();
        assert_eq!(deleted.len(), 10);
    }

    #[test]
    fn a_move_across_the_other_bank_is_refused() {
        let e = move_vertex(&left(), &right(), 5, [50.0, -10.0]).unwrap_err();
        assert!(e.contains("other bank"), "{e}");
    }

    #[test]
    fn a_move_that_ties_the_bank_in_a_knot_is_refused() {
        // Landing on another vertex's place is a touch: refused as a self-cross.
        let e = move_vertex(&left(), &right(), 5, [10.0, 50.0]).unwrap_err();
        assert!(e.contains("itself"), "{e}");
    }

    #[test]
    fn a_bank_keeps_two_vertices_and_bad_indices_are_refused() {
        let two = vec![[0.0, 50.0], [10.0, 50.0]];
        assert!(delete_vertex(&two, &right(), 0).is_err());
        assert!(delete_vertex(&left(), &right(), 99).is_err());
        assert!(move_vertex(&left(), &right(), 99, [0.0, 0.0]).is_err());
        assert!(add_vertex(&left(), &right(), 10, [0.0, 0.0]).is_err());
        assert!(move_vertex(&left(), &right(), 1, [f64::NAN, 0.0]).is_err());
    }

    #[test]
    fn the_polygon_and_centreline_follow_the_banks() {
        let p = polygon(&left(), &right());
        assert_eq!(p.first(), p.last());
        assert_eq!(p.len(), 11 + 11 + 1);
        let c = centreline(&left(), &right(), 25.0);
        assert!(c.iter().all(|q| (q[1] - 25.0).abs() < 1e-9));
        assert_eq!(c.first().unwrap()[0], 0.0);
        assert_eq!(c.last().unwrap()[0], 100.0);
    }
}
