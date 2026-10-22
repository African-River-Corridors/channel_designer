//! The drawing sheets of a design line option for the app's Sheets view (spec 0004, ticket 8).
//!
//! The sheets come from the same builder as `owd draw` (`owd_format::drawing`), with the same
//! options, so an exported PDF has the bytes `owd draw` writes for the same design and style. The view
//! gets each sheet's number, title and SVG; the front end only places and zooms the SVG.

use owd_format::drawing::{self, DrawOptions, SheetOutput};
use serde_json::{Value, json};
use std::sync::Arc;

/// The inputs of the sheets, taken under the lock so the drawing runs off it.
pub struct SheetsJob {
    design: String,
    dem: Option<Arc<Vec<u8>>>,
    option: String,
}

impl SheetsJob {
    pub(crate) fn new(design: String, dem: Option<Arc<Vec<u8>>>, option: &str) -> SheetsJob {
        SheetsJob {
            design,
            dem,
            option: option.to_string(),
        }
    }

    /// Every sheet of the option, made and rendered as `owd draw --option OPTION` makes them.
    fn outputs(&self) -> Result<Vec<(String, SheetOutput)>, String> {
        let options = DrawOptions {
            option: Some(self.option.clone()),
            ..DrawOptions::default()
        };
        let sheets = drawing::sheets_from_text(
            &self.design,
            self.dem.as_deref().map(Vec::as_slice),
            &options,
        )
        .map_err(|e| e.to_string())?;
        sheets
            .iter()
            .map(|s| {
                let title = s.document["title_block"]["fields"]["title"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                drawing::render(s)
                    .map(|out| (title, out))
                    .map_err(|e| e.to_string())
            })
            .collect()
    }

    /// The sheets for the view: `[{number, title, svg, warnings}]`, in the order `owd draw` writes
    /// them.
    pub fn pages(&self) -> Result<Value, String> {
        Ok(Value::Array(
            self.outputs()?
                .into_iter()
                .map(|(title, out)| {
                    json!({
                        "number": out.number,
                        "title": title,
                        "svg": out.svg,
                        "warnings": out.warnings,
                    })
                })
                .collect(),
        ))
    }

    /// Each sheet's PDF, named as `owd draw` names it (`<drawing number in lower case>.pdf`).
    pub fn pdfs(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        Ok(self
            .outputs()?
            .into_iter()
            .map(|(_, out)| (format!("{}.pdf", out.stem), out.pdf))
            .collect())
    }
}
