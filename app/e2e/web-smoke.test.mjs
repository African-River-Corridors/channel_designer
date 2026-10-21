// Web smoke test (spec 0011, T4): the built web app in headless Chrome, served like the live site. Run by
// e2e/run-web.sh. Checks: the page loads with its web label and no Fetch DEM; the bundled example opens,
// a changed input moves the result, and the result equals native `owd run` (ADR 0005 tolerance); a
// design and its DEM load through the file picker and a full run (the pool search, in wasm) equals native;
// Cancel leaves the design as it was; a reload brings the design back; blocked storage still loads.
import { after, before, describe, it } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { remote } from "webdriverio";
import { serve } from "./web-server.mjs";
import { compare } from "../../tools/web_parity_compare.mjs";

const here = import.meta.dirname;
const FIXTURES = path.resolve(here, "../../examples/web");
const DIST = path.resolve(here, "../dist-web-test");
const CLI = process.env.OWD_CLI;
assert.ok(CLI, "OWD_CLI must be set (run e2e/run-web.sh)");
const SHOTS = process.env.OWD_WEB_SHOTS;

/** The document `owd run` leaves for a fixture, after the same edit (if any) the app made. */
function nativeDoc(file, edit) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-smoke-"));
  const doc = path.join(dir, file);
  fs.copyFileSync(path.join(FIXTURES, file), doc);
  fs.copyFileSync(path.join(FIXTURES, "river-dem.tif"), path.join(dir, "river-dem.tif"));
  const owd = (args) => {
    try {
      execFileSync(CLI, args, { encoding: "utf8", maxBuffer: 1 << 28 });
    } catch (e) {
      if (e.status !== 1) throw e; // exit 1: a check failed; the document is still written
    }
  };
  if (edit) owd(["size", doc, "--ruleset", "PIANC-B", "--vessel", "CEMT_IV", "--draught", edit]);
  owd(["run", doc]);
  return JSON.parse(fs.readFileSync(doc, "utf8"));
}

