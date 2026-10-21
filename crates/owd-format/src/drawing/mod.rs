//! Drawings (spec 0004, ADR 0009): the design document and its stage results in, drawing documents
//! of the Drawing Format Standard out. `otd` turns a drawing document into normalised content, SVG,
//! PDF and DXF. This module owns what a sheet shows; it owns no layout engine, no text width table,
//! no hatch and no writer.
//!
//! The builder computes no engineering value. Every number it draws is a result the design
//! already holds (the station walk the volumes stage sums). [`check`] reads the finished drawing
//! documents back and fails when a drawn value differs from its result.

pub mod check;
mod furniture;
mod gallery;
pub mod plan;
mod setting_out;
pub mod style;
mod typical;

use crate::design::terrain::dem_sampler;
use crate::design::{DemCheck, DesignError, Session, Terrain};
use otd::raster::WithImages;
use otd::{NoFiles, RunResult};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub use check::Finding;
pub use gallery::GalleryParams;
pub use setting_out::{SettingOutParams, Table as SettingOutTable, table as setting_out_table};
pub use style::{Style, StyleError};
pub use typical::{Facts, TypicalParams, facts as typical_facts};

/// The kinds of sheet. Each has a code in the drawing number: `OWD-<design>-<code>-<nnn>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SheetKind {
    /// The cross-section gallery.
    CrossSections,
    /// The typical section.
    TypicalSection,
    /// The setting-out table.
    SettingOut,
    /// The plan sheets: an overview and strips along chainage.
    Plan,
}

impl SheetKind {
    pub const ALL: &'static [SheetKind] = &[
        SheetKind::CrossSections,
        SheetKind::TypicalSection,
        SheetKind::SettingOut,
        SheetKind::Plan,
    ];

    /// The kind code in the drawing number.
    pub fn code(self) -> &'static str {
        match self {
            SheetKind::CrossSections => "XS",
            SheetKind::TypicalSection => "TS",
            SheetKind::SettingOut => "SO",
            SheetKind::Plan => "PL",
        }
    }

    /// The word the command line uses for the kind.
    pub fn word(self) -> &'static str {
        match self {
            SheetKind::CrossSections => "xs",
            SheetKind::TypicalSection => "ts",
            SheetKind::SettingOut => "so",
            SheetKind::Plan => "pl",
        }
    }

    pub fn from_word(word: &str) -> Option<SheetKind> {
        Self::ALL.iter().copied().find(|k| k.word() == word)
    }
}

/// What `draw` is asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawOptions {
    /// The design line option, by id or name. `None` is the first option.
    pub option: Option<String>,
    /// The kinds of sheet to draw.
    pub sheets: Vec<SheetKind>,
    /// The revision letter in the title block.
    pub revision: String,
    /// The date in the title block, `YYYY-MM-DD`. `None` is the date the design was created, so
    /// the same design always gives the same drawing documents.
    pub date: Option<String>,
    /// The sheet style, with overrides applied; its hash is on every sheet.
    pub style: Style,
}

impl Default for DrawOptions {
    fn default() -> Self {
        DrawOptions {
            option: None,
            sheets: SheetKind::ALL.to_vec(),
            revision: "A".into(),
            date: None,
            style: Style::builtin(),
        }
    }
}

/// One sheet: a drawing document and what names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub kind: SheetKind,
    /// The drawing number, for example `OWD-SYNTHETIC-XS-001`.
    pub number: String,
    pub document: Value,
    /// Files that go with the sheet, by name (the setting-out CSV).
    pub extra: Vec<(String, Vec<u8>)>,
}

impl Sheet {
    /// The file-name stem of the sheet: its drawing number in lower case.
    pub fn stem(&self) -> String {
        self.number.to_ascii_lowercase()
    }

    /// The document as text. One writer, so the command line and Python give the same bytes.
    pub fn document_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(&self.document).unwrap_or_default();
        text.push('\n');
        text
    }
}

/// Why a drawing could not be made.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawError {
    /// The design is not ready, or the input is wrong.
    Input(String),
    /// A value the sheet needs is not in the design. The ticket stops and reports it.
    Missing(String),
    /// A drawn value differs from its result.
    Checks(Vec<Finding>),
    /// `otd` refused a drawing document (the standard's clause numbers).
    Format { sheet: String, clauses: Vec<String> },
    /// `otd`'s layout review found an error on a sheet: what a reader would trip on.
    Layout {
        sheet: String,
        findings: Vec<String>,
    },
}

