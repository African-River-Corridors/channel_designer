//! The `cdesign` subcommands. File, stdin, stdout and clock access live here.

use cd_format::design::{DesignError, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};
use std::io::{Read, Write};

/// A failed command: its message and exit code (1 a failed check, 2 a usage or input error).
pub struct Failure {
    pub message: String,
    pub code: u8,
}

pub type Result = core::result::Result<(), Failure>;

fn input_error(message: impl Into<String>) -> Failure {
    Failure {
        message: message.into(),
        code: 2,
    }
}

fn from_design(e: DesignError) -> Failure {
    let code = match e {
        DesignError::StaleUpstream { .. } => 1,
        _ => 2,
    };
    Failure {
        message: e.to_string(),
        code,
    }
}

fn read(path: &str) -> core::result::Result<String, Failure> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| input_error(format!("cannot read stdin: {e}")))?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).map_err(|e| input_error(format!("cannot read {path}: {e}")))
    }
}

fn write(path: &str, text: &str) -> Result {
    if path == "-" {
        std::io::stdout()
            .write_all(text.as_bytes())
            .map_err(|e| input_error(format!("cannot write stdout: {e}")))
    } else {
        std::fs::write(path, text).map_err(|e| input_error(format!("cannot write {path}: {e}")))
    }
}

fn open(path: &str) -> core::result::Result<Session, Failure> {
    Session::open(&read(path)?).map_err(from_design)
}

/// Print a result. When the document itself goes to stdout the result goes to stderr, so the two
/// never mix.
fn say(doc_on_stdout: bool, json_mode: bool, human: &str, value: Value) {
    let text = if json_mode {
        format!("{value}\n")
    } else {
        format!("{human}\n")
    };
    if doc_on_stdout {
        eprint!("{text}");
    } else {
        print!("{text}");
    }
}

/// Where a command that rewrites a document puts it: `-o`, else back where it came from
/// (stdout if it came from stdin).
fn destination(doc: &str, output: &Option<String>) -> String {
    output.clone().unwrap_or_else(|| doc.to_string())
}

/// Today's date in UTC, `YYYY-MM-DD`, from the system clock.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    cd_format::design::date_from_unix(secs)
}

pub fn new(file: &str, title: &str, date: Option<&str>, json_mode: bool) -> Result {
    let date = date.map_or_else(today, str::to_string);
    let ok = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !ok {
        return Err(input_error(format!(
            "`{date}` is not a date: use YYYY-MM-DD"
        )));
    }
    if title.trim().is_empty() {
        return Err(input_error("the title must not be empty"));
    }
    let s = Session::new(title, &date);
    write(file, &s.to_text())?;
    say(
        file == "-",
        json_mode,
        &format!("created {file}: \"{title}\""),
        json!({"ok": true, "file": file, "title": title, "created": date}),
    );
    Ok(())
}

pub fn import(
    doc: &str,
    inputs: [(&str, Option<String>); 4],
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let roles = [
        Role::LeftBank,
        Role::RightBank,
        Role::Centreline,
        Role::RiverPolygon,
    ];
    let mut texts: Vec<(Role, String)> = Vec::new();
    let mut stdin_used = doc == "-";
    for (role, (_, path)) in roles.into_iter().zip(inputs) {
        if let Some(path) = path {
            if path == "-" {
                if stdin_used {
                    return Err(input_error("only one input can be read from stdin"));
                }
                stdin_used = true;
            }
            texts.push((role, read(&path)?));
        }
    }
    if texts.is_empty() {
        return Err(input_error(
            "nothing to import: give --left-bank, --right-bank, --centreline or --polygon",
        ));
    }
    // The document is read first when it is on stdin: stdin can be read once.
    let mut session = open(doc)?;
    let items: Vec<(Role, &str)> = texts.iter().map(|(r, t)| (*r, t.as_str())).collect();
    let report = session.import(&items).map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let human = {
        let mut lines = vec![format!("project CRS: EPSG:{}", report.project_epsg)];
        for (name, src, n) in &report.inputs {
            lines.push(format!("{name}: {n} positions, from {src}"));
        }
        lines.join("\n")
    };
    let inputs_json: Vec<Value> = report
        .inputs
        .iter()
        .map(|(n, c, k)| json!({"input": n, "source_crs": c, "positions": k}))
        .collect();
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "project_epsg": report.project_epsg, "inputs": inputs_json}),
    );
    Ok(())
}

