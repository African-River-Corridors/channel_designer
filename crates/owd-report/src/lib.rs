//! Reports of a channel design (spec 0014, ADR 0012): the screening report, the quantity schedule,
//! the section pack and the run report, each as PDF, HTML and Word through the Open Report
//! Designer (ORD).
//!
//! Pure: no file, clock or network. The caller gives the design text, the DEM bytes when it has
//! them, and a [`Request`]; [`build`] writes an ORD document and [`Report::render`] renders it. A
//! report computes nothing: every number is read from the document's stage results.

mod fmt;
mod limits;
mod screening;

use owd_format::design::Session;
use owd_format::design::screening::SCREENING_NOTICE;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub use ord::Format;

/// The version of the ORD engine OWD is built with (pinned by tag in the workspace).
pub const ORD_VERSION: &str = ord::VERSION;

/// The kinds of report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The screening report, for decision-makers (spec 0014, report 1).
    Screening,
}

impl Kind {
    pub const ALL: &'static [Kind] = &[Kind::Screening];

    /// The word `--kind` takes.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Screening => "screening",
        }
    }

    pub fn from_word(word: &str) -> Option<Kind> {
        Kind::ALL.iter().copied().find(|k| k.word() == word)
    }

    /// The ORD document class (OWD's register).
    pub fn class(self) -> &'static str {
        match self {
            Kind::Screening => "screening-report",
        }
    }

    /// The code in the document number: `OWD-<design>-<code>`.
    pub fn code(self) -> &'static str {
        match self {
            Kind::Screening => "SR",
        }
    }
}

/// Document control (spec 0014, "What every report has"). ORD checks it: a status needs its
/// sign-offs, and the revision's form fits the status.
#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub revision: String,
    /// ISO 8601. The core reads no clock: the caller gives the date.
    pub date: String,
    pub status: String,
    pub prepared: Option<String>,
    pub checked: Option<String>,
    pub approved: Option<String>,
    pub organisation: Option<String>,
}

impl Control {
    /// Revision `A`, status `draft`, on `date`, with no sign-offs.
    pub fn draft(date: &str) -> Control {
        Control {
            revision: "A".into(),
            date: date.into(),
            status: "draft".into(),
            prepared: None,
            checked: None,
            approved: None,
            organisation: None,
        }
    }
}

/// What to report on.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub kind: Kind,
    /// The design line option, by id or name (default: the first, as `owd draw`).
    pub option: Option<String>,
    pub control: Control,
}

/// A report: its ORD document and the bytes of its figures, ready to render.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub kind: Kind,
    /// The document number, for example `OWD-SYNTHETIC-RIVER-SR`.
    pub number: String,
    /// The ORD document (JSON).
    pub document: Value,
    /// Asset id to bytes.
    pub assets: BTreeMap<String, Vec<u8>>,
}

/// One rendered output.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub bytes: Vec<u8>,
    /// ORD's warnings, as text.
    pub warnings: Vec<String>,
}

/// Why a report could not be made.
#[derive(Debug, Clone, PartialEq)]
pub enum ReportError {
    /// The design could not be read.
    Design(String),
    /// ORD refused the document: its findings, each with its code (such as `ORD004`).
    Ord(Vec<String>),
}

impl core::fmt::Display for ReportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReportError::Design(m) => write!(f, "{m}"),
            ReportError::Ord(found) => write!(f, "{}", found.join("\n")),
        }
    }
}

impl std::error::Error for ReportError {}

fn ord_errors(found: Vec<ord::Finding>) -> ReportError {
    ReportError::Ord(found.iter().map(ToString::to_string).collect())
}

/// OWD's ORD register: its four document classes on ORD's `standard` lifecycle, numbered `OWD-`.
pub fn register() -> ord::Register {
    let builtin = ord::builtin_register();
    let classes = [
        ("screening-report", "Screening report", true),
        ("quantity-schedule", "Quantity schedule", false),
        ("section-pack", "Section pack", true),
        ("run-report", "Run report", false),
    ];
    let reg = json!({
        "classes": classes.iter().map(|(id, name, contents)| json!({
            "id": id, "name": name, "number_prefix": "OWD", "lifecycle": "standard",
            "contents": contents,
        })).collect::<Vec<_>>(),
        "lifecycles": serde_json::to_value(&builtin.lifecycles).expect("a register serialises"),
    });
    ord::Register::from_json(&reg.to_string()).expect("OWD's register is valid")
}

impl Report {
    /// The file stem: the document number in lower case.
    pub fn stem(&self) -> String {
        self.number.to_lowercase()
    }

    fn parsed(&self) -> Result<(ord::Document, ord::Assets), ReportError> {
        let doc = ord::parse(&self.document.to_string()).map_err(ord_errors)?;
        let (assets, found) = ord::assets::collect(&doc, &self.assets);
        if !found.is_empty() {
            return Err(ord_errors(found));
        }
        Ok((doc, assets))
    }