impl core::fmt::Display for DrawError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DrawError::Input(m) | DrawError::Missing(m) => write!(f, "{m}"),
            DrawError::Checks(found) => {
                for (i, x) in found.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{x}")?;
                }
                Ok(())
            }
            DrawError::Layout { sheet, findings } => {
                for (i, x) in findings.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "sheet {sheet}: layout review: {x}")?;
                }
                Ok(())
            }
            DrawError::Format { sheet, clauses } => write!(
                f,
                "sheet {sheet}: the Drawing Format Standard refuses the drawing document: clause {}",
                clauses.join(", clause ")
            ),
        }
    }
}

impl std::error::Error for DrawError {}

impl From<StyleError> for DrawError {
    fn from(e: StyleError) -> Self {
        DrawError::Input(e.to_string())
    }
}

impl From<DesignError> for DrawError {
    fn from(e: DesignError) -> Self {
        DrawError::Input(e.to_string())
    }
}

/// The drawing documents of a design line option, checked against the design's results.
pub fn sheets(
    session: &Session,
    terrain: Terrain,
    options: &DrawOptions,
) -> Result<Vec<Sheet>, DrawError> {
    let option_id = resolve_option(session, options.option.as_deref())?;
    let walk = session.option_section_walk(&option_id, terrain)?;
    let mut out = Vec::new();
    for kind in &options.sheets {
        match kind {
            SheetKind::CrossSections => {
                let built = gallery::build(session, &option_id, &walk, options)?;
                let findings = check::gallery(&built, &walk);
                if !findings.is_empty() {
                    return Err(DrawError::Checks(findings));
                }
                out.extend(built);
            }
            SheetKind::SettingOut => {
                let built = setting_out::build(session, &option_id, options)?;
                let table =
                    setting_out::table(session, &option_id, options.style.setting_out().spacing_m)?;
                let findings = check::setting_out(&built, &table);
                if !findings.is_empty() {
                    return Err(DrawError::Checks(findings));
                }
                out.extend(built);
            }
            SheetKind::Plan => {
                let built = plan::build(session, &option_id, &walk, terrain, options)?;
                let findings = check::plan(&built, &walk, session, &option_id, options);
                if !findings.is_empty() {
                    return Err(DrawError::Checks(findings));
                }
                out.extend(built);
            }
            SheetKind::TypicalSection => {
                let built = typical::build(session, &option_id, options)?;
                let findings = check::typical(&built, &typical::facts(session)?);
                if !findings.is_empty() {
                    return Err(DrawError::Checks(findings));
                }
                out.extend(built);
            }
        }
    }
    Ok(out)
}

/// The constraint stretches of an option (spec 0007), as the volumes stage stored them in its
/// screening. Empty when there are none.
pub(super) fn stretches(session: &Session, option_id: &str) -> Vec<Value> {
    session.document()["stages"]["volumes"]["results"]["options"]
        .as_array()
        .and_then(|a| a.iter().find(|r| r["id"] == option_id))
        .and_then(|r| r["screening"]["constraint_stretches"].as_array())
        .cloned()
        .unwrap_or_default()
}

/// The worst stations of an option's constraint stretches, by chainage.
pub(super) fn worst_chainages(session: &Session, option_id: &str) -> Vec<f64> {
    stretches(session, option_id)
        .iter()
        .filter_map(|x| x["worst_chainage_m"].as_f64())
        .collect()
}

/// The id of the option asked for: by id or by name, or the first option.
fn resolve_option(session: &Session, asked: Option<&str>) -> Result<String, DrawError> {
    let list = session.document()["options"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let found = match asked {
        None => list.first(),
        Some(a) => list.iter().find(|o| o["id"] == a || o["name"] == a),
    };
    match (found, asked) {
        (Some(o), _) => Ok(o["id"].as_str().unwrap_or_default().to_string()),
        (None, Some(a)) => Err(DrawError::Input(format!(
            "there is no design line option `{a}`"
        ))),
        (None, None) => Err(DrawError::Input(
            "the design has no design line option to draw: run `owd align` first".into(),
        )),
    }
}

/// Every output of one sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetOutput {
    pub number: String,
    pub stem: String,
    pub document: String,
    pub normalised: String,
    pub svg: String,
    pub pdf: Vec<u8>,
    pub dxf: String,
    /// Notes from the DXF writer (what it could not draw).
    pub warnings: Vec<String>,
    /// The sheet's own extra files.
    pub extra: Vec<(String, Vec<u8>)>,
}