const chrome = (extra = {}) => ({
  browserName: "chrome",
  "wdio:enforceWebDriverClassic": true,
  "goog:chromeOptions": {
    ...(process.env.CHROME_BINARY ? { binary: process.env.CHROME_BINARY } : fs.existsSync("/Applications/Google Chrome.app") ? { binary: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" } : {}),
    args: ["--headless=new", "--no-sandbox", "--disable-gpu", "--window-size=1500,950"],
    ...extra,
  },
});

let origin, server;
const sessions = [];
async function start(options = {}) {
  const b = await remote({ logLevel: "warn", strictSelectors: false, capabilities: chrome(options) });
  sessions.push(b);
  return b;
}
const until = (b, what, check, timeout = 60000) => b.waitUntil(check, { timeout, interval: 150, timeoutMsg: `timed out waiting for: ${what}` });
const tid = (b, id) => b.$(`[data-testid=${id}]`);
const text = async (b, id) => (await tid(b, id)).getText();
const exists = async (b, id) => (await tid(b, id)).isExisting();
const loaded = (b) => until(b, "the app", async () => (await exists(b, "file-label")) && (await exists(b, "web-label")));
/** The document the page holds now. Only a build in test mode has the hook. */
const pageDoc = (b) => b.execute(async () => (await window.__owdPlatform.call("snapshot")).document);
const stageStatus = (b) => b.execute(async () => (await window.__owdPlatform.call("snapshot")).stages.map((s) => `${s.name}:${s.status}`).join(" "));
/** The browser's file chooser would be cancelled in headless Chrome: stop the page opening it, fill the input in. */
async function pick(b, open, file) {
  await b.execute(() => {
    HTMLInputElement.prototype.click = function () {};
  });
  await open();
  const picker = await tid(b, "file-picker");
  await picker.waitForExist({ timeout: 5000 });
  await picker.setValue(file);
}
const openButton = (b) => b.$("button=Open");
const sameAsNative = (page, native, what) => {
  for (const key of ["stages", "options"]) {
    const r = compare(native[key], page[key]);
    assert.deepEqual(r.problems.slice(0, 5), [], `${what}: ${key} equal to native (ADR 0005 tolerance)`);
    assert.ok(r.numbers > 0 || key === "options", `${what}: ${key} had numbers to compare`);
  }
};

before(async () => {
  ({ server, origin } = await serve(DIST));
});
after(async () => {
  for (const b of sessions) await b.deleteSession().catch(() => {});
  server?.close();
});

describe("OWD web smoke tests", () => {
  it("loads with the web label and the desktop link, and has no Fetch DEM", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    assert.match((await text(b, "web-label")).toLowerCase(), /web version/);
    assert.equal(await (await b.$("a=Get the desktop app")).getAttribute("href"), "/owd/docs#install");
    assert.equal(await exists(b, "fetch-dem"), false, "Fetch DEM is not offered in the browser");
    assert.equal(await exists(b, "error"), false);
  });

  it("opens the example, a changed input moves the result, and the result equals owd run", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the option table", async () => (await b.$$("[data-testid=option-row]")).length === 1);
    assert.equal(await stageStatus(b), "sizing:fresh alignment:fresh optimise:not_run volumes:fresh");
    const dredge = async () => (await (await b.$("[data-testid=option-row]")).getText()).replace(/\s+/g, " ");
    const before = await dredge();
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await new Promise((r) => setTimeout(r, 1500));
      await b.saveScreenshot(path.join(SHOTS, "web-example.png"));
    }
    // Change one input: the draught.
    await (await b.$("input[aria-label=Draught]")).setValue("2.6");
    await until(b, "later stages to go stale", async () => (await stageStatus(b)).includes("volumes:stale"));
    await (await b.$("button=Run stale")).click();
    await until(b, "volumes fresh again", async () => (await stageStatus(b)) === "sizing:fresh alignment:fresh optimise:not_run volumes:fresh");
    const after = await dredge();
    assert.notEqual(after, before, "the result moved");
    sameAsNative(await pageDoc(b), nativeDoc("river-example.owd", "2.6"), "example at draught 2.6");
    if (SHOTS) await b.saveScreenshot(path.join(SHOTS, "web-example-changed.png"));
    // The design is kept: a reload brings it back (with its unsaved changes, as saved text).
    await b.url(`${origin}/owd/app`);
    await until(b, "the restored design", async () => (await exists(b, "file-label")) && (await text(b, "file-label")).includes("river-example.owd"));
  });

  it("edits a bank vertex: the mark turns edited, the stages go stale, a crossing is refused, and undo brings it back", async () => {
    // The map needs WebGL: headless Chrome gets it from SwiftShader, in software.
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    // The example has no bank lines: load two of our own (marked `user`), and rerun what that made stale.
    const bankFile = (side) => path.join(FIXTURES, `river-bank-${side}.geojson`);
    await pick(b, async () => (await tid(b, "load-left_bank")).click(), bankFile("left"));
    await until(b, "the left bank", async () => (await (await tid(b, "data-left_bank")).getText()).length > 0 && !(await (await tid(b, "data-left_bank")).getText()).includes("not loaded"));
    await pick(b, async () => (await tid(b, "load-right_bank")).click(), bankFile("right"));
    await until(b, "the editor to be ready", async () => (await tid(b, "bank-edit-toggle")).isEnabled());
    await (await b.$("button=Run stale")).click();
    const fresh = "sizing:fresh alignment:fresh optimise:not_run volumes:fresh";
    await until(b, "stages fresh", async () => (await stageStatus(b)) === fresh, 120000);
    await (await tid(b, "bank-edit-toggle")).click();
    assert.equal(await (await tid(b, "mark-left_bank")).getAttribute("data-mark"), "user", "a loaded line is marked user");
    await (await tid(b, "bank-tool-delete")).click();
    await until(b, "vertex handles", async () => (await b.$$("[data-testid=bank-vertex]")).length > 2);
    const handles = await b.$$("[data-testid=bank-vertex]");
    const open = [];
    for (const h of handles) if (await h.isClickable()) open.push(h);
    assert.ok(open.length > 2, "some vertex handles are clear of the panel");
    await open[Math.floor(open.length / 2)].click();
    await until(b, "the left bank to be marked edited", async () => (await (await tid(b, "mark-left_bank")).getAttribute("data-mark")) === "edited");
    assert.equal(await (await tid(b, "mark-river_polygon")).getAttribute("data-mark"), "edited");
    assert.equal(await (await tid(b, "mark-centreline")).getAttribute("data-mark"), "edited");
    assert.ok((await stageStatus(b)).includes("alignment:stale"), await stageStatus(b));
    // A move that crosses the other bank is refused with a message, and nothing changes.
    const before = await pageDoc(b);
    const refusal = await b.execute(async () => {
      const g = await window.__owdPlatform.call("map_geometry");
      const right = g.banks.find((x) => x.side === "right").line.coordinates;
      try {
        await window.__owdPlatform.call("edit_bank", { side: "left_bank", edit: { kind: "move", index: 1, to: right[Math.floor(right.length / 2)] } });
        return "accepted";
      } catch (e) {
        return String(e);
      }
    });
    assert.match(refusal, /cross/);
    assert.deepEqual((await pageDoc(b)).inputs, before.inputs, "a refused edit changes nothing");
    await (await tid(b, "bank-undo")).click();
    await until(b, "the stages to be fresh again", async () => (await stageStatus(b)) === fresh);
    assert.equal(await (await tid(b, "mark-left_bank")).getAttribute("data-mark"), "user", "undo brings the old record back");
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await b.saveScreenshot(path.join(SHOTS, "web-bank-editor.png"));
    }
  });

  it("opens a design and its DEM through the file picker, and a full run equals owd run", async () => {
    const FILE = "river-rolling-circle.owd";
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), path.join(FIXTURES, FILE));
    await until(b, "the file name", async () => (await text(b, "file-label")).includes(FILE));
    // The document names its DEM; the browser does not have it yet.
    await until(b, "the missing-DEM warning", async () => (await exists(b, "warning")) && (await text(b, "warning")).toLowerCase().includes("dem"));
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the DEM", async () => (await text(b, "data-dem")).includes("river-dem.tif"));
    await until(b, "the warning to go", async () => !(await exists(b, "warning")));
    assert.equal(await stageStatus(b), "sizing:fresh alignment:stale optimise:stale volumes:stale");
    await (await b.$("button=Run stale")).click();
    await until(b, "every stage fresh", async () => (await stageStatus(b)) === "sizing:fresh alignment:fresh optimise:fresh volumes:fresh", 240000);
    sameAsNative(await pageDoc(b), nativeDoc(FILE), FILE);
    assert.equal(await exists(b, "error"), false);
  });

  it("Cancel leaves the design exactly as it was", async () => {
    const FILE = "river-rolling-circle.owd";
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), path.join(FIXTURES, FILE));
    await until(b, "the file name", async () => (await text(b, "file-label")).includes(FILE));
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the DEM", async () => (await text(b, "data-dem")).includes("river-dem.tif"));
    const docBefore = await pageDoc(b);
    const statusBefore = await stageStatus(b);
    // Alignment is quick; the pool search after it is not. Start the whole chain, cancel in the search.
    await (await tid(b, "run-alignment")).click();
    await until(b, "alignment fresh", async () => (await stageStatus(b)).includes("alignment:fresh"));
    const docBeforeOptimise = await pageDoc(b);
    await (await tid(b, "run-optimise")).click();
    await (await tid(b, "cancel-optimise")).waitForExist({ timeout: 10000 });
    await (await tid(b, "cancel-optimise")).click();
    await until(b, "the cancelled note", async () => (await exists(b, "notice")) && (await text(b, "notice")).includes("cancelled"));
    // The worker was restarted from the design as it was before the optimise run (after alignment).
    const status = await stageStatus(b);
    assert.ok(status.includes("alignment:fresh") && status.includes("optimise:stale"), `the optimise run left nothing behind: ${status}`);
    assert.notEqual(status, statusBefore, "alignment did run");
    assert.deepEqual(await pageDoc(b), docBeforeOptimise, "the document is exactly as it was before the cancelled run");
    assert.equal(await exists(b, "error"), false);
    // And the app still works: an edit reaches the restarted worker and marks the stages stale.
    await (await b.$("input[aria-label=Draught]")).setValue("2.7");
    await until(b, "the edit to land", async () => (await stageStatus(b)).includes("alignment:stale"));
    assert.equal(await exists(b, "error"), false);
    assert.ok(docBefore.stages.optimise.results !== undefined, "the design was opened with every stage run");
  });

  it("still loads when browser storage is blocked", async () => {
    // Blocking cookies and site data makes `localStorage` throw.
    const b = await start({ prefs: { "profile.default_content_setting_values.cookies": 2 } });
    await b.url(`${origin}/owd/app`);
    const blocked = await b.execute(() => {
      try {
        window.localStorage.getItem("x");
        return false;
      } catch {
        return true;
      }
    });
    assert.ok(blocked, "storage is blocked in this browser");
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    assert.equal(await exists(b, "error"), false);
  });
});