    /// ORD's findings on the document (errors and warnings), as text.
    pub fn check(&self) -> Result<Vec<String>, ReportError> {
        let (doc, assets) = self.parsed()?;
        Ok(ord::check(&doc, &assets, &register())
            .iter()
            .map(ToString::to_string)
            .collect())
    }

    /// The report in one format. ORD's errors refuse it.
    pub fn render(&self, format: Format) -> Result<Rendered, ReportError> {
        let (doc, assets) = self.parsed()?;
        let out = ord::render(&doc, &assets, &register(), format).map_err(ord_errors)?;
        Ok(Rendered {
            bytes: out.bytes,
            warnings: out.warnings.iter().map(ToString::to_string).collect(),
        })
    }
}

/// The file extension of a format.
pub fn extension(format: Format) -> &'static str {
    match format {
        Format::Pdf => "pdf",
        Format::Html => "html",
        Format::Docx => "docx",
    }
}

/// Build the report a request asks for, from the design text and the DEM bytes.
pub fn build(design: &str, dem: Option<&[u8]>, req: &Request) -> Result<Report, ReportError> {
    let session = Session::open(design).map_err(|e| ReportError::Design(e.to_string()))?;
    let ctx = Context::new(&session, dem, req.option.as_deref())?;
    let number = format!("OWD-{}-{}", design_code(&ctx.title), req.kind.code());
    let (body, appendices) = match req.kind {
        Kind::Screening => screening::body(&ctx),
    };
    let c = &req.control;
    let mut control = json!({
        "number": number,
        "revision": c.revision,
        "date": c.date,
        "status": c.status,
        "history": [{
            "revision": c.revision, "date": c.date, "status": c.status,
            "description": "Made by Open Waterway Designer.",
        }],
    });
    for (k, v) in [
        ("prepared", &c.prepared),
        ("checked", &c.checked),
        ("approved", &c.approved),
    ] {
        if let Some(v) = v {
            control[k] = json!(v);
        }
    }
    let mut document = json!({
        "ord": "0.1",
        "class": req.kind.class(),
        "title": ctx.title,
        "subtitle": subtitle(&ctx),
        "control": control,
        "summary": summary(&ctx),
        "notice": SCREENING_NOTICE,
        "body": body,
    });
    if let Some(org) = &c.organisation {
        document["organisation"] = json!(org);
    }
    if !appendices.is_empty() {
        document["appendices"] = json!(appendices);
    }
    Ok(Report {
        kind: req.kind,
        number,
        document,
        assets: BTreeMap::new(),
    })
}

/// What every part of a report reads.
pub(crate) struct Context<'a> {
    pub session: &'a Session,
    pub doc: &'a Value,
    pub dem: Option<&'a [u8]>,
    pub title: String,
    /// The chosen design line option (from `options`), if the design has one.
    pub option: Option<Value>,
}

impl<'a> Context<'a> {
    fn new(
        session: &'a Session,
        dem: Option<&'a [u8]>,
        asked: Option<&str>,
    ) -> Result<Context<'a>, ReportError> {
        let doc = session.document();
        let list = doc["options"].as_array().cloned().unwrap_or_default();
        let option = match asked {
            None => list.first().cloned(),
            Some(a) => Some(
                list.iter()
                    .find(|o| o["id"] == a || o["name"] == a)
                    .cloned()
                    .ok_or_else(|| {
                        ReportError::Design(format!("there is no design line option `{a}`"))
                    })?,
            ),
        };
        let title = doc["project"]["title"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or("Channel design")
            .to_string();
        Ok(Context {
            session,
            doc,
            dem,
            title,
            option,
        })
    }

    /// The volumes stage's row of the chosen option.
    pub fn volumes_row(&self) -> Option<&'a Value> {
        let id = self.option.as_ref()?["id"].clone();
        self.doc["stages"]["volumes"]["results"]["options"]
            .as_array()?
            .iter()
            .find(|r| r["id"] == id)
    }
}

/// The design part of the number: the title in capitals, words joined by hyphens (as the drawing
/// numbers, spec 0004, before they are cut to fit a title block).
fn design_code(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_uppercase)
        .collect();
    if words.is_empty() {
        "DESIGN".into()
    } else {
        words.join("-")
    }
}

fn subtitle(ctx: &Context) -> String {
    match &ctx.doc["reach"]["river"] {
        Value::String(r) if !r.trim().is_empty() => format!("The {r}"),
        _ => "Channel design screening".into(),
    }
}

/// The cover summary: what the report is, and the rules notice while every Rule is unverified.
fn summary(ctx: &Context) -> String {
    let mut s = String::from(
        "A screening of the river for navigation, made by Open Waterway Designer from the design \
         file. Every number is read from the design's stage results.",
    );
    if let Some(n) = ctx.doc["stages"]["sizing"]["results"]["rules_notice"].as_str() {
        s.push(' ');
        s.push_str(&fmt::md(n));
    }
    s
}
