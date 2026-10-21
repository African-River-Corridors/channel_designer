//! `owd`: the owd command-line tool. A thin layer over the design session in
//! `owd-format` (spec 0002): every command reads a channel design, does one thing and writes the
//! richer document back. All file, stdin and clock access is here; the session has none.
//!
//! Exit codes: 0 success; 1 a failed check or a stale-stage refusal; 2 a usage or input error.

use crate::commands;

use clap::{Args, Parser, Subcommand};
use std::process::ExitCode;

/// Screening-level design of navigation channels in inland rivers.
#[derive(Parser)]
#[command(name = "owd", bin_name = "owd", version, about, long_about = None)]
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
enum DrawSub {
    /// Run one drawing document through the drawing crate and print the result (clause 17.3).
    Run {
        /// The drawing document.
        document: String,
    },
}

#[derive(Subcommand)]
enum Command {
    /// Start a channel design: one file for one study.
    New {
        /// The `.owd` file to create. `-` is stdout.
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
        /// The water level in metres: a constant, or with --water-slope the level at the river's start.
        #[arg(long, value_name = "METRES")]
        water_level: Option<f64>,
        /// The fall of the water per metre of river chainage (with --water-level).
        #[arg(long, value_name = "M_PER_M")]
        water_slope: Option<f64>,
        /// The water level as a table: lines of `chainage_m,level_m` along the river centreline.
        /// Two lines at one chainage make a step (a weir).
        #[arg(long, value_name = "FILE")]
        water_table: Option<String>,
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
        /// Read the minimum radius on this chord, in metres (default: a third of R_min, for every
        /// method).
        #[arg(long, value_name = "METRES")]
        chord: Option<f64>,
        /// Re-route: a GeoTIFF DEM (WGS84 or the project's UTM zone) for the terrain.
        #[arg(long, value_name = "FILE")]
        dem: Option<String>,
        /// Re-route: set the water level (the level at the river's start), in metres.
        #[arg(long, value_name = "METRES")]
        water_level: Option<f64>,
        /// Re-route: the fall of the water per metre of river chainage (with --water-level).
        #[arg(long, value_name = "M_PER_M")]
        water_slope: Option<f64>,
        /// Re-route: the half-widths of the corridors to try, in metres, comma separated.
        #[arg(long, value_name = "LIST")]
        corridors: Option<String>,
        /// Re-route: the fairness weight, on the squared change of heading between stations
        /// (default 0: off). Prefers the smoother of two lines that cut the same.
        #[arg(long, value_name = "WEIGHT")]
        fairness_weight: Option<f64>,
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
        /// The water level below the first weir, in metres (default: the water level at the line's start).
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
        /// Set the water level (the level at the river's start), in metres.
        #[arg(long, value_name = "METRES")]
        water_level: Option<f64>,
        /// The fall of the water per metre of river chainage (with --water-level).
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
    /// Download the Copernicus GLO-30 DEM tiles that cover the river, write them next to the
    /// document with their licence notice, and record them as the DEM. The only command that uses
    /// the network.
    FetchDem {
        /// The channel design. `-` is not allowed: files are written next to it.
        doc: String,
        /// Metres to add around the river on every side (default 1000).
        #[arg(long, value_name = "METRES", default_value_t = 1000.0)]
        margin: f64,
        /// Where the tiles come from: an http(s) URL laid out as `<url>/<tile>/<tile>.tif`, or a
        /// folder with the same layout (default: the public Copernicus open-data bucket).
        #[arg(long, value_name = "URL_OR_FOLDER")]
        mirror: Option<String>,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Start a study from a river reach (spec 0012): search a river by name and give a start and an
    /// end (and via points), or give a rough trace. The route is found on the OSM waterway graph and
    /// kept as way ids. Writes a new document. Uses the network (Overpass).
    Reach {
        /// The `.owd` file to create. `-` is stdout.
        file: String,
        /// The river's name, searched in OSM. The route may leave this river.
        #[arg(long)]
        river: Option<String>,
        /// The start of the reach, as `lon,lat`.
        #[arg(
            long,
            value_name = "LON,LAT",
            requires = "end",
            conflicts_with = "trace",
            allow_hyphen_values = true
        )]
        start: Option<String>,
        /// The end of the reach, as `lon,lat`.
        #[arg(
            long,
            value_name = "LON,LAT",
            requires = "start",
            allow_hyphen_values = true
        )]
        end: Option<String>,
        /// A point the route must pass, as `lon,lat`: put one on the branch you want at a split.
        /// Repeat it for more.
        #[arg(
            long,
            value_name = "LON,LAT",
            requires = "start",
            allow_hyphen_values = true
        )]
        via: Vec<String>,
        /// A rough trace of the river, as a GeoJSON line (WGS84). It snaps to the waterways it
        /// follows; a stretch with none near it stays as drawn.
        #[arg(long, value_name = "FILE")]
        trace: Option<String>,
        /// The study's title (default: the river's name).
        #[arg(long)]
        title: Option<String>,
        /// Another Overpass server (default: overpass-api.de).
        #[arg(long, value_name = "URL")]
        overpass: Option<String>,
        /// Created date, YYYY-MM-DD (default: today, UTC).
        #[arg(long, value_name = "DATE")]
        date: Option<String>,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
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
    /// Draw sheets of a design line option: cross sections and the typical section now, more kinds as they land. Writes, for
    /// each sheet, its drawing document, normalised content, SVG, PDF and DXF. `owd draw run FILE`
    /// reads one drawing document with the drawing crate and prints the result of clause 17.3.
    #[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
    Draw {
        #[command(subcommand)]
        sub: Option<DrawSub>,
        /// The channel design (after the volumes stage). `-` is stdin.
        doc: Option<String>,
        /// Write the drawings into this folder.
        #[arg(long, value_name = "DIR", required_unless_present = "document")]
        out: Option<String>,
        /// Print the drawing documents only, as one JSON object keyed by drawing number.
        #[arg(long, conflicts_with = "out")]
        document: bool,
        /// The design line option, by id or name (default: the first).
        #[arg(long, value_name = "NAME")]
        option: Option<String>,
        /// The kinds of sheet, comma separated: xs (cross sections), ts (typical section). Default: all.
        #[arg(long, value_name = "KINDS")]
        sheets: Option<String>,
        /// The revision letter in the title block (default A).
        #[arg(long, value_name = "REV")]
        revision: Option<String>,
        /// The date in the title block, YYYY-MM-DD (default: the date the design was created).
        #[arg(long, value_name = "DATE")]
        date: Option<String>,
        /// A sheet style file (JSON): the members it holds are laid over the built-in default. Its hash
        /// is printed on every sheet.
        #[arg(long, value_name = "FILE")]
        style: Option<String>,
        /// Override the style: a section every this many metres of chainage.
        #[arg(long, value_name = "METRES")]
        section_spacing: Option<f64>,
        /// Override the style: the sheet size of the cross sections (A0 to A4).
        #[arg(long, value_name = "SIZE")]
        sheet_size: Option<String>,
        /// Override the style: draw the cross sections at this one scale (the denominator, for example 1000).
        #[arg(long, value_name = "N")]
        scale: Option<u32>,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
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
    /// Print the JSON Schema of the `.owd` format.
    Schema,
}

