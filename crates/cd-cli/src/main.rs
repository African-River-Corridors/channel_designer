//! `cdesign`: the channel_designer command-line tool. A thin layer over the design session in
//! `cd-format` (spec 0002): every command reads a channel design, does one thing and writes the
//! richer document back. All file, stdin and clock access is here; the session has none.
//!
//! Exit codes: 0 success; 1 a failed check or a stale-stage refusal; 2 a usage or input error.
//!
//! The hidden `cdesign eval <entry-point>` of spec 0001 reads one parity case input as JSON on
//! stdin and writes the result as JSON on stdout. It exists for parity only and is removed with
//! the Reference Results (ticket 37).

mod commands;

use clap::{Args, Parser, Subcommand};
use std::process::ExitCode;

/// Screening-level design of navigation channels in inland rivers.
#[derive(Parser)]
#[command(name = "cdesign", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Options every command that writes a document takes.
#[derive(Args)]
struct Out {
    /// Write the document here instead of back to the input file. `-` is stdout.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output: Option<String>,
    /// Print a machine-readable result as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Start a channel design: one file for one study.
    New {
        /// The `.cdesign` file to create. `-` is stdout.
        file: String,
        /// The study's title.
        #[arg(long)]
        title: String,
        /// Created date, YYYY-MM-DD (default: today, UTC).
        #[arg(long, value_name = "DATE")]
        date: Option<String>,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Import bank lines, a river centreline or a river polygon from GeoJSON or a GeoPackage (WGS84 or
    /// WGS84 / UTM). Name a GeoPackage layer as FILE#LAYER.
    Import {
        /// The channel design. `-` is stdin.
        doc: String,
        /// The left bank, as a GeoJSON line. `-` is stdin.
        #[arg(long, value_name = "FILE")]
        left_bank: Option<String>,
        /// The right bank, as a GeoJSON line.
        #[arg(long, value_name = "FILE")]
        right_bank: Option<String>,
        /// The river centreline, as a GeoJSON line.
        #[arg(long, value_name = "FILE")]
        centreline: Option<String>,
        /// The river polygon, as a GeoJSON polygon.
        #[arg(long, value_name = "FILE")]
        polygon: Option<String>,
        #[command(flatten)]
        out: Out,
    },
    /// Choose a ruleset, a vessel class and a draught, and size the channel.
    Size {
        /// The channel design. `-` is stdin.
        doc: String,
        /// The ruleset, for example PIANC-B.
        #[arg(long)]
        ruleset: String,
        /// The vessel class, for example CEMT_IV.
        #[arg(long)]
        vessel: String,
        /// The design draught in metres (default: the vessel class's).
        #[arg(long, value_name = "METRES")]
        draught: Option<f64>,
        #[command(flatten)]
        out: Out,
    },
    /// Check a channel design against the schema and its own rules.
    Validate {
        /// The channel design. `-` is stdin.
        doc: String,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show each stage as not run, fresh or stale.
    Status {
        /// The channel design. `-` is stdin.
        doc: String,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the JSON Schema of the `.cdesign` format.
    Schema,
    /// Run a parity case (spec 0001; removed with the Reference Results).
    #[command(hide = true)]
    Eval { entry_point: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::New {
            file,
            title,
            date,
            json,
        } => commands::new(&file, &title, date.as_deref(), json),
        Command::Import {
            doc,
            left_bank,
            right_bank,
            centreline,
            polygon,
            out,
        } => commands::import(
            &doc,
            [
                ("left_bank", left_bank),
                ("right_bank", right_bank),
                ("centreline", centreline),
                ("river_polygon", polygon),
            ],
            &out.output,
            out.json,
        ),
        Command::Size {
            doc,
            ruleset,
            vessel,
            draught,
            out,
        } => commands::size(&doc, &ruleset, &vessel, draught, &out.output, out.json),
        Command::Validate { doc, json } => commands::validate(&doc, json),
        Command::Status { doc, json } => commands::status(&doc, json),
        Command::Schema => commands::schema(),
        Command::Eval { entry_point } => commands::eval(&entry_point),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cdesign: {}", e.message);
            ExitCode::from(e.code)
        }
    }
}
