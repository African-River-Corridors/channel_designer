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
        /// A GeoTIFF DEM, referenced by relative path and SHA-256 (never copied into the file).
        #[arg(long, value_name = "FILE")]
        dem: Option<String>,
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
    /// Add a design line option by the rolling circle or the least-cost re-route.
    Align {
        /// The channel design. `-` is stdin.
        doc: String,
        /// The method: rolling-circle or reroute. Running a method again replaces its own option.
        #[arg(long, value_name = "METHOD")]
        method: String,
        /// Rolling circle: the sampling step in metres (default 10).
        #[arg(long, value_name = "METRES")]
        step: Option<f64>,
        /// Rolling circle: the number of passes (default 2).
        #[arg(long)]
        passes: Option<u32>,
        /// Read the minimum radius on this chord, in metres (default: a third of R_min for the
        /// rolling circle, one station each side for the re-route).
        #[arg(long, value_name = "METRES")]
        chord: Option<f64>,
        /// Re-route: a GeoTIFF DEM (WGS84 or the project's UTM zone) for the terrain.
        #[arg(long, value_name = "FILE")]
        dem: Option<String>,
        /// Re-route: the water surface level at the start of the reach, in metres.
        #[arg(long, value_name = "METRES")]
        water_level: Option<f64>,
        /// Re-route: the fall of the water surface per metre of chainage (default 0).
        #[arg(long, value_name = "M_PER_M")]
        water_slope: Option<f64>,
        /// Re-route: the half-widths of the corridors to try, in metres, comma separated.
        #[arg(long, value_name = "LIST")]
        corridors: Option<String>,
        #[command(flatten)]
        out: Out,
    },
    /// Choose weir sites and pool levels, and add a design line option through the pools (the lattice).
    Optimise {
        /// The channel design. `-` is stdin.
        doc: String,
        /// A GeoTIFF DEM to record as the document's DEM and use.
        #[arg(long, value_name = "FILE")]
        dem: Option<String>,
        /// The water level below the first weir, in metres (default: the re-route's start level).
        #[arg(long, value_name = "METRES")]
        base_level: Option<f64>,
        /// Candidate weir chainages along the design line, in metres, comma separated
        /// (default: every 500 m).
        #[arg(long, value_name = "LIST")]
        sites: Option<String>,
        /// The lifts a weir may have, in metres, comma separated (default 0.5,1).
        #[arg(long, value_name = "LIST")]
        lifts: Option<String>,
        /// The weight of dredging in the cost (default 1).
        #[arg(long)]
        wet_weight: Option<f64>,
        /// The weight of dry cut in the cost (default 1).
        #[arg(long)]
        dry_weight: Option<f64>,
        /// The cut model: a or b (default a).
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,
        #[command(flatten)]
        out: Out,
    },
    /// Work out dredge, dry cut and tallest face for every design line option, on the DEM.
    Volumes {
        /// The channel design. `-` is stdin.
        doc: String,
        /// A GeoTIFF DEM to record as the document's DEM and use. Needed the first time, or to
        /// accept a DEM file that has changed.
        #[arg(long, value_name = "FILE")]
        dem: Option<String>,
        /// The cut model: a or b (default a).
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,
        /// The water surface level at the start of the reach, in metres (default: the re-route's).
        #[arg(long, value_name = "METRES")]
        water_level: Option<f64>,
        /// The fall of the water surface per metre of chainage (default: the re-route's, else 0).
        #[arg(long, value_name = "M_PER_M")]
        water_slope: Option<f64>,
        /// The station spacing in metres (default 25).
        #[arg(long, value_name = "METRES")]
        station: Option<f64>,
        /// The half-width of each cross-section in metres (default 200).
        #[arg(long, value_name = "METRES")]
        maxw: Option<f64>,
        #[command(flatten)]
        out: Out,
    },
    /// Export the design line options as GeoJSON, or the quantity table as CSV. Every export says
    /// "Screening — not for construction", and an option that fails a check says so.
    Export {
        /// The channel design. `-` is stdin.
        doc: String,
        /// GeoJSON: the options as lines, in the project CRS (or WGS84 with --wgs84).
        #[arg(long, conflicts_with = "csv")]
        geojson: bool,
        /// CSV: one row per option, with dredge, dry cut, tallest face and failed checks.
        #[arg(long)]
        csv: bool,
        /// With --geojson: write longitude and latitude instead of the project CRS.
        #[arg(long, requires = "geojson")]
        wgs84: bool,
        /// Write the export here instead of stdout.
        #[arg(short = 'o', long = "output", value_name = "FILE")]
        output: Option<String>,
    },
    /// Rerun every stale stage, in order.
    Run {
        /// The channel design. `-` is stdin.
        doc: String,
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
            dem,
            out,
        } => commands::import(
            &doc,
            [
                ("left_bank", left_bank),
                ("right_bank", right_bank),
                ("centreline", centreline),
                ("river_polygon", polygon),
            ],
            dem,
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
        Command::Align {
            doc,
            method,
            step,
            passes,
            chord,
            dem,
            water_level,
            water_slope,
            corridors,
            out,
        } => commands::align(
            &doc,
            &method,
            commands::AlignFlags {
                step,
                passes,
                chord,
                dem,
                water_level,
                water_slope,
                corridors,
            },
            &out.output,
            out.json,
        ),
        Command::Optimise {
            doc,
            dem,
            base_level,
            sites,
            lifts,
            wet_weight,
            dry_weight,
            model,
            out,
        } => commands::optimise(
            &doc,
            commands::OptimiseFlags {
                dem,
                base_level,
                sites,
                lifts,
                wet_weight,
                dry_weight,
                model,
            },
            &out.output,
            out.json,
        ),
        Command::Volumes {
            doc,
            dem,
            model,
            water_level,
            water_slope,
            station,
            maxw,
            out,
        } => commands::volumes(
            &doc,
            commands::VolumesFlags {
                dem,
                model,
                water_level,
                water_slope,
                station,
                maxw,
            },
            &out.output,
            out.json,
        ),
        Command::Export {
            doc,
            geojson,
            csv,
            wgs84,
            output,
        } => commands::export(&doc, geojson, csv, wgs84, output.as_deref()),
        Command::Run { doc, out } => commands::run(&doc, &out.output, out.json),
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
