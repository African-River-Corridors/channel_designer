#!/usr/bin/env node
// Web parity (spec 0011): every web fixture gives the same document from the wasm design session as
// from native `owd run`, compared by the ADR 0005 tolerance (1e-9 relative and absolute for numbers,
// equality for everything else), never by bytes. No npm dependencies.
//
//   bash tools/build_web.sh && cargo build --release -p owd-cli && node tools/web_parity.mjs
//
// OWD_BIN overrides the native binary (default <CARGO_TARGET_DIR or target>/release/owd). Use a release
// build: the pool search is far too slow in a debug build.
//
// Each case opens a document (and its DEM) in the wasm session and in a temporary folder for the CLI,
// makes the same edit in both (if the case has one), runs every stale stage in both, and compares the
// two documents. Last, a GeoPackage bank is imported both ways.
import { readFileSync, copyFileSync, mkdtempSync, existsSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { compare, REL_TOL, ABS_TOL } from "./web_parity_compare.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pkg = join(root, "app", "wasm-pkg");
const web = join(root, "examples", "web");
const target = process.env.CARGO_TARGET_DIR || join(root, "target");
const bin = process.env.OWD_BIN || join(target, "release", process.platform === "win32" ? "owd.exe" : "owd");

if (!existsSync(join(pkg, "owd_web.js"))) throw new Error("no wasm build: run tools/build_web.sh first");
if (!existsSync(bin)) throw new Error(`no native owd at ${bin}: run cargo build --release -p owd-cli`);

const { default: init, Session, engine_version } = await import(join(pkg, "owd_web.js"));
await init({ module_or_path: readFileSync(join(pkg, "owd_web_bg.wasm")) });

function owd(args) {
  const r = spawnSync(bin, args, { encoding: "utf8", maxBuffer: 1 << 28 });
  if (r.error) throw r.error;
  // Exit 1 means a check failed; the document is still written.
  if (r.status !== 0 && r.status !== 1) throw new Error(`owd ${args.join(" ")}: ${r.stderr}`);
  return r;
}

const cases = [
  { file: "river-rolling-circle.owd", dem: "river-dem.tif" },
  { file: "river-reroute.owd", dem: "river-dem.tif" },
  // Fresh in the file; the edit makes every stage stale, as it does in the app.
  { file: "river-example.owd", dem: "river-dem.tif", edit: { set: "pianc-wg141", input: ["se_quality", "B"], vessel: "CEMT_IV", draught: "2.6" } },
  // No DEM and nothing stale: both sides must leave it alone.
  { path: join(root, "examples", "synthetic-river.owd") },
];

const rows = [];
let bad = 0;
let compared = 0;
const work = mkdtempSync(join(tmpdir(), "owd-parity-"));

for (const c of cases) {
  const src = c.path ?? join(web, c.file);
  const name = c.path ? "synthetic-river.owd" : c.file;
  const dir = mkdtempSync(join(work, "case-"));
  const doc = join(dir, name);
  copyFileSync(src, doc);
  if (c.dem) copyFileSync(join(web, c.dem), join(dir, c.dem));

  // Native.
  if (c.edit) owd(["size", doc, "--rule-set", c.edit.set, "--set-input", c.edit.input.join("="), "--vessel", c.edit.vessel, "--draught", c.edit.draught]);
  const t0 = Date.now();
  const ran = JSON.parse(owd(["run", "--json", doc]).stdout).ran;
  const nativeMs = Date.now() - t0;
  const nat = JSON.parse(readFileSync(doc, "utf8"));

  // Wasm.
  const s = new Session();
  s.open_text(name, readFileSync(src, "utf8"), c.dem ? readFileSync(join(web, c.dem)) : undefined);
  if (c.edit) {
    s.set_rule_sets(JSON.stringify([c.edit.set]), JSON.stringify({ [c.edit.input[0]]: c.edit.input[1] }));
    s.set_vessel(c.edit.vessel, c.edit.draught);
  }
  const t1 = Date.now();
  // As the app's "Run stale": each stale stage in order, on the DEM (the session's run_stale has no terrain).
  for (const st of JSON.parse(s.snapshot()).stages) {
    if (st.status === "stale") s.run_stage(st.name, undefined, undefined);
  }
  const wasmMs = Date.now() - t1;
  const web_ = JSON.parse(s.save_text(name));
  s.free();

  const r = compare(nat, web_);
  compared += r.numbers;
  const verdict = r.problems.length === 0 ? "equal" : "MISMATCH";
  if (r.problems.length) {
    bad++;
    for (const p of r.problems.slice(0, 8)) console.error(`  ${name}: ${p}`);
  }
  rows.push([name, ran.join("+") || "(nothing stale)", String(r.numbers), r.worst.toExponential(1), `${nativeMs}/${wasmMs} ms`, verdict]);
}

// A GeoPackage bank, read in wasm (SQLite compiled to wasm) and natively.
{
  const gpkg = join(root, "crates", "owd-format", "tests", "data", "gdal", "river.gpkg");
  const dir = mkdtempSync(join(work, "gpkg-"));
  const doc = join(dir, "d.owd");
  owd(["new", doc, "--title", "gpkg", "--date", "2026-10-21"]);
  owd(["import", doc, "--left-bank", `${gpkg}#left_bank`]);
  const nat = JSON.parse(readFileSync(doc, "utf8"));
  const s = new Session();
  s.load_input("left_bank", "river.gpkg", readFileSync(gpkg), "left_bank");
  const web_ = JSON.parse(s.save_text("d.owd"));
  s.free();
  // The bank line and the project CRS the import chose.
  const r = compare({ bank: nat.inputs.left_bank, crs: nat.project.crs }, { bank: web_.inputs.left_bank, crs: web_.project.crs });
  compared += r.numbers;
  const verdict = r.problems.length === 0 && r.numbers > 0 ? "equal" : "MISMATCH";
  if (verdict !== "equal") {
    bad++;
    for (const p of r.problems.slice(0, 8)) console.error(`  gpkg: ${p}`);
  }
  rows.push(["river.gpkg#left_bank", "(import)", String(r.numbers), r.worst.toExponential(1), "-", verdict]);
}

rmSync(work, { recursive: true, force: true });

console.log(`engine ${engine_version()}`);
const head = ["document", "stages run", "numbers", "worst/limit", "native/wasm", "result"];
const all = [head, ...rows];
const widths = head.map((_, c) => Math.max(...all.map((r) => String(r[c]).length)));
for (const r of all) console.log(r.map((v, c) => String(v).padEnd(widths[c])).join("  "));
console.log(`${compared} numbers compared (ADR 0005: ${REL_TOL} relative and ${ABS_TOL} absolute), ${bad} problems`);
if (bad > 0 || compared === 0) {
  console.error("web parity: FAIL");
  process.exit(1);
}
console.log("web parity: ok");
