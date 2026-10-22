// Recorded answers for "Start from a river" (spec 0012, T10), so no e2e test touches the network: the
// Overpass network of a river arm of the lower Sassandra (synthetic), the Earth Search answer and the
// Sentinel-2 band windows of 23 December 2023, the SWORD subset on a Zenodo-like record, and the
// Hydrocron series; and, for the river search (T13 to T15), Photon's answer for "Ank" and the Ankobra
// relation from Overpass, both recorded on 2026-10-22 (© OpenStreetMap contributors). Byte ranges are honoured and every answer allows CORS, as the real hosts do.
// `endpoints(origin)` gives what to point the app at.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const EX = path.resolve(import.meta.dirname, "../../examples/reach");
const S2 = "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/29/N/RF/2023/12/S2B_29NRF_20231223_0_L2A";
const read = (name) => fs.readFileSync(path.join(EX, name));

export function endpoints(origin) {
  return {
    overpass: `${origin}/overpass/api/interpreter`,
    photon: `${origin}/photon/api/`,
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
    if (q.toLowerCase().startsWith("ank")) return { body: read("photon_ank.json"), type: "application/json" };
    return { body: Buffer.from(JSON.stringify({ type: "FeatureCollection", features: [] })), type: "application/json" };
  }
  if (p === "/overpass/api/interpreter" && (url.searchParams.get("data") ?? "").includes("rel(3328713)")) return { body: read("overpass_ankobra.json"), type: "application/json" };
  if (p === "/overpass/api/interpreter") return { body: read("synthetic/sassandra_arm.json"), type: "application/json" };
  if (p === "/stac/search") return { body: Buffer.from(read("stac_search_sassandra.json").toString().replaceAll(S2, `${origin}/s2`)), type: "application/json" };
  const band = p.match(/^\/s2\/(B03|B04|B08|B11|SCL)\.tif$/);
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

export function serveReach(port = 0) {
  let origin = "";
  const server = http.createServer((req, res) => {
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
