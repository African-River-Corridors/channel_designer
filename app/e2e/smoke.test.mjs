// End-to-end smoke test (spec 0003, ticket 68): one journey on the built app, driven through Tauri's
// WebDriver (`tauri-driver`). Run by `e2e/run.sh`, which starts the driver and sets
// CD_TEST_DIALOG_DIR: a debug build answers its native dialogs from files in that folder, since
// WebDriver cannot click a native dialog (see `test_answer` in src-tauri/src/lib.rs).
import { after, before, beforeEach, describe, it } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { remote } from "webdriverio";

const here = import.meta.dirname;
const EXAMPLE = path.resolve(here, "../../examples/synthetic-river.cdesign");
const APP = path.resolve(here, process.env.CD_APP_BINARY ?? "../../target/debug/cd-app");
const DIALOGS = process.env.CD_TEST_DIALOG_DIR;
assert.ok(DIALOGS, "CD_TEST_DIALOG_DIR must be set (run e2e/run.sh)");
const WORK = fs.mkdtempSync(path.join(os.tmpdir(), "cd-e2e-work-"));

const answer = (file, text) => fs.writeFileSync(path.join(DIALOGS, file), text);
const forget = (file) => fs.rmSync(path.join(DIALOGS, file), { force: true });

let browser;

/** What the page shows, for a failure message: the file label, the error banner and the chosen vessel. */
const pageState = () =>
  browser
    .execute(() => ({
      label: document.querySelector("[data-testid=file-label]")?.textContent,
      error: document.querySelector("[data-testid=error]")?.textContent?.trim() ?? null,
      vessel: document.querySelector("select[aria-label='Vessel class']")?.value,
      radius: document.querySelector("[data-testid=min-radius]")?.textContent,
    }))
    .then((s) => JSON.stringify(s))
    .catch((e) => `(page state unavailable: ${e.message})`);

const until = async (what, check, timeout = 15000) => {
  try {
    await browser.waitUntil(check, { timeout, interval: 100 });
  } catch {
    throw new Error(`timed out waiting for: ${what}; page: ${await pageState()}`);
  }
};

const text = async (testid) => (await browser.$(`[data-testid=${testid}]`)).getText();
const fileLabel = () => text("file-label");


/** A small uncompressed float32 GeoTIFF in WGS 84 / UTM 30N: 80 x 12 cells of 100 m over the example river, rolling hills. */
function demTiff() {
  const [w, h, cell, x0, y0] = [80, 12, 100, 499800, 600600];
  const pixels = Buffer.alloc(w * h * 4);
  for (let r = 0; r < h; r++)
    for (let c = 0; c < w; c++) pixels.writeFloatLE(40 + 6 * Math.sin(c / 6) + 4 * Math.cos(r / 2), (r * w + c) * 4);
  // One IFD; tag values that do not fit in four bytes live after it.
  const entries = [];
  const extra = [];
  const IFD_AT = 8;
  const nEntries = 13;
  let extraAt = IFD_AT + 2 + nEntries * 12 + 4;
  const put = (tag, type, count, value) => entries.push({ tag, type, count, value });
  const putBlob = (tag, type, count, bytes) => {
    const at = extraAt + extra.reduce((n, b) => n + b.length, 0);
    extra.push(bytes);
    entries.push({ tag, type, count, value: at });
  };
  const f64s = (...v) => Buffer.concat(v.map((x) => { const b = Buffer.alloc(8); b.writeDoubleLE(x); return b; }));
  const u16s = (...v) => Buffer.concat(v.map((x) => { const b = Buffer.alloc(2); b.writeUInt16LE(x); return b; }));
  const dataAt = extraAt + 24 + 48 + 16; // after the three blobs below
  put(256, 4, 1, w);
  put(257, 4, 1, h);
  put(258, 3, 1, 32);
  put(259, 3, 1, 1);
  put(262, 3, 1, 1);
  put(273, 4, 1, dataAt);
  put(277, 3, 1, 1);
  put(278, 4, 1, h);
  put(279, 4, 1, pixels.length);
  put(339, 3, 1, 3);
  putBlob(33550, 12, 3, f64s(cell, cell, 0));
  putBlob(33922, 12, 6, f64s(0, 0, 0, x0, y0, 0));
  putBlob(34735, 3, 8, u16s(1, 1, 0, 1, 3072, 0, 1, 32630)); // ProjectedCSTypeGeoKey = EPSG:32630
  const head = Buffer.alloc(IFD_AT + 2 + nEntries * 12 + 4);
  head.write("II", 0);
  head.writeUInt16LE(42, 2);
  head.writeUInt32LE(IFD_AT, 4);
  head.writeUInt16LE(nEntries, IFD_AT);
  entries.sort((a, b) => a.tag - b.tag).forEach((e, i) => {
    const o = IFD_AT + 2 + i * 12;
    head.writeUInt16LE(e.tag, o);
    head.writeUInt16LE(e.type, o + 2);
    head.writeUInt32LE(e.count, o + 4);
    if (e.type === 3 && e.count === 1) head.writeUInt16LE(e.value, o + 8);
    else head.writeUInt32LE(e.value, o + 8);
  });
  return Buffer.concat([head, ...extra, pixels]);
}


