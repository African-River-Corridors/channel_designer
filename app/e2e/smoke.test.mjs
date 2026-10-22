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
import { endpoints, searches, serveReach, slow } from "./reach-server.mjs";

const here = import.meta.dirname;
const EXAMPLE = path.resolve(here, "../../examples/synthetic-river.owd");
const APP = path.resolve(here, process.env.CD_APP_BINARY ?? "../../target/debug/owd-app");
const DIALOGS = process.env.CD_TEST_DIALOG_DIR;
assert.ok(DIALOGS, "CD_TEST_DIALOG_DIR must be set (run e2e/run.sh)");
const WORK = fs.mkdtempSync(path.join(os.tmpdir(), "cd-e2e-work-"));
const PROJECTS = path.join(WORK, "projects");

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
/** Open "Start from a river" if it is not open: the button toggles the view. */
const openReachView = async () => {
  const b = await browser.$("[data-testid=start-from-river]");
  if ((await b.getAttribute("aria-pressed")) !== "true") await b.click();
  await until("the reach view", async () => (await browser.$("[data-testid=reach-view]")).isExisting());
};
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


/** The worked example with a DEM beside it, in a fresh folder. Returns the .owd path. */
function designWithDem(prefix) {
  const dir = fs.mkdtempSync(path.join(WORK, prefix));
  const tif = demTiff();
  fs.writeFileSync(path.join(dir, "dem.tif"), tif);
  const doc = JSON.parse(fs.readFileSync(EXAMPLE, "utf8"));
  doc.inputs.dem = { path: "dem.tif", sha256: crypto.createHash("sha256").update(tif).digest("hex"), crs: "EPSG:32630", nodata: -9999 };
  const file = path.join(dir, "with-dem.owd");
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

/** Choose a value in a list by its test id, and send the change event once, as a user's pick does. */
const choose = (testid, value) =>
  browser.execute(
    (id, v) => {
      const select = document.querySelector(`[data-testid=${id}]`);
      select.value = v;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    },
    testid,
    value,
  );

/** The worked example is sized for this class. After an open the form takes a moment to show it. */
const EXAMPLE_VESSEL = "CEMT-IV"; // the catalogue id (spec 0015 T0); older files say CEMT_IV
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

describe("Open Waterway Designer app smoke test", () => {
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
    for (const f of ["open.txt", "save.txt", "discard.txt", "load.txt", "export.txt", "sheets.txt", "report.txt", "qa.txt", "jrc.txt", "reach-endpoints.json", "dem-mirror.txt", "jrc-mirror.txt"]) forget(f);
    // Studies from a river save themselves here, not in the runner's home (#322).
    answer("projects.txt", PROJECTS);
  });

  it("shows the screening label on its status line from the start", async () => {
    assert.equal(await text("status-line"), "Screening — not for construction");
  });

  it("opens the worked example, changes the vessel, sees the channel shape change, saves and reopens", async () => {
    await open(EXAMPLE, "synthetic-river.owd");
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
    const saved = path.join(WORK, "changed.owd");
    answer("save.txt", saved);
    await click("Save as");
    await until("the file to be saved", async () => !(await fileLabel()).includes("(unsaved)"));
    assert.ok(fs.existsSync(saved));
    const doc = JSON.parse(fs.readFileSync(saved, "utf8"));
    assert.equal(doc.stages.sizing.settings.vessel_standard_id, other);
    assert.equal(doc.stages.sizing.results.vessel_standard_id, other);

    // Reopen it: the same channel shape.
    const radiusAfter = await text("min-radius");
    await open(EXAMPLE, "synthetic-river.owd");
    assert.equal(await text("min-radius"), radiusBefore);
    await open(saved, "changed.owd");
    assert.equal(await text("min-radius"), radiusAfter);
    assert.equal(await text("status-sizing"), "fresh");
  });

  it("asks before it throws away unsaved changes, and keeps them when the answer is no", async () => {
    await open(EXAMPLE, "synthetic-river.owd");
    await formShowsExample();
    const { values, current } = await vesselChoices();
    const other = values.find((v) => v !== current && v.startsWith("CEMT"));
    await chooseVessel(other);
    await until("the edit to count as unsaved", async () => (await fileLabel()).includes("(unsaved)"));
    // Refuse the discard: nothing changes.
    answer("discard.txt", "no");
    await click("New");
    await browser.pause(500);
    assert.match(await fileLabel(), /synthetic-river\.owd \(unsaved\)/);
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
    await open(file, "with-dem.owd");
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
    await open(designWithDem("run-"), "with-dem.owd");
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
  it("refuses a .cdesign file with a message to rename it to .owd", async () => {
    // The old name of the design file is no longer read (spec 0006, T10); the content is the same.
    const file = path.join(WORK, "old-name.cdesign");
    fs.copyFileSync(EXAMPLE, file);
    answer("open.txt", file);
    await click("Open");
    await until("the refusal to be shown", async () => ((await (await browser.$("[data-testid=error]")).getText()) ?? "").includes("rename it to `.owd`"));
    assert.doesNotMatch(await fileLabel(), /old-name/);
  });
  it("picks a second Rule Set and its input, runs, and shows the verdict, the design basis and the constraint stretches on the table and the map", async () => {
    await open(designWithDem("rules-"), "with-dem.owd");
    await formShowsExample();
    // One set, as the example has it, with its mark in words.
    assert.equal(await (await browser.$("[data-testid=rule-set-0]")).getValue(), "pianc-wg141");
    assert.match(await text("source-set-0"), /typed/);
    assert.match(await text("always-owd"), /Always applied/);
    assert.equal(await (await browser.$("[data-testid=rule-set-1]")).isExisting(), false);
    // Add GB 50139: it has no default end of range, so the form asks for one before it sends.
    await (await browser.$("[data-testid=add-rule-set]")).click();
    await until("the second set", async () => (await (await browser.$("[data-testid=rule-set-1]")).isExisting()) && (await (await browser.$("[data-testid=rule-set-1]")).getValue()) === "gb50139");
    assert.match(await text("rule-set-missing"), /range/);
    assert.match(await text("source-set-1"), /not sent yet/);
    await choose("set-input-range_end", "strict");
    await until("sizing under both sets", async () => (await text("shape-rule-sets")).includes("gb50139@0.3.0"));
    assert.match(await text("source-input-range_end"), /typed/);
    assert.match(await text("source-set-1"), /typed/);
    // The design basis: each quantity with its governing Rule, and every Rule unverified, in words.
    const basis = await browser.execute(() =>
      [...document.querySelectorAll("[data-testid=basis-row]")].map((r) => ({ q: r.dataset.quantity, status: r.querySelector("[data-testid=basis-status]").textContent })),
    );
    assert.ok(basis.length >= 3, JSON.stringify(basis));
    for (const b of basis) assert.match(b.status, /unverified/, b.q);
    assert.match(await text("rules-notice"), /unverified/);
    // Run alignment and volumes.
    await (await browser.$("[data-testid=run-alignment]")).click();
    await until("alignment to be fresh", async () => (await text("status-alignment")) === "fresh", 60000);
    await (await browser.$("[data-testid=water-level]")).setValue("38");
    await (await browser.$("[data-testid=set-water]")).click();
    await until("the water level to be set", async () => !(await (await browser.$("[data-testid=error]")).isExisting()));
    await (await browser.$("[data-testid=run-volumes]")).click();
    await until("volumes to be fresh", async () => (await text("status-volumes")) === "fresh", 120000);
    // The verdict per option, in words and a shape.
    await until("a verdict", async () => (await browser.$$("[data-testid=verdict]")).length > 0, 30000);
    const verdicts = await browser.execute(() => [...document.querySelectorAll("[data-testid=verdict]")].map((v) => [v.dataset.verdict, v.textContent.trim()]));
    for (const [v, words] of verdicts) {
      assert.ok(["navigable-as-is", "navigable-with-works", "not-navigable"].includes(v), v);
      assert.match(words, new RegExp(v.replace(/-/g, " ")));
    }
    // The constraint stretches of the selected option, with the screening notices.
    const tab = await browser.$("[data-testid=tab-stretches]");
    await tab.scrollIntoView();
    await browser.execute((el) => el.click(), tab);
    await until("the stretch table", async () => (await browser.$$("[data-testid=stretch-row]")).length > 0, 30000);
    assert.match(await text("screening-notice"), /Screening/);
    assert.match(await text("screening-notice"), /unverified/);
    const first = await browser.execute(() => {
      const r = document.querySelector("[data-testid=stretch-row]");
      return { kind: r.dataset.kind, text: r.textContent.replace(/\s+/g, " ").trim() };
    });
    assert.ok(first.text.includes(first.kind), "the kind is a word");
    // Picking a row draws it on the map.
    const option = await (await browser.$("[data-testid=plan-map]")).getAttribute("data-selected");
    const row = await browser.$("[data-testid=stretch-row][data-index='0']");
    await row.scrollIntoView();
    await browser.execute((el) => el.click(), row);
    await until("the map to show the picked stretch", async () => (await (await browser.$("[data-testid=plan-map]")).getAttribute("data-stretch")) === `${option}:0`, 30000);
    // Removing the second set sizes again under one set, and the later stages go stale.
    const remove = await browser.$("[data-testid=remove-rule-set-1]");
    await remove.scrollIntoView();
    await browser.execute((el) => el.click(), remove);
    await until("one set again", async () => !(await text("shape-rule-sets")).includes("gb50139"));
    await until("volumes to go stale", async () => (await text("status-volumes")) === "stale");
    // Back to the options tab, as the next tests expect: the tab stays as picked across files.
    const options = await browser.$("[data-testid=tab-options]");
    await browser.execute((el) => el.click(), options);
    await until("the options tab", async () => (await options.getAttribute("aria-selected")) === "true");
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
    const file = path.join(WORK, "two-options.owd");
    fs.writeFileSync(file, JSON.stringify(doc, null, 2));
    await open(file, "two-options.owd");
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
    // The panel is short: scroll the row into view, then click it.
    const row = await browser.$("[data-testid=option-row][data-option=rolling-circle]");
    await row.scrollIntoView();
    await browser.execute((el) => el.click(), row);
    await until("the map to follow the table", async () => (await mapSelected()) === "rolling-circle", 30000);
  });
  it("moves the cross-section and the map marker with the station slider, from the volumes' own station walk", async () => {
    await open(designWithDem("sections-"), "with-dem.owd");
    await formShowsExample();
    await (await browser.$("[data-testid=run-alignment]")).click();
    await until("alignment to be fresh", async () => (await text("status-alignment")) === "fresh", 60000);
    await (await browser.$("[data-testid=water-level]")).setValue("38");
    await (await browser.$("[data-testid=set-water]")).click();
    await (await browser.$("[data-testid=run-volumes]")).click();
    await until("volumes to be fresh", async () => (await text("status-volumes")) === "fresh", 120000);
    // The long section of the selected option, and a cross-section at its first station.
    await until("the long section", async () => Number(await (await browser.$("[data-testid=long-section]")).getAttribute("data-stations")) > 2, 60000);
    const stations = Number(await (await browser.$("[data-testid=long-section]")).getAttribute("data-stations"));
    await until("the cross-section", async () => (await (await browser.$("[data-testid=cross-section]")).getAttribute("data-chainage")) !== "", 60000);
    const slideTo = (i) =>
      browser.execute((i) => {
        const el = document.querySelector("[data-testid=station-slider]");
        el.value = String(i);
        el.dispatchEvent(new Event("input", { bubbles: true }));
      }, i);
    const snapshotOf = () =>
      browser.execute(() => ({
        chainage: document.querySelector("[data-testid=cross-section]").dataset.chainage,
        terrain: document.querySelector("[data-testid=cs-terrain]")?.getAttribute("d") ?? "",
        marker: document.querySelector(".station-marker")?.dataset.position ?? null,
      }));
    await slideTo(0);
    await until("the first station", async () => (await snapshotOf()).chainage === "0");
    await until("the map marker", async () => (await snapshotOf()).marker !== null, 30000);
    const first = await snapshotOf();
    await slideTo(Math.floor(stations / 2));
    await until("the cross-section to change", async () => (await snapshotOf()).chainage !== first.chainage, 30000);
    await until("the marker to move", async () => (await snapshotOf()).marker !== first.marker, 30000);
    const middle = await snapshotOf();
    assert.notEqual(middle.terrain, first.terrain, "a different station has a different ground profile");
    // Edit the vessel: the volumes go stale, and the sections say so rather than showing old numbers.
    const { values, current } = await vesselChoices();
    await chooseVessel(values.find((v) => v !== current && v.startsWith("CEMT")));
    await until("the sections to be withdrawn", async () => (await (await browser.$("[data-testid=section-note]")).isExisting()), 30000);
  });
  it("fills the option table, saves and reopens to the same table, and exports with the screening label", async () => {
    await open(designWithDem("full-"), "with-dem.owd");
    await formShowsExample();
    await (await browser.$("[data-testid=run-alignment]")).click();
    await until("alignment to be fresh", async () => (await text("status-alignment")) === "fresh", 60000);
    await (await browser.$("[data-testid=water-level]")).setValue("38");
    await (await browser.$("[data-testid=set-water]")).click();
    await (await browser.$("[data-testid=run-volumes]")).click();
    await until("volumes to be fresh", async () => (await text("status-volumes")) === "fresh", 120000);
    const tableRows = () =>
      browser.execute(() => [...document.querySelectorAll("[data-testid=option-row]")].map((r) => r.textContent.replace(/\s+/g, " ").trim()));
    await until("the table to fill", async () => {
      const rows = await tableRows();
      return rows.length > 0 && rows.every((r) => !r.includes("—"));
    }, 30000);
    const before = await tableRows();

    // Save, open something else, reopen: the same table.
    const saved = path.join(WORK, "full-saved.owd");
    answer("save.txt", saved);
    await click("Save as");
    await until("the file to be saved", async () => !(await fileLabel()).includes("(unsaved)"));
    await open(EXAMPLE, "synthetic-river.owd");
    await open(saved, "full-saved.owd");
    await until("the table to return", async () => (await tableRows()).length === before.length, 30000);
    assert.deepEqual(await tableRows(), before);
    assert.equal(await text("status-volumes"), "fresh");

    // Exports go through the session and say what they are.
    const csv = path.join(WORK, "full.csv");
    answer("export.txt", csv);
    await click("Export CSV");
    await until("the CSV to be written", async () => fs.existsSync(csv));
    assert.equal(fs.readFileSync(csv, "utf8").split("\n")[0], "# Screening \u2014 not for construction");
    const gj = path.join(WORK, "full.geojson");
    answer("export.txt", gj);
    await click("Export GeoJSON");
    await until("the GeoJSON to be written", async () => fs.existsSync(gj));
    const fc = JSON.parse(fs.readFileSync(gj, "utf8"));
    assert.equal(fc.screening, "Screening \u2014 not for construction");
    assert.ok(fc.features.length >= 1);
    assert.ok(fc.features.every((f) => f.properties.screening === "Screening \u2014 not for construction" && Array.isArray(f.properties.failed_checks)));
  });
  it("shows the drawing sheets of the active option and exports them as PDF (spec 0004)", async () => {
    await open(designWithDem("sheets-"), "with-dem.owd");
    await formShowsExample();
    await (await browser.$("[data-testid=run-alignment]")).click();
    await until("alignment to be fresh", async () => (await text("status-alignment")) === "fresh", 60000);
    await (await browser.$("[data-testid=water-level]")).setValue("38");
    await (await browser.$("[data-testid=set-water]")).click();
    await (await browser.$("[data-testid=run-volumes]")).click();
    await until("volumes to be fresh", async () => (await text("status-volumes")) === "fresh", 120000);
    await (await browser.$("[data-testid=sheets-toggle]")).click();
    // The view shows a sheet: the SVG as the drawing builder wrote it.
    await until("a sheet", async () => (await (await browser.$("[data-testid=sheet] svg")).isExisting()), 180000);
    const shown = await browser.execute(() => ({
      tabs: document.querySelectorAll("[data-testid=sheet-tab]").length,
      number: document.querySelector("[data-testid=sheet]").dataset.number,
      texts: document.querySelectorAll("[data-testid=sheet] svg text").length,
    }));
    assert.ok(shown.tabs >= 4, `sheets listed: ${shown.tabs}`);
    assert.match(shown.number, /^OWD-.+-(XS|TS|SO|PL)-\d{3}$/);
    assert.ok(shown.texts > 10, "the sheet draws its title block and notes");
    // Export: one PDF a sheet, named as owd draw names them.
    const dir = path.join(WORK, "sheets-pdf");
    answer("sheets.txt", dir);
    await (await browser.$("[data-testid=export-sheets]")).click();
    await until("the PDFs to be written", async () => fs.existsSync(dir) && fs.readdirSync(dir).filter((f) => f.endsWith(".pdf")).length === shown.tabs, 180000);
    for (const f of fs.readdirSync(dir).filter((f) => f.endsWith(".pdf"))) {
      assert.equal(fs.readFileSync(path.join(dir, f)).subarray(0, 5).toString(), "%PDF-", f);
    }
    // The Report button (spec 0014): the screening report as PDF, named as owd report names it.
    const reports = path.join(WORK, "reports");
    answer("report.txt", reports);
    await (await browser.$("[data-testid=report-toggle]")).click();
    await (await browser.$("[data-testid=export-report]")).click();
    await until("the report to be written", async () => fs.existsSync(reports) && fs.readdirSync(reports).some((f) => f.endsWith("-sr.pdf")), 180000);
    const report = fs.readdirSync(reports).find((f) => f.endsWith("-sr.pdf"));
    assert.match(report, /^owd-.+-sr\.pdf$/);
    assert.equal(fs.readFileSync(path.join(reports, report)).subarray(0, 5).toString(), "%PDF-");
    await (await browser.$("[data-testid=sheets-toggle]")).click();
    await until("the map to return", async () => !(await (await browser.$("[data-testid=sheets-view]")).isExisting()));
  });
  it("finds a river as you type, draws it from its mouth to its upstream end, and cuts a reach out of it", async () => {
    // The recorded answers stand in for Photon and Overpass.
    const reach = await serveReach();
    try {
      const at = path.join(WORK, "reach-endpoints.json");
      fs.writeFileSync(at, JSON.stringify(endpoints(reach.origin)));
      answer("reach-endpoints.json", at);
      answer("discard.txt", "yes");
      await openReachView();
      const before = searches.length;
      await (await browser.$("[data-testid=reach-river]")).setValue("Ank");
      const ankobra = '[data-testid="reach-hit"][data-osm="relation/3328713"]';
      await until("the list", async () => (await browser.$(ankobra)).isExisting(), 30000);
      assert.match(await (await browser.$(ankobra)).getText(), /Ankobra River — Western Region, Ghana/);
      assert.deepEqual(searches.slice(before), ["Ank"], "one request per pause, none under 3 letters");
      await (await browser.$(`${ankobra} button`)).click();
      await until("the whole river", async () => /km/.test((await text("reach-length").catch(() => "")) ?? ""), 60000);
      // Saved at once in its own folder, named for the minute it started and its river, with no
      // dialog (#322).
      const studies = () => (fs.existsSync(PROJECTS) ? fs.readdirSync(PROJECTS).filter((d) => d.includes("Ankobra River")) : []);
      await until("the study's folder", async () => studies().length === 1);
      assert.match(studies()[0], /^\d{4}-\d{2}-\d{2} \d{4} Ankobra River$/);
      const first = studies()[0];
      assert.ok(fs.existsSync(path.join(PROJECTS, studies()[0], `${studies()[0]}.owd`)));
      assert.doesNotMatch(await fileLabel(), /\(unsaved\)/);
      assert.equal(await (await browser.$("[data-testid=reach-start]")).getValue(), "-2.26885, 4.89839");
      assert.equal(await (await browser.$("[data-testid=reach-end]")).getValue(), "-2.13492, 5.72949");
      const whole = Number((await text("reach-length")).match(/([\d.]+) km/)[1]);
      const asked = searches.length;
      await (await browser.$("[data-testid=reach-end]")).setValue("-2.1533567, 5.3025015");
      await (await browser.$("[data-testid=reach-route]")).click();
      await until("the shorter reach", async () => Number((await text("reach-length")).match(/([\d.]+) km/)[1]) < whole);
      assert.equal(searches.length, asked);
      // The study held only its reach: the new cut keeps its folder, and no other is made.
      assert.deepEqual(studies(), [first]);
      assert.ok(fs.existsSync(path.join(PROJECTS, first, `${first}.owd`)));
      // A reach inside one DEM tile: Fetch DEM saves into the study's folder and never asks first.
      const lower = await text("reach-length");
      await (await browser.$("[data-testid=reach-start]")).setValue("-2.1533567, 5.3025015");
      await (await browser.$("[data-testid=reach-end]")).setValue("-2.1349229, 5.7294889");
      await (await browser.$("[data-testid=reach-route]")).click();
      await until("the upper reach", async () => (await text("reach-length").catch(() => lower)) !== lower, 30000);
      assert.deepEqual(studies(), [first]);
      answer("dem-mirror.txt", path.resolve(here, "../../crates/owd-cli/tests/fixtures/mirror"));
      await until("the route's status gone", async () => !(await (await browser.$("[data-testid=reach-status]")).isExisting()), 30000);
      await (await browser.$("[data-testid=reach-fetch-dem]")).click();
      const study = () => path.join(PROJECTS, studies()[0], `${studies()[0]}.owd`);
      await until("the DEM in the study", async () => fs.existsSync(study()) && fs.readFileSync(study(), "utf8").includes("cop-dem-glo30"), 60000);
      assert.ok(fs.readdirSync(path.dirname(study())).length > 1, "the DEM's files are in the study's folder");
      // Typing again lets go of the river: the next route is planned from clicks, as before. The
      // desktop driver refuses empty text, so the box is cleared with an input event.
      await browser.execute((el) => {
        el.value = "";
        el.dispatchEvent(new Event("input", { bubbles: true }));
      }, await browser.$("[data-testid=reach-river]"));
    } finally {
      reach.server.close();
    }
  });
  it("shows what a long step is doing, with a bar, the time and Cancel, which leaves the study as it was", async () => {
    const reach = await serveReach();
    try {
      const at = path.join(WORK, "reach-endpoints.json");
      fs.writeFileSync(at, JSON.stringify(endpoints(reach.origin)));
      answer("reach-endpoints.json", at);
      answer("discard.txt", "yes");
      await openReachView();
      await (await browser.$("[data-testid=reach-start]")).setValue("-6.0715, 4.966");
      await (await browser.$("[data-testid=reach-end]")).setValue("-6.07, 4.983");
      slow(300);
      await (await browser.$("[data-testid=reach-route]")).click();
      await until("the route's status", async () => (await browser.$("[data-testid=reach-status]")).isExisting(), 30000);
      assert.match(await text("reach-progress"), /Finding the route/);
      await until("the route", async () => /km/.test((await text("reach-length").catch(() => "")) ?? ""), 60000);
      // The buttons are enabled again once the status is gone.
      await until("the route's status gone", async () => !(await (await browser.$("[data-testid=reach-status]")).isExisting()), 30000);
      const tile = path.join(WORK, "occurrence_10W_10Nv1_4_2021.tif");
      fs.copyFileSync(path.resolve(here, "../../examples/reach/jrc_occurrence_sassandra_estuary.tif"), tile);
      answer("jrc.txt", tile);
      await (await browser.$("[data-testid=reach-add-tile]")).click();
      await until("the tile given", async () => (await (await browser.$("[data-testid='reach-jrc-occurrence_10W_10Nv1_4_2021.tif']")).getAttribute("data-given")) === "true");
      const mark = async () => (await browser.$("[data-testid=mark-left_bank]")).getAttribute("data-mark");
      const before = await mark();
      await (await browser.$("[data-testid=reach-generate]")).click();
      await until("a request named in the status line", async () => /\(request \d+\)/.test((await text("reach-now").catch(() => "")) ?? ""), 30000);
      assert.match(await text("reach-progress"), /Step 1 of \d+: /);
      assert.match(await text("reach-elapsed"), /^\d+:\d\d$/);
      await (await browser.$("[data-testid=reach-cancel]")).click();
      await until("the status gone", async () => !(await (await browser.$("[data-testid=reach-status]")).isExisting()), 30000);
      assert.match(await text("error"), /cancelled/i);
      assert.equal(await mark(), before, "Cancel leaves the study as it was");
    } finally {
      slow(0);
      reach.server.close();
    }
  });
  it("starts a study from a river: route, inputs with badges, a depth, the class table, and Use this vessel", async () => {
    // The recorded answers stand in for Overpass, Earth Search, Sentinel-2, Zenodo and Hydrocron.
    const reach = await serveReach();
    try {
      const at = path.join(WORK, "reach-endpoints.json");
      fs.writeFileSync(at, JSON.stringify(endpoints(reach.origin)));
      answer("reach-endpoints.json", at);
      answer("discard.txt", "yes");
      await openReachView();
      await (await browser.$("[data-testid=reach-start]")).setValue("-6.0715, 4.966");
      await (await browser.$("[data-testid=reach-end]")).setValue("-6.07, 4.983");
      await (await browser.$("[data-testid=reach-route]")).click();
      await until("the route", async () => /km/.test((await text("reach-length").catch(() => "")) ?? ""), 60000);
      const km = await text("reach-length");
      assert.equal(await (await browser.$("[data-testid=mark-route]")).getAttribute("data-mark"), "default");
      // The JRC tile is not given: Make the inputs downloads it into the study's folder (#321).
      answer("jrc-mirror.txt", `${reach.origin}/jrc`);
      assert.equal(await (await browser.$("[data-testid='reach-jrc-occurrence_10W_10Nv1_4_2021.tif']")).getAttribute("data-given"), "false");
      answer("load.txt", path.resolve(here, "../../examples/reach/glo30_sassandra_lower.tif"));
      await (await browser.$("[data-testid=reach-load-dem]")).click();
      await until("the DEM", async () => (await (await browser.$("[data-testid=mark-dem]")).getAttribute("data-mark")) === "user");
      await (await browser.$("[data-testid=reach-generate]")).click();
      await until("the banks moved onto Sentinel-2", async () => (await text("reach-input-left_bank")).includes("sentinel2-l2a"), 120000);
      await until("the water level", async () => (await (await browser.$("[data-testid=mark-water_level]")).getAttribute("data-mark")) === "default", 120000);
      const tileRow = await browser.$("[data-testid='reach-jrc-occurrence_10W_10Nv1_4_2021.tif']");
      assert.equal(await tileRow.getAttribute("data-given"), "true");
      assert.match(await tileRow.getText(), /in the study's folder/);
      const studyDir = fs.readdirSync(PROJECTS).find((d) => d.includes("Reach study") && fs.existsSync(path.join(PROJECTS, d, "jrc")));
      assert.ok(studyDir, "the study has its folder");
      assert.ok(fs.existsSync(path.join(PROJECTS, studyDir, "jrc", "occurrence_10W_10Nv1_4_2021.tif")), "the tile is in the study's folder");
      assert.match(fs.readFileSync(path.join(PROJECTS, studyDir, "jrc", "NOTICE.txt"), "utf8"), /Pekel et al\. 2016/);
      for (const input of ["left_bank", "right_bank", "river_polygon", "centreline"]) {
        const badge = await browser.$(`[data-testid=mark-${input}]`);
        assert.equal(await badge.getAttribute("data-mark"), "default", input);
        assert.match(await badge.getText(), /default/, `${input}: a word, not colour alone`);
      }
      assert.match(await text("map-notices"), /OpenStreetMap/);
      // The QA overlay (spec 0013): the banks over the scene, a PNG and a KMZ in the folder chosen.
      const qa = path.join(WORK, "qa-overlay");
      answer("qa.txt", qa);
      await (await browser.$("[data-testid=reach-qa-overlay]")).click();
      await until("the QA overlay", async () => (await browser.$("[data-testid=reach-qa-saved]")).isExisting(), 120000);
      const saved = fs.readdirSync(qa).sort();
      assert.equal(saved.length, 2, saved.join(", "));
      assert.ok(saved[0].endsWith("QA overlay.kmz") && saved[1].endsWith("QA overlay.png"), saved.join(", "));
      assert.equal(fs.readFileSync(path.join(qa, saved[0])).subarray(0, 2).toString(), "PK");
      assert.equal(fs.readFileSync(path.join(qa, saved[1])).subarray(1, 4).toString(), "PNG");
      await (await browser.$("[data-testid=bed-depth]")).setValue("3");
      await (await browser.$("[data-testid=bed-set]")).click();
      await until("the depth", async () => (await (await browser.$("[data-testid=mark-bed]")).getAttribute("data-mark")) === "user");
      await (await browser.$("[data-testid=fits-run]")).click();
      await until("the class table", async () => (await browser.$$("[data-testid^=fits-row-]")).length > 3, 60000);
      const row = (await browser.$$("[data-testid^=fits-row-]"))[0];
      const vessel = (await row.getAttribute("data-testid")).replace("fits-row-", "");
      await (await browser.$(`[data-testid=use-vessel-${vessel}]`)).click();
      await until("the vessel set", async () => (await vesselChoices()).current === vessel);
      assert.match(await text("notice"), /Vessel set/);

      // Save, then open the study again: the panel keeps its route, inputs and steps (#318).
      const study = path.join(WORK, "reach-study.owd");
      answer("save.txt", study);
      await click("Save as");
      await until("the study saved", async () => fs.existsSync(study) && !(await fileLabel()).includes("(unsaved)"));
      await openReachView();
      assert.equal(await text("reach-length"), km, "the route is still drawn after Save");
      assert.ok(await (await browser.$("[data-testid=reach-load-dem]")).isExisting(), "step 3 is still shown");
      await open(study, "reach-study.owd");
      await until("the route drawn again", async () => (await text("reach-length").catch(() => "")) === km, 60000);
      assert.match(await text("reach-input-left_bank"), /sentinel2-l2a/);
      assert.ok(await (await browser.$("[data-testid=reach-load-dem]")).isExisting(), "step 3 is shown after Open");
    } finally {
      reach.server.close();
    }
  });
});