/// Make every output of a sheet with `otd`.
pub fn render(sheet: &Sheet) -> Result<SheetOutput, DrawError> {
    let text = sheet.document_text();
    // the images of the sheet (the plan's relief) go to `otd` from memory, by the path the
    // document names
    let images: Vec<otd::raster::Backdrop> = sheet
        .extra
        .iter()
        .filter(|(name, _)| name.ends_with(".jpg"))
        .map(|(name, bytes)| otd::raster::Backdrop {
            path: name.clone(),
            jpeg: bytes.clone(),
            bbox_m: [0.0; 4],
        })
        .collect();
    let files = WithImages::new(&NoFiles, &images);
    let norm = match otd::run_text(&text, &files) {
        RunResult::Ok(n) => n,
        RunResult::Errors(set) => {
            return Err(DrawError::Format {
                sheet: sheet.number.clone(),
                clauses: set.into_iter().collect(),
            });
        }
    };
    // otd's layout review: an error fails the sheet; a warning is printed with the DXF's
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    for f in otd::review::review(&norm, &otd::review::ReviewOptions::default()) {
        let line = format!(
            "{} `{}`{}: {}{}",
            f.severity.word(),
            f.rule,
            f.viewport
                .as_deref()
                .map_or(String::new(), |v| format!(" in viewport {v}")),
            f.message,
            if f.ids.is_empty() {
                String::new()
            } else {
                format!(" (entities {})", f.ids.join(", "))
            }
        );
        match f.severity {
            otd::review::Severity::Error => errors.push(line),
            otd::review::Severity::Warning => warnings.push(line),
        }
    }
    if !errors.is_empty() {
        return Err(DrawError::Layout {
            sheet: sheet.number.clone(),
            findings: errors,
        });
    }
    let parsed = otd::json::parse(&text).map_err(|_| {
        DrawError::Input(format!("sheet {}: the document is not JSON", sheet.number))
    })?;
    let dxf = otd::render::dxf::to_dxf(&parsed, &norm, &files);
    warnings.extend(dxf.warnings);
    let mut normalised = serde_json::to_string_pretty(&norm.to_value()).unwrap_or_default();
    normalised.push('\n');
    Ok(SheetOutput {
        number: sheet.number.clone(),
        stem: sheet.stem(),
        document: text,
        normalised,
        svg: otd::render::svg::to_svg(&norm, &otd::render::svg::SvgOptions::default()),
        pdf: otd::render::pdf::to_pdf(&norm),
        dxf: dxf.text,
        warnings,
        extra: sheet.extra.clone(),
    })
}

/// The drawing documents as one JSON object, keyed by drawing number (`owd draw --document`).
pub fn documents_json(sheets: &[Sheet]) -> Value {
    let map: BTreeMap<&str, &Value> = sheets
        .iter()
        .map(|s| (s.number.as_str(), &s.document))
        .collect();
    json!(map)
}

impl SheetOutput {
    /// The files of the sheet, by name: the drawing document, the normalised content, SVG, PDF
    /// and DXF. The command line and Python both write exactly these.
    pub fn files(&self) -> Vec<(String, Vec<u8>)> {
        let name = |ext: &str| format!("{}.{ext}", self.stem);
        let mut files = vec![
            (name("drawing.json"), self.document.clone().into_bytes()),
            (
                name("normalised.json"),
                self.normalised.clone().into_bytes(),
            ),
            (name("svg"), self.svg.clone().into_bytes()),
            (name("pdf"), self.pdf.clone()),
            (name("dxf"), self.dxf.clone().into_bytes()),
        ];
        files.extend(self.extra.clone());
        files
    }
}

