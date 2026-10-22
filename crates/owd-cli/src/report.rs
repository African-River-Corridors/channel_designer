//! `owd report` (spec 0014): the reports of a design as PDF, HTML and Word, through `owd-report`
//! and the Open Report Designer. Each file is named after its document number in lower case.

use crate::commands::{Failure, Result, doc_dir, input_error, open_with_dem, say, today};
use owd_report::{Control, Format, Kind, Request, extension};
use serde_json::json;
use std::path::Path;

/// The arguments of `owd report`.
pub struct ReportFlags {
    pub doc: String,
    pub kind: String,
    pub format: String,
    pub option: Option<String>,
    pub out: Option<String>,
    pub revision: Option<String>,
    pub date: Option<String>,
    pub status: Option<String>,
    pub prepared: Option<String>,
    pub checked: Option<String>,
    pub approved: Option<String>,
    pub organisation: Option<String>,
    pub json: bool,
}

fn kinds(word: &str) -> core::result::Result<Vec<Kind>, Failure> {
    if word == "all" {
        return Ok(Kind::ALL.to_vec());
    }
    Kind::from_word(word).map(|k| vec![k]).ok_or_else(|| {
        let known: Vec<&str> = Kind::ALL.iter().map(|k| k.word()).collect();
        input_error(format!(
            "`{word}` is not a kind of report (one of {}, or all)",
            known.join(", ")
        ))
    })
}

fn formats(word: &str) -> core::result::Result<Vec<Format>, Failure> {
    if word == "all" {
        return Ok(Format::ALL.to_vec());
    }
    Format::parse(word)
        .map(|f| vec![f])
        .ok_or_else(|| input_error(format!("`{word}` is not a format (pdf, html, docx or all)")))
}

pub fn report(flags: ReportFlags) -> Result {
    let kinds = kinds(&flags.kind)?;
    let formats = formats(&flags.format)?;
    let (session, dem) = open_with_dem(&flags.doc)?;
    let mut control = Control::draft(&flags.date.unwrap_or_else(today));
    if let Some(r) = flags.revision {
        control.revision = r;
    }
    if let Some(s) = flags.status {
        control.status = s;
    }
    control.prepared = flags.prepared;
    control.checked = flags.checked;
    control.approved = flags.approved;
    control.organisation = flags.organisation;
    let out = match &flags.out {
        Some(o) => Path::new(o).to_path_buf(),
        None => doc_dir(&flags.doc),
    };
    std::fs::create_dir_all(&out)
        .map_err(|e| input_error(format!("cannot create {}: {e}", out.display())))?;
    let text = session.to_text();
    let mut written = Vec::new();
    let mut listed = Vec::new();
    for kind in kinds {
        let req = Request {
            kind,
            option: flags.option.clone(),
            control: control.clone(),
        };
        let report = owd_report::build(&text, dem.as_deref(), &req)
            .map_err(|e| input_error(e.to_string()))?;
        let mut files = Vec::new();
        let mut warnings = Vec::new();
        for &format in &formats {
            let r = report
                .render(format)
                .map_err(|e| input_error(e.to_string()))?;
            for w in &r.warnings {
                eprintln!("owd: warning: {}: {w}", report.number);
            }
            warnings.extend(r.warnings);
            let name = format!("{}.{}", report.stem(), extension(format));
            let path = out.join(&name);
            std::fs::write(&path, &r.bytes)
                .map_err(|e| input_error(format!("cannot write {}: {e}", path.display())))?;
            written.push(path.display().to_string());
            files.push(name);
        }
        listed.push(json!({
            "kind": kind.word(), "number": report.number, "files": files, "warnings": warnings,
        }));
    }
    say(
        false,
        flags.json,
        &written
            .iter()
            .map(|w| format!("wrote {w}"))
            .collect::<Vec<_>>()
            .join("\n"),
        json!({"ok": true, "reports": listed}),
    );
    Ok(())
}
