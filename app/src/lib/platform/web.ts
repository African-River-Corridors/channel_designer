// The browser adapter: the owd-web WebAssembly session, in a Web Worker on this page. No server sees a
// design or a DEM. Where the desktop opens a dialog or writes a file, the browser picks a file or
// downloads one. A run that is cancelled ends the worker, and a new worker starts from the design as
// it was before the run. Errors thrown by the session are strings; they become rejected promises.
import type { FileDropEvent, Platform } from "./types";

const LAST_DESIGN = "owd.lastDesign";
const DEFAULT_NAME = "waterway-design.owd";
const EXAMPLE = "river-example.owd";
const EXAMPLE_DEM = "river-dem.tif";
const DISCARD = "The channel design has unsaved changes. Discard them?";

/** Storage may be missing or throw (private window, blocked site data): every use is guarded. */
function readKey(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}
function writeKey(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    /* not remembered */
  }
}

/** The worker, and the requests waiting on it. */
class Host {
  private worker: Worker;
  private next = 1;
  private waiting = new Map<number, { ok: (v: unknown) => void; fail: (e: unknown) => void }>();
  constructor(private onProgress: (n: number) => void) {
    this.worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    this.worker.onmessage = (e: MessageEvent) => {
      if ("progress" in e.data) return this.onProgress(e.data.progress as number);
      const w = this.waiting.get(e.data.id);
      this.waiting.delete(e.data.id);
      if (!w) return;
      if (e.data.ok) w.ok(e.data.result);
      else w.fail(e.data.error);
    };
    this.worker.onerror = (e) => this.failAll(e.message || "the session stopped unexpectedly");
  }
  rpc<T>(method: string, ...args: unknown[]): Promise<T> {
    return new Promise<T>((ok, fail) => {
      const id = this.next++;
      this.waiting.set(id, { ok: ok as (v: unknown) => void, fail });
      this.worker.postMessage({ id, method, args });
    });
  }
  private failAll(why: string) {
    for (const w of this.waiting.values()) w.fail(why);
    this.waiting.clear();
  }
  /** End the worker. Every waiting request fails with `why`. */
  end(why: string) {
    this.worker.terminate();
    this.failAll(why);
  }
}

const CANCELLED = "cancelled";

function download(name: string, data: BlobPart, type: string) {
  const url = URL.createObjectURL(new Blob([data], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.style.display = "none";
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}

/** Ask for a file with the browser's picker. null when the user cancels. */
function pickFile(accept: string): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.dataset.testid = "file-picker";
    // Present but out of sight, so a test can fill it in.
    input.style.cssText = "position:fixed;left:0;top:0;width:1px;height:1px;opacity:0;";
    const done = (file: File | null) => {
      input.remove();
      resolve(file);
    };
    input.addEventListener("change", () => done(input.files?.[0] ?? null));
    input.addEventListener("cancel", () => done(null));
    document.body.append(input);
    input.click();
  });
}

const bytesOf = async (file: File) => new Uint8Array(await file.arrayBuffer());