/// Parse the arguments, run the command and give the exit code. The `owd` binary and the old
/// `cdesign` alias binary both call this.
pub fn main() -> ExitCode {
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
            water_level,
            water_slope,
            water_table,
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
            commands::WaterFlags {
                level: water_level,
                slope: water_slope,
                table: water_table,
            },
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
            fairness_weight,
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
                fairness_weight,
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
        Command::FetchDem {
            doc,
            margin,
            mirror,
            json,
        } => commands::fetch_dem(&doc, margin, mirror.as_deref(), json),
        Command::Reach {
            file,
            river,
            start,
            end,
            via,
            trace,
            title,
            overpass,
            date,
            json,
        } => commands::reach(commands::ReachFlags {
            file,
            river,
            start,
            end,
            via,
            trace,
            title,
            overpass,
            date,
            json,
        }),
        Command::Export {
            doc,
            geojson,
            csv,
            wgs84,
            output,
        } => commands::export(&doc, geojson, csv, wgs84, output.as_deref()),
        Command::Draw {
            sub: Some(DrawSub::Run { document }),
            ..
        } => commands::draw_run(&document),
        Command::Draw {
            doc,
            out,
            document,
            option,
            sheets,
            revision,
            date,
            style,
            section_spacing,
            sheet_size,
            scale,
            json,
            ..
        } => commands::draw(
            &doc.unwrap_or_default(),
            commands::DrawFlags {
                out,
                documents_only: document,
                option,
                sheets,
                revision,
                date,
                style,
                section_spacing,
                sheet_size,
                scale,
            },
            json,
        ),
        Command::Run { doc, out } => commands::run(&doc, &out.output, out.json),
        Command::Validate { doc, json } => commands::validate(&doc, json),
        Command::Status { doc, json } => commands::status(&doc, json),
        Command::Schema => commands::schema(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("owd: {}", e.message);
            ExitCode::from(e.code)
        }
    }
}