/// Draw a design given as text, with the bytes of the DEM file it names (if any). The DEM must be
/// the file whose hash the design recorded: a drawn section is what the volumes stage summed.
pub fn sheets_from_text(
    design: &str,
    dem: Option<&[u8]>,
    options: &DrawOptions,
) -> Result<Vec<Sheet>, DrawError> {
    let session = Session::open(design)?;
    let name = session.dem_path().unwrap_or("DEM").to_string();
    let grid = match session.dem_check(dem) {
        DemCheck::Matches => crate::geotiff::read_bytes(dem.unwrap_or_default())
            .map_err(|e| DrawError::Input(format!("{name}: {e}")))?,
        DemCheck::Changed => {
            return Err(DrawError::Input(format!(
                "the DEM `{name}` has changed since its hash was recorded: run `owd volumes --dem {name}` first"
            )));
        }
        DemCheck::Missing => {
            return Err(DrawError::Input(format!(
                "the DEM `{name}` is missing: the sections are drawn from the terrain it gives"
            )));
        }
        DemCheck::NoDem => {
            return Err(DrawError::Input(
                "the design has no DEM: the sections are drawn from the terrain it gives".into(),
            ));
        }
    };
    let epsg = session.document()["project"]["crs"]["epsg"]
        .as_u64()
        .map(|c| c as u32);
    let sampler = dem_sampler(&grid, epsg)?;
    sheets(&session, &sampler, options)
}

/// Every file of every sheet of a design: the one call that `owd draw` and `owd.draw` share.
pub fn files_from_text(
    design: &str,
    dem: Option<&[u8]>,
    options: &DrawOptions,
) -> Result<Vec<(String, Vec<u8>)>, DrawError> {
    let mut out = Vec::new();
    for sheet in sheets_from_text(design, dem, options)? {
        out.extend(render(&sheet)?.files());
    }
    Ok(out)
}

impl DrawOptions {
    /// Read the options from JSON: `option`, `sheets` (a list of kind words), `revision`, `date`.
    /// A key that is not here is not accepted, so a mistyped option does not pass quietly.
    pub fn from_json(v: &Value) -> Result<DrawOptions, DrawError> {
        let mut o = DrawOptions::default();
        let Some(map) = v.as_object() else {
            return Err(DrawError::Input("the options are not a JSON object".into()));
        };
        // The style: a file's members over the default, then the single-value overrides.
        let overrides: Vec<(String, Value)> = match map.get("overrides") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            Some(_) => {
                return Err(DrawError::Input(
                    "`overrides` is not an object of dotted path to value".into(),
                ));
            }
        };
        o.style = match map.get("style") {
            None | Some(Value::Null) => Style::builtin(),
            Some(v) => Style::from_overlay(v)?,
        }
        .with_overrides(&overrides)?;
        for (k, v) in map {
            match k.as_str() {
                "option" => o.option = v.as_str().map(str::to_string),
                "revision" => o.revision = v.as_str().unwrap_or("A").to_string(),
                "date" => o.date = v.as_str().map(str::to_string),
                "style" | "overrides" => {}
                "sheets" => {
                    let words: Vec<&str> = v
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    o.sheets = SheetKind::parse_list(&words.join(","))?;
                }
                other => {
                    return Err(DrawError::Input(format!("unknown draw option `{other}`")));
                }
            }
        }
        Ok(o)
    }
}

impl SheetKind {
    /// A comma list of kind words (`xs`), in the order given, without repeats.
    pub fn parse_list(list: &str) -> Result<Vec<SheetKind>, DrawError> {
        let mut out: Vec<SheetKind> = Vec::new();
        for w in list.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            let k = SheetKind::from_word(w).ok_or_else(|| {
                DrawError::Input(format!(
                    "unknown sheet kind `{w}`; the kinds are: {}",
                    SheetKind::ALL
                        .iter()
                        .map(|k| k.word())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
            if !out.contains(&k) {
                out.push(k);
            }
        }
        if out.is_empty() {
            return Err(DrawError::Input("give at least one sheet kind".into()));
        }
        Ok(out)
    }
}

/// The result of one drawing document through `otd`, as the runner of clause 17.3 prints it.
pub fn run_document(text: &str) -> Value {
    otd::run_text(text, &NoFiles).to_value()
}

/// The same, with the files the document names (its images) read through `files`.
pub fn run_document_with(text: &str, files: &dyn otd::Files) -> Value {
    otd::run_text(text, files).to_value()
}