export async function createWebPlatform(): Promise<Platform> {
  const progressHandlers = new Set<(n: number) => void>();
  const onProgress = (n: number) => progressHandlers.forEach((h) => h(n));
  let host = new Host(onProgress);
  await host.rpc("ping");

  /** The DEM the session holds, kept so a restart can give it back. */
  let dem: Uint8Array | null = null;
  /** Whether the design has unsaved changes, as the last snapshot said (or as it was before a restart). */
  let dirty = false;
  let carriedDirty = false;
  let running: { stage: string; cancelled: boolean } | null = null;

  const json = async <T>(method: string, ...args: unknown[]): Promise<T> => JSON.parse(await host.rpc<string>(method, ...args)) as T;

  /** A snapshot: note its title and unsaved flag, and remember the design for a reload. */
  function follow<T extends { dirty: boolean; title: string; file_name: string }>(snap: T): T {
    if (carriedDirty) snap.dirty = true;
    dirty = snap.dirty;
    try {
      document.title = snap.title;
    } catch {
      /* no document title to set */
    }
    host
      .rpc<string>("save_text", snap.file_name)
      .then((text) => writeKey(LAST_DESIGN, JSON.stringify({ name: snap.file_name, text })))
      .catch(() => {});
    return snap;
  }
  const change = async <T extends { dirty: boolean; title: string; file_name: string }>(method: string, ...args: unknown[]) => follow(await json<T>(method, ...args));

  const fileName = (snap: { file_name: string }) => (/\.owd$/.test(snap.file_name) ? snap.file_name : DEFAULT_NAME);
  const confirmDiscard = () => !dirty || window.confirm(DISCARD);

  async function saveNamed(name: string) {
    const text = await host.rpc<string>("save_text", name);
    download(name, text, "application/json");
    carriedDirty = false;
    return change("saved", name);
  }

  /** Bring back the design as it stood before a run, in a new worker. */
  async function restart(before: { name: string; text: string; dem: Uint8Array | null; dirty: boolean }) {
    host = new Host(onProgress);
    await host.rpc("ping");
    dem = before.dem;
    await host.rpc("open_text", before.name, before.text, before.dem ?? undefined);
    carriedDirty = before.dirty;
  }

  async function runStage(stage: string, method: string | null) {
    const snap = await json<{ file_name: string; dirty: boolean }>("snapshot");
    const before = { name: snap.file_name, text: await host.rpc<string>("save_text", snap.file_name), dem, dirty: snap.dirty || carriedDirty };
    running = { stage, cancelled: false };
    const mine = running;
    try {
      const result = await json<{ dirty: boolean; title: string; file_name: string }>("run_stage", stage, method ?? undefined);
      carriedDirty = false;
      return follow(result);
    } catch (e) {
      // Cancel ended the worker; so can a stage that stopped unexpectedly. Either way the design is as it was.
      let alive = true;
      if (!mine.cancelled) await host.rpc("ping").catch(() => (alive = false));
      if (mine.cancelled || !alive) {
        await restart(before);
        const back = await change<{ dirty: boolean; title: string; file_name: string }>("snapshot");
        if (mine.cancelled) return { ...back, run: { stage, outcome: "cancelled" } };
        throw "the stage stopped unexpectedly: the design is as it was";
      }
      throw e;
    } finally {
      if (running === mine) running = null;
    }
  }

  /** Where the reach sources are: empty is the public services; a test points them at recordings.
   * `?photon=URL` and `?nominatim=URL` on the page name another river search server, and
   * `?overpass=URL,URL` the Overpass servers, asked in that order (#319). */
  let reachAt = (() => {
    try {
      const q = new URLSearchParams(window.location.search);
      const at: Record<string, string> = {};
      for (const k of ["photon", "nominatim", "overpass"]) if (q.get(k)) at[k] = q.get(k)!;
      return Object.keys(at).length ? JSON.stringify(at) : "";
    } catch {
      return "";
    }
  })();

  /** One reach step in the worker. Cancel ends the worker, as for a stage, and the design comes back as it was. */
  async function reachStep(step: string, percentile: number | null) {
    const snap = await json<{ file_name: string; dirty: boolean }>("snapshot");
    const before = { name: snap.file_name, text: await host.rpc<string>("save_text", snap.file_name), dem, dirty: snap.dirty || carriedDirty };
    running = { stage: step, cancelled: false };
    const mine = running;
    try {
      const result = await change<{ dirty: boolean; title: string; file_name: string }>("reach_step", step, percentile ?? undefined, reachAt);
      carriedDirty = false;
      return result;
    } catch (e) {
      if (mine.cancelled) {
        await restart(before);
        throw "cancelled: the design is as it was before the step (pick the reach again to make more inputs)";
      }
      throw e;
    } finally {
      if (running === mine) running = null;
    }
  }

  async function loadInput(role: string, given: unknown) {
    const file = given instanceof File ? given : await pickFile(role === "dem" ? ".tif,.tiff" : ".geojson,.json,.gpkg");
    if (!file) return null;
    const bytes = await bytesOf(file);
    if (role === "dem") {
      const snap = await change("load_dem", file.name, bytes);
      dem = bytes;
      return snap;
    }
    return change("load_input", role, file.name, bytes, undefined);
  }

  async function openExample() {
    if (!confirmDiscard()) return null;
    const base = import.meta.env.BASE_URL;
    const [text, tif] = await Promise.all([fetch(`${base}examples/${EXAMPLE}`).then((r) => (r.ok ? r.text() : Promise.reject(`cannot load the example (${r.status})`))), fetch(`${base}examples/${EXAMPLE_DEM}`).then((r) => (r.ok ? r.arrayBuffer() : Promise.reject(`cannot load the example DEM (${r.status})`)))]);
    dem = new Uint8Array(tif);
    carriedDirty = false;
    return change("open_text", EXAMPLE, text, dem);
  }

  const commands: Record<string, (args: Record<string, unknown>) => Promise<unknown>> = {
    snapshot: () => change("snapshot"),
    sizing_choices: () => json("sizing_choices"),
    new_file: async () => {
      if (!confirmDiscard()) return null;
      dem = null;
      carriedDirty = false;
      return change("new_design");
    },
    open_file: async () => {
      if (!confirmDiscard()) return null;
      const file = await pickFile(".owd");
      if (!file) return null;
      const text = await file.text();
      dem = null;
      carriedDirty = false;
      return change("open_text", file.name, text, undefined);
    },
    save: async () => saveNamed(fileName(await json("snapshot"))),
    save_as: async () => {
      const name = window.prompt("File name", fileName(await json("snapshot")));
      if (name === null || name.trim() === "") return null;
      const clean = name.trim();
      return saveNamed(/\.owd$/.test(clean) ? clean : `${clean}.owd`);
    },
    set_rule_sets: (a) => change("set_rule_sets", JSON.stringify(a.ids), JSON.stringify(a.inputs)),
    set_vessel: (a) => change("set_vessel", a.vesselStandardId, a.draught),
    stretches_geojson: () => json("stretches_geojson"),
    run_stale: () => change("run_stale"),
    map_geometry: () => json("map_geometry"),
    option_table: () => json("option_table"),
    hillshade: () => json("hillshade"),
    long_section: (a) => json("long_section", a.option),
    sheets: (a) => json("sheets", a.option),
    export_sheets: async (a) => {
      // each sheet's PDF, as `owd draw` names it; the browser saves them as downloads
      const pdfs = await host.rpc<[string, Uint8Array][]>("sheet_pdfs", a.option);
      for (const [name, bytes] of pdfs) download(name, bytes as BlobPart, "application/pdf");
      return pdfs.length ? `${pdfs.length} PDF${pdfs.length === 1 ? "" : "s"}` : null;
    },
    export_report: async (a) => {
      // each report file, as `owd report` names it; the browser saves them as downloads
      const files = await host.rpc<[string, Uint8Array][]>("report_files", a.option ?? undefined, JSON.stringify(a.ask));
      const types: Record<string, string> = {
        pdf: "application/pdf",
        html: "text/html",
        docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      };
      for (const [name, bytes] of files) download(name, bytes as BlobPart, types[name.split(".").pop() ?? ""] ?? "application/octet-stream");
      return files.length ? files.map(([name]) => name).join(", ") : null;
    },
    export_qa_overlay: async (a) => {
      // the QA overlay's PNG and KMZ (spec 0013), drawn in the worker from the scenes; saved as downloads
      const files = await host.rpc<[string, Uint8Array][]>("qa_overlay", a.fromKm ?? undefined, a.toKm ?? undefined, reachAt);
      for (const [name, bytes] of files) download(name, bytes as BlobPart, name.endsWith(".kmz") ? "application/vnd.google-earth.kmz" : "image/png");
      return files.length ? files.map(([name]) => name).join(", ") : null;
    },
    cross_section: (a) => json("cross_section", a.option, a.chainage),
    station_near: (a) => host.rpc("station_near", a.option, a.lon, a.lat),
    load_input: (a) => loadInput(a.role as string, a.path),
    set_water: (a) => change("set_water", a.level, a.fall),
    edit_bank: (a) => change("edit_bank", a.side, JSON.stringify(a.edit)),
    undo_bank_edit: () => change("undo_bank_edit"),
    run_stage: (a) => runStage(a.stage as string, (a.method as string | null) ?? null),
    cancel_run: async () => {
      if (!running) return false;
      running.cancelled = true;
      host.end(CANCELLED);
      return true;
    },
    fetch_dem: () => Promise.reject("Fetching a DEM is not offered in the browser: load a GeoTIFF instead."),
    basemap_tiles: async () => null,
    reach_route: async (a) => {
      if (!confirmDiscard()) return null;
      carriedDirty = false;
      return change("reach_route", a.river ?? "", JSON.stringify({ start: a.start, end: a.end, via: a.via ?? [] }), reachAt);
    },
    reach_search: (a) => json("reach_search", a.text ?? "", a.country ?? "", a.once === true, reachAt),
    reach_river: async (a) => {
      if (!confirmDiscard()) return null;
      carriedDirty = false;
      return change("reach_river", a.osmId, JSON.stringify(a.also ?? []), (a.search as string) ?? "", reachAt);
    },
    reach_cut: async (a) => {
      if (!(await host.rpc<boolean>("reach_route_only")) && !confirmDiscard()) return null;
      carriedDirty = false;
      return change("reach_cut", JSON.stringify({ start: a.start, end: a.end, via: a.via ?? [] }));
    },
    reach_restore: () => change("reach_restore", reachAt),
    reach_add_tile: async (a) => {
      const file = a.file instanceof File ? a.file : await pickFile(".tif,.tiff");
      if (!file) return null;
      return change("reach_add_tile", file.name, await bytesOf(file));
    },
    reach_step: (a) => reachStep(a.step as string, (a.percentile as number | null) ?? null),
    reach_cancel: async () => {
      if (!running) return;
      running.cancelled = true;
      host.end(CANCELLED);
    },
    set_bed: (a) => change("set_bed", JSON.stringify(a.points), (a.season as string) ?? ""),
    run_fits: () => change("run_fits"),
    open_example: openExample,
    export_file: async (a) => {
      const kind = a.kind as string;
      if (kind !== "geojson" && kind !== "csv") throw `\`${kind}\` is not an export: use geojson or csv`;
      const snap = await json<{ file_name: string }>("snapshot");
      const base = fileName(snap).replace(/\.owd$/, "");
      const text = await host.rpc<string>(kind === "geojson" ? "export_geojson" : "export_csv");
      const name = `${base}.${kind}`;
      download(name, text, kind === "geojson" ? "application/geo+json" : "text/csv");
      return name;
    },
  };

  // Bring back the last design, if there is one. Any failure leaves the empty design.
  try {
    const raw = readKey(LAST_DESIGN);
    if (raw) {
      const last = JSON.parse(raw) as { name: string; text: string };
      await host.rpc("open_text", last.name, last.text, undefined);
    }
  } catch {
    /* ignore: the empty design stays */
  }

  window.addEventListener("beforeunload", (e) => {
    if (dirty) {
      e.preventDefault();
      e.returnValue = "";
    }
  });

  const platform: Platform = {
    kind: "web",
    call: (command, args) => {
      const run = commands[command];
      return (run ? run(args ?? {}) : Promise.reject(`\`${command}\` is not available in the browser`)) as Promise<never>;
    },
    onProgress: async (handler) => {
      progressHandlers.add(handler);
      return () => progressHandlers.delete(handler);
    },
    onFileDrop: async (handler: (e: FileDropEvent) => void) => {
      const over = (e: DragEvent) => {
        if (!e.dataTransfer?.types.includes("Files")) return;
        e.preventDefault();
        handler({ type: "over", x: e.clientX, y: e.clientY });
      };
      const leave = (e: DragEvent) => {
        if (e.relatedTarget === null) handler({ type: "leave" });
      };
      const drop = (e: DragEvent) => {
        const file = e.dataTransfer?.files[0];
        if (!file) return;
        e.preventDefault();
        handler({ type: "drop", x: e.clientX, y: e.clientY, name: file.name, file });
      };
      window.addEventListener("dragover", over);
      window.addEventListener("dragleave", leave);
      window.addEventListener("drop", drop);
      return () => {
        window.removeEventListener("dragover", over);
        window.removeEventListener("dragleave", leave);
        window.removeEventListener("drop", drop);
      };
    },
  };
  // The smoke test reads results through this, and points the reach sources at recorded answers;
  // only a build in test mode has it.
  if (import.meta.env.MODE === "test") {
    commands.set_reach_endpoints = async (a) => {
      reachAt = JSON.stringify(a.endpoints ?? {});
    };
    (window as unknown as { __owdPlatform: Platform }).__owdPlatform = platform;
  }
  return platform;
}