/** The worked example with a DEM beside it, in a fresh folder. Returns the .cdesign path. */
function designWithDem(prefix) {
  const dir = fs.mkdtempSync(path.join(WORK, prefix));
  const tif = demTiff();
  fs.writeFileSync(path.join(dir, "dem.tif"), tif);
  const doc = JSON.parse(fs.readFileSync(EXAMPLE, "utf8"));
  doc.inputs.dem = { path: "dem.tif", sha256: crypto.createHash("sha256").update(tif).digest("hex"), crs: "EPSG:32630", nodata: -9999 };
  const file = path.join(dir, "with-dem.cdesign");
  fs.writeFileSync(file, JSON.stringify(doc, null, 2));
  return file;
}

/** The values of the vessel class list, and the one now chosen. */
const vesselChoices = () =>
  browser.execute(() => {
    const select = document.querySelector("select[aria-label='Vessel class']");
    return { values: [...select.options].map((o) => o.value), current: select.value };
  });

/** Choose a vessel class. WebDriver sets the value; the change event is sent too, in case the driver did not. */
async function chooseVessel(value) {
  const select = await browser.$("select[aria-label='Vessel class']");
  await select.selectByAttribute("value", value);
  await browser.execute(() => {
    document.querySelector("select[aria-label='Vessel class']").dispatchEvent(new Event("change", { bubbles: true }));
  });
}

/** The worked example is sized for this class. After an open the form takes a moment to show it. */
const EXAMPLE_VESSEL = "CEMT_IV";
const formShowsExample = () => until("the form to show the example's vessel", async () => (await vesselChoices()).current === EXAMPLE_VESSEL);

/** Click a header button, after the dialog answer is in place. */
async function click(label) {
  await (await browser.$(`button=${label}`)).click();
}

/** Open a file and wait for it to load. */
async function open(file, expectName) {
  answer("open.txt", file);
  await click("Open");
  await until(`${expectName} to open`, async () => (await fileLabel()).includes(expectName));
}

