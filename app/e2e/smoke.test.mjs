// End-to-end smoke test (spec 0003, ticket 68): one journey on the built app, driven through Tauri's
// WebDriver (`tauri-driver`). Run by `e2e/run.sh`, which starts the driver and sets
// CD_TEST_DIALOG_DIR: a debug build answers its native dialogs from files in that folder, since
// WebDriver cannot click a native dialog (see `test_answer` in src-tauri/src/lib.rs).
import { after, before, beforeEach, describe, it } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
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
    for (const f of ["open.txt", "save.txt", "discard.txt"]) forget(f);
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
});
