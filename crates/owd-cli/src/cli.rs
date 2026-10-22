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
enum WaterwaySub {
    /// Mark a reach. Reaches may not overlap; chainage not marked is a free-flowing river.
    Add {
        /// The channel design. `-` is stdin.
        doc: String,
        /// Where the reach starts, metres of river chainage.
        #[arg(long, value_name = "M")]
        from: f64,
        /// Where it ends (default: the end of the river).
        #[arg(long, value_name = "M")]
        to: Option<f64>,
        /// canal, impounded-river or free-flowing-river.
        #[arg(long)]
        waterway: String,
        /// Claim the reduced GB 50139 radius on this canal reach (WG 141 A.3.2). Recorded as typed.
        #[arg(long)]
        claim_reduced_radius: bool,
        #[command(flatten)]
        out: Out,
    },
    /// List the marked reaches.
    List {
        /// The channel design. `-` is stdin.
        doc: String,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Remove the reach that starts at a chainage.
    Remove {
        /// The channel design. `-` is stdin.
        doc: String,
        /// The chainage the reach starts at, metres.
        #[arg(long, value_name = "M")]
        from: f64,
        #[command(flatten)]
        out: Out,
    },
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
    /// Choose the Rule Sets, a vessel class and a draught, and size the channel.
    Size {
        /// The channel design. `-` is stdin.
        doc: String,
        /// A Rule Set to size to, by id or id@version (repeat for more than one; see `owd rules`).
        /// Every selected Rule applies and the strictest governs. `owd` is always selected.
        #[arg(long = "rule-set", value_name = "ID")]
        rule_set: Vec<String>,
        /// A set input, such as se_quality=B, range_end=strict or gb_class=IV (repeatable).
        #[arg(long = "set-input", value_name = "NAME=VALUE")]
        set_input: Vec<String>,
        /// Load a Rule Set file at run time, then select it with --rule-set (repeatable).
        #[arg(long = "rule-set-file", value_name = "FILE")]
        rule_set_file: Vec<String>,
        /// The vessel class, for example CEMT_IV (default: the one the design has).
        #[arg(long)]
        vessel: Option<String>,
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
    /// document with their licence notice, and record them as the DEM. With `owd reach`, one of the
    /// two commands that use the network.
    FetchDem {
        /// The channel design. `-` is not allowed: files are written next to it.
        doc: String,
        /// Metres to add around the river on every side (default 1000).
        #[arg(long, value_name = "METRES", default_value_t = 1000.0)]
        margin: f64,
        /// Fetch for this box instead of the river's extent: `west,south,east,north` in degrees.
        /// A study started from a river reach (`owd reach`) with no banks yet uses its reach.
        #[arg(long, value_name = "W,S,E,N", allow_hyphen_values = true)]
        bbox: Option<String>,
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
    /// kept as way ids. Writes a new document. Uses the network (Overpass). With --search, lists the
    /// rivers whose name starts with or matches a text, and writes nothing.
    Reach {
        /// The `.owd` file to create. `-` is stdout. Not needed with --search.
        #[arg(required_unless_present = "search")]
        file: Option<String>,
        /// List the rivers and canals whose name starts with or matches this text (at least 3
        /// letters), each with its region, country and OSM id. Writes no file. Uses the network
        /// (Photon, OSM data).
        #[arg(long, value_name = "TEXT", conflicts_with_all = ["start", "trace", "banks"])]
        search: Option<String>,
        /// With --search: only rivers in this country, as a two-letter code (for example GH).
        #[arg(long, value_name = "CC", requires = "search")]
        country: Option<String>,
        /// With --search: another Photon server (default: photon.komoot.io).
        #[arg(long, value_name = "URL", requires = "search")]
        search_endpoint: Option<String>,
        /// The river's name, searched in OSM. The route may leave this river.
        #[arg(long, conflicts_with = "osm_id")]
        river: Option<String>,
        /// A whole river from OSM, as --search lists it (for example relation/3328713): the route
        /// runs from its mouth to its upstream end. --start, --end and --via cut a reach out of it.
        #[arg(long, value_name = "OSM_ID", conflicts_with = "trace")]
        osm_id: Option<String>,
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
        /// Also make bank lines, a river polygon and a centreline from the JRC water mask. Reads a few
        /// small windows of the JRC tile over the network, or the tile files you give with --jrc-tile.
        #[arg(long)]
        banks: bool,
        /// A JRC occurrence tile you downloaded (for example occurrence_10W_10Nv1_4_2021.tif), by its
        /// own file name. Repeat for more tiles. Needs --banks.
        #[arg(long, value_name = "FILE", requires = "banks")]
        jrc_tile: Vec<String>,
        /// The JRC occurrence, in percent, from which a pixel is water (default 75, provisional).
        #[arg(long, value_name = "PERCENT", requires = "banks")]
        mask_threshold: Option<f64>,
        /// Then move the mask banks onto the water edge in a dry-season, low-cloud Sentinel-2 scene
        /// (Earth Search STAC; a few small windows of the scene are read). Needs --banks.
        #[arg(long, requires = "banks")]
        edge_fit: bool,
        /// The most cloud, in percent of the scene, for a scene to be used (default 20, provisional).
        #[arg(long, value_name = "PERCENT", requires = "edge_fit")]
        max_cloud: Option<f64>,
        /// Another STAC search endpoint for the Sentinel-2 scenes (default: Earth Search).
        #[arg(long, value_name = "URL", requires = "edge_fit")]
        stac: Option<String>,
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
        /// Export the constraint stretches (spec 0007) instead of the options: as GeoJSON lines or
        /// as CSV, whose head holds the design basis and the verdicts.
        #[arg(long)]
        stretches: bool,
        /// Write the export here instead of stdout.
        #[arg(short = 'o', long = "output", value_name = "FILE")]
        output: Option<String>,
    },
    /// Draw sheets of a design line option: cross sections, the typical section, the setting-out table, plan sheets and the long section. Writes, for
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
        /// The kinds of sheet, comma separated: xs (cross sections), ts (typical section), so (setting-out table and CSV), pl (plan sheets), ls (long section). Default: all.
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
        /// Override the style: the long section's vertical exaggeration, a whole number of 2 or more,
        /// or `auto` (the default: the largest allowed factor whose levels fit the sheet).
        #[arg(long, value_name = "N")]
        vertical_exaggeration: Option<String>,
        /// Draw the plan sheets over this orthophoto (an 8 bit RGB GeoTIFF in WGS84 or the design's UTM
        /// zone) instead of shaded relief.
        #[arg(long, value_name = "GEOTIFF", conflicts_with = "imagery")]
        orthophoto: Option<String>,
        /// Draw the plan sheets over fetched imagery. The one source is `eox-cloudless-2016`
        /// (EOxCloudless 2016, CC BY 4.0; later years need a paid licence for commercial use, so none is
        /// offered). Each sheet prints its attribution. A failed fetch falls back to shaded relief.
        #[arg(long, value_name = "SOURCE")]
        imagery: Option<String>,
        /// Where fetched imagery tiles are kept (default: `$XDG_CACHE_HOME/owd/imagery`, or
        /// `~/.cache/owd/imagery`). Tiles are never written next to the drawings.
        #[arg(long, value_name = "DIR")]
        imagery_cache: Option<String>,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Give the depth of a river that has no survey: one depth, or a table of depths along the
    /// river centreline. Inside the water the volumes then read the bed as the water level less the
    /// depth, shaped as a parabola across the water. The depth is assumed, not measured, and every
    /// output says so. Nothing else in the document changes.
    Bed {
        /// The channel design. `-` is stdin.
        doc: String,
        /// One depth in metres, for the whole river.
        #[arg(long, value_name = "METRES")]
        depth: Option<f64>,
        /// Depths as lines of `chainage_m,depth_m` along the river centreline, read linearly between
        /// lines and held flat beyond the first and last.
        #[arg(long, value_name = "FILE")]
        table: Option<String>,
        /// The season the depths hold for, for example `dry`.
        #[arg(long)]
        season: Option<String>,
        /// Remove the bed model, as a survey does when it replaces it.
        #[arg(long)]
        clear: bool,
        #[command(flatten)]
        out: Out,
    },
    /// Check a document's bank lines (spec 0013): each bank on its own side, no outward jump (an
    /// inlet or a spike), a width plausible against its neighbours, no crossing. Prints what fails and
    /// exits 1 if anything does; changes nothing.
    CheckBanks {
        /// The channel design. `-` is stdin.
        doc: String,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Draw the banks, the centreline and the low-confidence stretches over the Sentinel-2 true-colour
    /// image of the scenes the banks came from, as a PNG (with a legend) and a KMZ, to check a reach
    /// by eye (spec 0013). Needs the network. At most 30 km across: draw a long reach in parts.
    QaOverlay {
        /// The channel design.
        doc: String,
        /// The PNG to write.
        #[arg(long, value_name = "FILE")]
        png: String,
        /// Also write a KMZ for Google Earth.
        #[arg(long, value_name = "FILE")]
        kmz: Option<String>,
        /// Start at this chainage of the centreline, in km.
        #[arg(long, value_name = "KM")]
        from_km: Option<f64>,
        /// End at this chainage of the centreline, in km.
        #[arg(long, value_name = "KM")]
        to_km: Option<f64>,
        /// Draw on this scene (a Sentinel-2 L2A id) instead of the ones the document names. Repeat
        /// for more: later scenes fill where earlier ones have no data.
        #[arg(long, value_name = "ID")]
        scene: Vec<String>,
        /// The STAC search endpoint (default: Earth Search).
        #[arg(long, value_name = "URL")]
        stac: Option<String>,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Say what vessel class fits the river as it is: for every class, under each ruleset, whether
    /// it fits by width and radius, and the stretches that stop it. Width and radius only, no depth.
    /// The answer is kept in the document and shows as stale when the banks change; it never
    /// reruns by itself.
    Fits {
        /// The channel design. `-` is stdin.
        doc: String,
        /// A ruleset to answer under, for example PIANC-B. Repeat for more (default: the ones the
        /// document holds, else PIANC-B).
        #[arg(long, value_name = "RULESET")]
        ruleset: Vec<String>,
        /// Print the answer the document holds, and say if it is stale, without running.
        #[arg(long)]
        show: bool,
        /// The line the full answer reads the river's own radius on: `centreline` (the default),
        /// or `fairway`, each class's fairway fit (ADR 0007) in the water.
        #[arg(long, value_name = "LINE")]
        plan_line: Option<String>,
        #[command(flatten)]
        out: Out,
    },
    /// Mark reaches of the river as canal, impounded river or free-flowing river (spec 0007), so
    /// Rules written for one kind apply only there. Chainage is along the river centreline.
    Waterway {
        #[command(subcommand)]
        action: WaterwaySub,
    },
    /// List the Rule Sets, show one with its Rules and their status, or list the Measures.
    Rules {
        /// `show` to print one set.
        action: Option<String>,
        /// The set to show, by id or id@version.
        id: Option<String>,
        /// List the Measures a Rule may test.
        #[arg(long)]
        measures: bool,
        /// Load a Rule Set file at run time first (repeatable).
        #[arg(long = "rule-set-file", value_name = "FILE")]
        rule_set_file: Vec<String>,
        /// Print a machine-readable result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Set the river's default low-water level: the 10th percentile of each SWORD reach's SWOT levels
    /// (Hydrocron), and the DEM water surface, never rising downstream, where SWOT has too little or
    /// the river is under 100 m wide. Reads the SWORD reaches near the river from Zenodo.
    WaterLevel {
        /// The channel design (with its river centreline or banks, and a DEM for the fallback).
        doc: String,
        /// Another percentile of the SWOT levels (default 10, provisional). The level is then yours.
        #[arg(long, value_name = "P")]
        percentile: Option<f64>,
        /// A SWORD reach narrower than this takes the DEM (default 100 m, provisional).
        #[arg(long, value_name = "METRES")]
        min_swot_width: Option<f64>,
        /// The first date of the SWOT levels, YYYY-MM-DD (default: the start of SWOT's science orbit).
        #[arg(long, value_name = "DATE")]
        from: Option<String>,
        /// The last date, YYYY-MM-DD (default: today).
        #[arg(long, value_name = "DATE")]
        to: Option<String>,
        /// The vertical datum of a DEM that is not Copernicus GLO-30: `egm2008` (default) or `ellipsoid`.
        #[arg(long, value_name = "DATUM")]
        dem_datum: Option<String>,
        #[command(flatten)]
        out: Out,
    },
    /// Print the full Rule Trace of a design (spec 0007): every Rule of every selected set at every
    /// object, with value, limit, margin, outcome, governing flag and status. Rebuilt from the
    /// document, never stored; refused when a stored set hash differs from the engine's.
    Trace {
        /// The channel design. `-` is stdin.
        doc: String,
        /// A design line option: read the station Rules at its stations (default: one per reach).
        #[arg(long)]
        option: Option<String>,
        /// With --option: the station spacing, metres.
        #[arg(long, value_name = "M", default_value_t = 10.0)]
        station_m: f64,
        /// Load a Rule Set file the design selects (repeatable).
        #[arg(long = "rule-set-file", value_name = "FILE")]
        rule_set_file: Vec<String>,
        /// Print the rows as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Rerun every stale stage, in order.
    Run {
        /// The channel design. `-` is stdin.
        doc: String,
        /// Load a Rule Set file the design selects (repeatable).
        #[arg(long = "rule-set-file", value_name = "FILE")]
        rule_set_file: Vec<String>,
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

/// Parse the arguments, run the command and give the exit code. The `owd` binary calls this.
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
        Command::Bed {
            doc,
            depth,
            table,
            season,
            clear,
            out,
        } => crate::bed::bed(
            &doc,
            depth,
            table.as_deref(),
            season.as_deref(),
            clear,
            &out.output,
            out.json,
        ),
        Command::WaterLevel {
            doc,
            percentile,
            min_swot_width,
            from,
            to,
            dem_datum,
            out,
        } => crate::level::water_level(crate::level::LevelFlags {
            doc,
            percentile,
            min_swot_width,
            from,
            to,
            dem_datum,
            output: out.output,
            json: out.json,
        }),
        Command::CheckBanks { doc, json } => crate::check_banks::check_banks(&doc, json),
        Command::QaOverlay {
            doc,
            png,
            kmz,
            from_km,
            to_km,
            scene,
            stac,
            json,
        } => crate::qa_overlay::qa_overlay(crate::qa_overlay::OverlayFlags {
            doc,
            png,
            kmz,
            from_km,
            to_km,
            scene,
            stac,
            json,
        }),
        Command::Fits {
            doc,
            ruleset,
            show,
            plan_line,
            out,
        } => crate::fits::fits(
            &doc,
            &ruleset,
            show,
            plan_line.as_deref(),
            &out.output,
            out.json,
        ),
        Command::Size {
            doc,
            rule_set,
            set_input,
            rule_set_file,
            vessel,
            draught,
            out,
        } => commands::size(
            &doc,
            commands::SizeFlags {
                rule_sets: rule_set,
                set_inputs: set_input,
                files: rule_set_file,
                vessel,
                draught,
            },
            &out.output,
            out.json,
        ),
        Command::Waterway { action } => match action {
            WaterwaySub::Add {
                doc,
                from,
                to,
                waterway,
                claim_reduced_radius,
                out,
            } => commands::waterway_add(
                &doc,
                from,
                to,
                &waterway,
                claim_reduced_radius,
                &out.output,
                out.json,
            ),
            WaterwaySub::List { doc, json } => commands::waterway_list(&doc, json),
            WaterwaySub::Remove { doc, from, out } => {
                commands::waterway_remove(&doc, from, &out.output, out.json)
            }
        },
        Command::Rules {
            action,
            id,
            measures,
            rule_set_file,
            json,
        } => commands::rules(
            action.as_deref(),
            id.as_deref(),
            measures,
            &rule_set_file,
            json,
        ),
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
            bbox,
            mirror,
            json,
        } => commands::fetch_dem(&doc, margin, mirror.as_deref(), bbox.as_deref(), json),
        Command::Reach {
            search: Some(text),
            country,
            search_endpoint,
            json,
            ..
        } => commands::reach_search(&text, country.as_deref(), search_endpoint.as_deref(), json),
        Command::Reach {
            file,
            river,
            osm_id,
            start,
            end,
            via,
            trace,
            banks,
            jrc_tile,
            mask_threshold,
            edge_fit,
            max_cloud,
            stac,
            title,
            overpass,
            date,
            json,
            ..
        } => commands::reach(commands::ReachFlags {
            banks,
            jrc_tile,
            mask_threshold,
            edge_fit,
            max_cloud,
            stac,
            file: file.unwrap_or_default(),
            river,
            osm_id,
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
            stretches,
            output,
        } => commands::export(&doc, geojson, csv, wgs84, stretches, output.as_deref()),
        Command::Trace {
            doc,
            option,
            station_m,
            rule_set_file,
            json,
        } => commands::trace(&doc, option.as_deref(), station_m, &rule_set_file, json),
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
            vertical_exaggeration,
            orthophoto,
            imagery,
            imagery_cache,
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
                vertical_exaggeration,
                orthophoto,
                imagery,
                imagery_cache,
            },
            json,
        ),
        Command::Run {
            doc,
            rule_set_file,
            out,
        } => commands::run(&doc, &rule_set_file, &out.output, out.json),
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
