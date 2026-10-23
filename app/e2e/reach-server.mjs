// Recorded answers for "Start from a river" (spec 0012, T10), so no e2e test touches the network: the
// Overpass network of a river arm of the lower Sassandra (synthetic), the Earth Search answer, each scene by
// its id, and the Sentinel-2 band windows of 23 December 2023 (blue B02 too, for the QA overlay), the SWORD subset on a Zenodo-like record, and the
// Hydrocron series; and, for the river search (T13 to T15), Photon's answer for "Ank" and the Ankobra
// relation from Overpass, both recorded on 2026-10-22 (© OpenStreetMap contributors). Byte ranges are honoured and every answer allows CORS, as the real hosts do.
// `endpoints(origin)` gives what to point the app at.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const EX = path.resolve(import.meta.dirname, "../../examples/reach");
const S2 = "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/29/N/RF/2023/12/S2B_29NRF_20231223_0_L2A";
const read = (name) => fs.readFileSync(path.join(EX, name));
// The recorded scene is served from here; every other scene in the answer points here too and is not
// found, so no test reads a real scene (the seed of spec 0013 ranks every scene it is offered).
const stac = (origin) =>
  read("stac_search_sassandra.json").toString().replaceAll(S2, `${origin}/s2`).replaceAll("https://sentinel-cogs.s3.us-west-2.amazonaws.com/", `${origin}/cogs/`);

export function endpoints(origin) {
  return {
    overpass: `${origin}/overpass/api/interpreter`,
    photon: `${origin}/photon/api/`,
    nominatim: `${origin}/nominatim/search`,
    stac: `${origin}/stac/search`,
    sword_record: `${origin}/zenodo/record`,
    hydrocron: `${origin}/hydrocron/timeseries`,
  };
}

/** Every river search the server answered, by its text: a test checks how often the app asked. */
export const searches = [];

function answer(url, origin) {
  const p = url.pathname;
  if (p === "/photon/api/") {
    const q = url.searchParams.get("q") ?? "";
    searches.push(q);
    // "Ankobra" stands for Photon failing: a busy server.
    if (q === "Ankobra") return { status: 503, body: Buffer.from("busy"), type: "text/plain" };
    if (q.toLowerCase().startsWith("ank")) return { body: read("photon_ank.json"), type: "application/json" };
    return { body: Buffer.from(JSON.stringify({ type: "FeatureCollection", features: [] })), type: "application/json" };
  }
  // A busy Overpass server, for the mirror fallback (#319).
  if (p === "/down/api/interpreter") return { status: 502, body: Buffer.from("busy"), type: "text/plain" };
  // A JRC occurrence tile, by its own name, for the desktop's download into a study's folder (#321).
  if (p === "/jrc/occurrence_10W_10Nv1_4_2021.tif") return { body: read("jrc_occurrence_sassandra_estuary.tif"), type: "image/tiff" };
  if (p === "/nominatim/search") {
    searches.push(`nominatim:${url.searchParams.get("q") ?? ""}`);
    return { body: read("nominatim_ankobra.json"), type: "application/json" };
  }
  if (p === "/overpass/api/interpreter" && (url.searchParams.get("data") ?? "").includes("rel(3328713)")) return { body: read("overpass_ankobra.json"), type: "application/json" };
  if (p === "/overpass/api/interpreter") return { body: read("synthetic/sassandra_arm.json"), type: "application/json" };
  if (p === "/stac/search") return { body: Buffer.from(stac(origin)), type: "application/json" };
  // One scene by its id, as the QA overlay asks for it (spec 0013).
  const item = p.match(/^\/stac\/collections\/sentinel-2-l2a\/items\/([^/]+)$/);
  if (item) {
    const f = JSON.parse(stac(origin)).features.find((x) => x.id === item[1]);
    return f ? { body: Buffer.from(JSON.stringify(f)), type: "application/json" } : { status: 404, body: Buffer.from("not found"), type: "text/plain" };
  }
  const band = p.match(/^\/s2\/(B02|B03|B04|B08|B11|SCL)\.tif$/);
  if (band) return { body: read(`s2_sassandra_${band[1]}.tif`), type: "image/tiff" };
  if (p === "/zenodo/record") {
    const size = fs.statSync(path.join(EX, "sword_v17c_shp_subset.zip")).size;
    const record = { files: [{ key: "SWORD_v17c_shp.zip", size, links: { self: `${origin}/zenodo/sword.zip` } }] };
    return { body: Buffer.from(JSON.stringify(record)), type: "application/json" };
  }
  if (p === "/zenodo/sword.zip") return { body: read("sword_v17c_shp_subset.zip"), type: "application/zip" };
  if (p === "/hydrocron/timeseries") {
    const id = url.searchParams.get("feature_id") ?? "";
    const file = path.join(EX, `hydrocron_${id}.json`);
    if (fs.existsSync(file)) return { body: fs.readFileSync(file), type: "application/json" };
    return { status: 400, body: Buffer.from(JSON.stringify({ error: `400: Results with the specified Feature ID ${id} were not found` })), type: "application/json" };
  }
  return null;
}

/** Delay every answer by this many milliseconds, so a test can see a step's progress and cancel it. */
let delayMs = 0;
export const slow = (ms) => (delayMs = ms);

export function serveReach(port = 0) {
  let origin = "";
  const server = http.createServer(async (req, res) => {
    if (delayMs > 0) await new Promise((r) => setTimeout(r, delayMs));
    const cors = { "access-control-allow-origin": "*", "access-control-allow-headers": "range", "access-control-expose-headers": "content-range, content-length" };
    if (req.method === "OPTIONS") return res.writeHead(204, cors).end();
    const a = answer(new URL(req.url, origin), origin);
    if (!a) return res.writeHead(404, cors).end("not recorded");
    const range = (req.headers.range ?? "").match(/^bytes=(\d+)-(\d+)$/);
    if (range && (a.status ?? 200) === 200) {
      const [from, to] = [Number(range[1]), Math.min(Number(range[2]), a.body.length - 1)];
      return res.writeHead(206, { ...cors, "content-type": a.type, "content-range": `bytes ${from}-${to}/${a.body.length}` }).end(a.body.subarray(from, to + 1));
    }
    res.writeHead(a.status ?? 200, { ...cors, "content-type": a.type }).end(a.body);
  });
  return new Promise((resolve) =>
    server.listen(port, "127.0.0.1", () => {
      origin = `http://127.0.0.1:${server.address().port}`;
      resolve({ server, origin });
    }),
  );
}
