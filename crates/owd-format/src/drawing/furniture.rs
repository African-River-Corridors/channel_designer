//! The parts every sheet shares: the ISO 7200 title block, the revision register, the notes strip,
//! the status, the drawing number and the places they leave for viewports (clauses 6, 14).
//!
//! Only the standard's own sizes appear here (the block is 180 by 56 mm, a register row 5 mm, a
//! note 4.5 mm). The writing of every line and every cell is `otd`'s.

use otd::rules::{drawing_space, text_width};
use serde_json::{Value, json};

/// The title block of the standard's ISO 7200 template, in millimetres (clause 14.3).
const BLOCK_W_MM: f64 = 180.0;
const BLOCK_H_MM: f64 = 56.0;
/// One revision row, and the header row above it (clause 14.7.3).
const REGISTER_ROW_MM: f64 = 5.0;
/// A strip of notes: the heading and the status line take 12 mm, each note 4.5 mm (clause 14.10.1).
const STRIP_BASE_MM: f64 = 12.0;
const NOTE_PITCH_MM: f64 = 4.5;
/// The centring marks reach 5 mm into the drawing space (clause 6.4); viewports keep 1 mm beyond.
const MARK_CLEAR_MM: f64 = 6.0;
/// Width of the title block's cells, less twice the pad (clause 14.6), by cell.
const TITLE_W_MM: f64 = 110.0 - 2.0;
const NUMBER_W_MM: f64 = 70.0 - 2.0;

/// A rectangle on the paper, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where the furniture stands on a sheet, and what it leaves free.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Layout {
    /// The strip of notes, from the left edge of the drawing space.
    pub strip_h: f64,
    /// Where viewports with drawings may stand: the drawing space above the strip.
    pub free: Area,
    /// The space left of the title block, for the key and the scale bar.
    pub key: Area,
}

pub(super) fn layout(size: &str, landscape: bool, notes: usize) -> Option<Layout> {
    let (_, _, s) = drawing_space(size, landscape)?;
    let strip_h = STRIP_BASE_MM + NOTE_PITCH_MM * notes as f64;
    let block_top = s.y + s.h - BLOCK_H_MM;
    let register_h = 2.0 * REGISTER_ROW_MM;
    let free = Area {
        x: s.x + MARK_CLEAR_MM,
        y: s.y + MARK_CLEAR_MM,
        w: s.w - 2.0 * MARK_CLEAR_MM,
        h: block_top - register_h - strip_h - s.y - MARK_CLEAR_MM,
    };
    let key = Area {
        x: s.x,
        y: block_top,
        w: s.w - BLOCK_W_MM,
        h: BLOCK_H_MM,
    };
    (free.h > 0.0 && key.w > 0.0).then_some(Layout { strip_h, free, key })
}

/// A text in the character set of the standard (clause 11.6.2), in capitals. Any other character
/// becomes a hyphen.
pub(super) fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            let c = c.to_ascii_uppercase();
            if (' '..='~').contains(&c) { c } else { '-' }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// A text cut at the end until its width at height `h_mm` is at most `max_w_mm`: the standard never
/// shrinks a text (clause 14.6).
pub(super) fn fit(text: &str, h_mm: f64, max_w_mm: f64) -> String {
    let mut t = text.trim_end().to_string();
    while !t.is_empty() && text_width(&t, h_mm).is_none_or(|w| w > max_w_mm) {
        t.pop();
    }
    t.trim_end().to_string()
}

/// The design part of a drawing number: the design's title in capitals, words joined by hyphens.
/// When the whole number would not fit its cell, each word is cut to three letters, then the code
/// is cut at the end.
pub(super) fn design_code(title: &str, kind: &str) -> String {
    let words: Vec<String> = clean(title)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    // The widest number: three digits of sheet number.
    let fits =
        |c: &str| text_width(&format!("OWD-{c}-{kind}-000"), 5.0).is_some_and(|w| w <= NUMBER_W_MM);
    let full = words.join("-");
    if fits(&full) {
        return full;
    }
    let mut short = words
        .iter()
        .map(|w| w.chars().take(3).collect::<String>())
        .collect::<Vec<_>>()
        .join("-");
    while !short.is_empty() && !fits(&short) {
        short.pop();
    }
    let short = short.trim_end_matches('-').to_string();
    if short.is_empty() {
        "DESIGN".into()
    } else {
        short
    }
}

pub(super) fn drawing_number(code: &str, kind: &str, n: usize) -> String {
    format!("OWD-{code}-{kind}-{n:03}")
}

/// What the title block shows, from the design.
pub(super) struct Titles {
    /// The design's title (the owner cell).
    pub design: String,
    pub option_name: String,
    pub sheet_title: String,
    pub number: String,
    pub revision: String,
    pub date: String,
    pub sheet_number: usize,
    pub sheet_count: usize,
}

/// The layers every document declares (all from the standard's table, clause 8.5.2).
pub(super) fn layers(names: &[&str]) -> Vec<Value> {
    names.iter().map(|n| json!({ "name": n })).collect()
}

/// The sheet's drawing document, less its viewports.
pub(super) fn document(
    size: &str,
    landscape: bool,
    layout: &Layout,
    titles: &Titles,
    notes: &[String],
    layer_names: &[&str],
    viewports: Vec<Value>,
) -> Value {
    json!({
        "standard_version": "1.0",
        "sheet": {
            "size": size,
            "orientation": if landscape { "landscape" } else { "portrait" },
        },
        "layers": layers(layer_names),
        "viewports": viewports,
        "title_block": {
            "template": "iso7200",
            "fields": {
                "owner": fit(&titles.design, 3.5, TITLE_W_MM),
                "title": fit(&clean(&titles.sheet_title), 5.0, TITLE_W_MM),
                "supplementary_title": fit(
                    &format!("DESIGN LINE OPTION: {}", clean(&titles.option_name)), 3.5, TITLE_W_MM),
                "document_number": titles.number,
                "document_type": "SCREENING",
                "sheet_number": titles.sheet_number,
                "sheet_count": titles.sheet_count,
            },
        },
        "revisions": {
            "entries": [{
                "rev": titles.revision,
                "date": titles.date,
                "description": "SCREENING ISSUE",
                "by": "OWD",
            }],
        },
        "status": "concept",
        "strips": [{
            "id": "notes",
            "edge": "bottom",
            "size_mm": layout.strip_h,
            "content": "notes",
        }],
        "notes": notes,
    })
}
