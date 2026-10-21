// A tiny static server that behaves like the live site: the page is at /owd/app (no slash) and its
// files are under /owd/app-files/. Serves .wasm as application/wasm.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".woff2": "font/woff2",
  ".svg": "image/svg+xml",
  ".owd": "application/json",
  ".tif": "image/tiff",
};

export function serve(dir, port = 0) {
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://localhost").pathname;
    let file = null;
    if (url === "/owd/app") file = path.join(dir, "index.html");
    else if (url.startsWith("/owd/app-files/")) {
      const rel = path.normalize(url.slice("/owd/app-files/".length));
      if (!rel.startsWith("..")) file = path.join(dir, rel);
    }
    if (!file || !fs.existsSync(file) || !fs.statSync(file).isFile()) {
      res.writeHead(404).end("not found");
      return;
    }
    res.writeHead(200, { "content-type": TYPES[path.extname(file)] ?? "application/octet-stream" });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise((resolve) =>
    server.listen(port, "127.0.0.1", () => resolve({ server, origin: `http://127.0.0.1:${server.address().port}` })),
  );
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { origin } = await serve(path.resolve(process.argv[2] ?? "dist-web"), Number(process.argv[3] ?? 4173));
  console.log(`serving at ${origin}/owd/app`);
}