describe("Channel Designer app smoke test", () => {
  before(async () => {
    browser = await remote({
      hostname: "127.0.0.1",
      port: 4444,
      path: "/",
      logLevel: "warn",
      strictSelectors: false,
      capabilities: { "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    await until("the app to draw", async () => (await browser.$("header")).isExisting(), 60000);
  });

  after(async () => {
    await browser?.deleteSession();
  });

  beforeEach(() => {
    for (const f of ["open.txt", "save.txt", "discard.txt", "load.txt"]) forget(f);
  });

  it("shows the screening label on its status line from the start", async () => {
    assert.equal(await text("status-line"), "Screening — not for construction");
  });

  it("opens the worked example, changes the vessel, sees the channel shape change, saves and reopens", async () => {
    await open(EXAMPLE, "synthetic-river.cdesign");
    await formShowsExample();
    assert.equal(await text("project-crs"), "WGS 84 / UTM zone 30N");
    assert.equal(await text("status-sizing"), "fresh");
    assert.equal(await text("status-alignment"), "not run");
    const radiusBefore = await text("min-radius");
    const widthBefore = await text("bottom-width");

    // Change the vessel class: the channel shape changes, after the pause, with no button pressed.
    const { values, current } = await vesselChoices();
    const other = values.find((v) => v !== current && v.startsWith("CEMT"));
    assert.ok(other, `another CEMT class to choose among ${values.join(" | ")}`);
    await chooseVessel(other);
    await until("the channel shape to change", async () => (await text("min-radius")) !== radiusBefore);
    assert.notEqual(await text("bottom-width"), widthBefore);
    assert.equal(await text("status-sizing"), "fresh", "the new sizing is already run");
    assert.match(await fileLabel(), /\(unsaved\)/);

    // Save as: the unsaved marker goes.
    const saved = path.join(WORK, "changed.cdesign");
    answer("save.txt", saved);
    await click("Save as");
    await until("the file to be saved", async () => !(await fileLabel()).includes("(unsaved)"));
    assert.ok(fs.existsSync(saved));
    const doc = JSON.parse(fs.readFileSync(saved, "utf8"));
    assert.equal(doc.stages.sizing.settings.vessel_standard_id, other);
    assert.equal(doc.stages.sizing.results.vessel_standard_id, other);

    // Reopen it: the same channel shape.
    const radiusAfter = await text("min-radius");
    await open(EXAMPLE, "synthetic-river.cdesign");
    assert.equal(await text("min-radius"), radiusBefore);
    await open(saved, "changed.cdesign");
    assert.equal(await text("min-radius"), radiusAfter);
    assert.equal(await text("status-sizing"), "fresh");
  });

  it("asks before it throws away unsaved changes, and keeps them when the answer is no", async () => {
    await open(EXAMPLE, "synthetic-river.cdesign");
    await formShowsExample();
    const { values, current } = await vesselChoices();
    const other = values.find((v) => v !== current && v.startsWith("CEMT"));
    await chooseVessel(other);
    await until("the edit to count as unsaved", async () => (await fileLabel()).includes("(unsaved)"));
    // Refuse the discard: nothing changes.
    answer("discard.txt", "no");
    await click("New");
    await browser.pause(500);
    assert.match(await fileLabel(), /synthetic-river\.cdesign \(unsaved\)/);
    // Agree: a new, empty design.
    answer("discard.txt", "yes");
    await click("New");
    await until("a new design", async () => (await fileLabel()).startsWith("Untitled"));
    assert.equal(await text("project-crs"), "none yet");
    assert.equal(await text("status-sizing"), "not run");
  });
  it("shows the basemap attribution on the map, before and after zooming", async () => {
    const ATTRIBUTION =
      "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2016)";
    const shown = async () => {
      const el = await browser.$("[data-testid=attribution]");
      return (await el.isDisplayed()) && (await el.getText()) === ATTRIBUTION;
    };
    await until("the attribution to show", shown);
    // Zoom with the map's own buttons when the map started (it needs WebGL; the attribution does not).
    const zoomIn = await browser.$(".maplibregl-ctrl-zoom-in");
    if (await zoomIn.isExisting()) {
      for (let i = 0; i < 4; i++) {
        await zoomIn.click();
        await browser.pause(300);
        assert.ok(await shown(), `the attribution is visible after zoom step ${i + 1}`);
      }
    }
  });
  it("shows the DEM hillshade and an imagery-offline note when the tiles cannot load", async () => {
    // run.sh points every tile request at a dead address. The design names a DEM beside it.
    const file = designWithDem("hillshade-");
    await open(file, "with-dem.cdesign");
    await until("the offline note", async () => (await (await browser.$("[data-testid=offline-note]")).isExisting()), 30000);
    await until("the hillshade", async () => (await (await browser.$("[data-testid=plan-map]")).getAttribute("data-hillshade")) === "shown", 30000);
    assert.match(await text("offline-note"), /Imagery offline/);
    // The attribution stays up while the imagery is off.
    assert.ok(await (await browser.$("[data-testid=attribution]")).isDisplayed());
  });
  it("loads data by file dialog and shows the UTM zone it was moved to", async () => {
    answer("discard.txt", "yes");
    await click("New");
    await until("a new design", async () => (await fileLabel()).startsWith("Untitled"));
    // A bank line in plain WGS84 longitude and latitude.
    const bank = path.join(WORK, "left-bank-wgs84.geojson");
    fs.writeFileSync(bank, JSON.stringify({ type: "LineString", coordinates: [[-3.0, 5.4], [-2.99, 5.41], [-2.98, 5.41]] }));
    answer("load.txt", bank);
    await (await browser.$("[data-testid=load-left_bank]")).click();
    await until("the bank to load", async () => (await text("data-left_bank")).includes("3 positions"));
    assert.equal(await text("project-crs"), "WGS 84 / UTM zone 30N");
    assert.match(await text("moved-note"), /EPSG:4326 was moved to WGS 84 \/ UTM zone 30N/);
    // A refused file shows the reason and changes nothing.
    const bad = path.join(WORK, "not-a-line.geojson");
    fs.writeFileSync(bad, "{");
    answer("load.txt", bad);
    await (await browser.$("[data-testid=load-right_bank]")).click();
    await until("the refusal", async () => (await browser.$("[data-testid=error]")).isExisting());
    assert.equal(await text("data-right_bank"), "not loaded");
  });

  it("runs alignment, then volumes, and the stages turn fresh; editing the vessel makes them stale", async () => {
    await open(designWithDem("run-"), "with-dem.cdesign");
    await formShowsExample();
    assert.equal(await text("status-alignment"), "not run");
    await (await browser.$("[data-testid=run-alignment]")).click();
    await until("alignment to be fresh", async () => (await text("status-alignment")) === "fresh", 60000);
    // Volumes needs a water level: the session says so, and the app shows its words.
    await (await browser.$("[data-testid=run-volumes]")).click();
    await until("the missing water level to be named", async () => ((await (await browser.$("[data-testid=error]")).getText()) ?? "").includes("water level"), 60000);
    await (await browser.$("[data-testid=water-level]")).setValue("38");
    await (await browser.$("[data-testid=set-water]")).click();
    await until("the water level to be set", async () => !(await (await browser.$("[data-testid=error]")).isExisting()));
    await (await browser.$("[data-testid=run-volumes]")).click();
    await until("volumes to be fresh", async () => (await text("status-volumes")) === "fresh", 120000);
    assert.equal(await text("status-alignment"), "fresh");
    // Change the vessel: the later stages go stale, and nothing reruns by itself.
    const { values, current } = await vesselChoices();
    await chooseVessel(values.find((v) => v !== current && v.startsWith("CEMT")));
    await until("volumes to go stale", async () => (await text("status-volumes")) === "stale");
    assert.equal(await text("status-alignment"), "stale");
  });
  it("sorts a failing option first with a red mark and its numbers, and selecting a row selects the option on the map", async () => {
    // Two options in the document, the passing one first. The failing one reads 222 m against R_min 330 m.
    const option = (id, name, ok, measured, y) => ({
      id, name, method: "rolling_circle", settings: {},
      line: { type: "LineString", coordinates: [[500000, y], [501000, y + 100], [502000, y]] },
      checks: { r_min_m: 330, radius_ok: ok, measured_min_radius_m: measured, radius_check_chord_m: 110 },
      details: {},
    });
    const doc = JSON.parse(fs.readFileSync(EXAMPLE, "utf8"));
    doc.options = [option("rolling-circle", "Rolling circle", true, 340, 600000), option("reroute", "Least-cost re-route", false, 222, 600150)];
    const file = path.join(WORK, "two-options.cdesign");
    fs.writeFileSync(file, JSON.stringify(doc, null, 2));
    await open(file, "two-options.cdesign");
    await until("the table to fill", async () => (await browser.$$("[data-testid=option-row]")).length === 2);
    const rows = await browser.execute(() =>
      [...document.querySelectorAll("[data-testid=option-row]")].map((r) => ({
        id: r.dataset.option,
        failed: r.dataset.failed,
        text: r.textContent.replace(/\s+/g, " ").trim(),
        minRadius: r.querySelector("[data-col=min-radius]").textContent,
      })),
    );
    assert.equal(rows[0].id, "reroute", "the failing option sorts first");
    assert.equal(rows[0].failed, "true");
    assert.match(rows[0].text, /✕ fails/);
    assert.equal(rows[0].minRadius, "222", "its numbers stay visible");
    assert.equal(rows[1].id, "rolling-circle");
    assert.doesNotMatch(rows[1].text, /✕/);
    // The checks list has the failure first, too.
    const checks = await browser.execute(() => [...document.querySelectorAll("[data-testid=check]")].map((c) => c.dataset.passed));
    assert.deepEqual(checks, ["false", "true"]);
    // The failing option starts selected; selecting the other row moves the map's selection.
    const mapSelected = async () => (await (await browser.$("[data-testid=plan-map]")).getAttribute("data-selected")) ?? "";
    await until("the map to select the failing option", async () => (await mapSelected()) === "reroute", 30000);
    await (await browser.$("[data-testid=option-row][data-option=rolling-circle]")).click();
    await until("the map to follow the table", async () => (await mapSelected()) === "rolling-circle", 30000);
  });
});