pub fn size(
    doc: &str,
    ruleset: &str,
    vessel: &str,
    draught: Option<f64>,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    // Opened first: when the document comes from stdin it must be read before anything else.
    let mut session = open(doc)?;
    session
        .set_sizing(ruleset, vessel, draught)
        .map_err(from_design)?;
    session.run_stage(Stage::Sizing).map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let shape = session.document()["stages"]["sizing"]["results"].clone();
    let f = |k: &str| shape[k].as_f64().unwrap_or(f64::NAN);
    let mut human = format!(
        "{} x {}: bottom width {:.1} m, R_min {:.0} m, design depth {:.2} m",
        ruleset,
        vessel,
        f("bottom_width_straight_m"),
        f("min_bend_radius_m"),
        f("design_depth_m")
    );
    for w in shape["warnings"].as_array().into_iter().flatten() {
        if let Some(w) = w.as_str() {
            human.push_str(&format!("\nwarning: {w}"));
        }
    }
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "sizing": shape}),
    );
    Ok(())
}

pub fn run(doc: &str, output: &Option<String>, json_mode: bool) -> Result {
    let mut session = open(doc)?;
    let ran = session.run_stale().map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let names: Vec<&str> = ran.iter().map(|s| s.name()).collect();
    let human = if names.is_empty() {
        "nothing to run: every stage is fresh or not set up".to_string()
    } else {
        format!("ran: {}", names.join(", "))
    };
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "ran": names}),
    );
    Ok(())
}

pub fn validate(doc: &str, json_mode: bool) -> Result {
    let text = read(doc)?;
    let errors = match Session::open(&text) {
        Ok(_) => Vec::new(),
        Err(DesignError::Document(e)) => e,
        Err(other) => return Err(from_design(other)),
    };
    if errors.is_empty() {
        say(
            false,
            json_mode,
            "valid",
            json!({"valid": true, "errors": []}),
        );
        Ok(())
    } else {
        if json_mode {
            println!("{}", json!({"valid": false, "errors": errors}));
        } else {
            for e in &errors {
                println!("{e}");
            }
        }
        Err(Failure {
            message: format!("{} problem(s) found", errors.len()),
            code: 1,
        })
    }
}

pub fn status(doc: &str, json_mode: bool) -> Result {
    let session = open(doc)?;
    let st = session.status();
    let mut human: Vec<String> = st
        .stages
        .iter()
        .map(|(s, status)| format!("{}: {}", s.name(), status.name()))
        .collect();
    let stale: Vec<&str> = st
        .stages
        .iter()
        .filter(|(_, s)| *s == StageStatus::Stale)
        .map(|(s, _)| s.name())
        .collect();
    if !stale.is_empty() {
        human.push(format!("stale: {}", stale.join(", ")));
    }
    let mut stages = serde_json::Map::new();
    for (s, status) in &st.stages {
        stages.insert(
            s.name().into(),
            json!(match status {
                StageStatus::NotRun => "not_run",
                StageStatus::Fresh => "fresh",
                StageStatus::Stale => "stale",
            }),
        );
    }
    say(
        false,
        json_mode,
        &human.join("\n"),
        json!({"stages": Value::Object(stages), "stale": stale}),
    );
    Ok(())
}

pub fn schema() -> Result {
    write("-", Session::schema_text())
}

pub fn eval(entry_point: &str) -> Result {
    let text = read("-")?;
    let input: Value =
        serde_json::from_str(&text).map_err(|e| input_error(format!("stdin is not JSON: {e}")))?;
    let output = cd_format::eval(entry_point, &input).map_err(|e| input_error(e.to_string()))?;
    println!("{output}");
    Ok(())
}
