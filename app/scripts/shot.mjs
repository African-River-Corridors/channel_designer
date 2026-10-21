// Screenshot the mock session (src/mock.ts) in headless Chrome over the DevTools protocol, light and dark.
// Needs `npm run dev` running on port 1420 and Node 22+. Usage: node scripts/shot.mjs <name> [outdir]
// OS_DARK=1 shows the app on the OS dark setting with no pinned theme (writes <name>-os-dark.png).
// HEIGHT=1100 makes the window taller; STATE=empty shows a design with nothing loaded. Writes <outdir>/<name>-light.png and <name>-dark.png (default /tmp/owd-shots).
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";

const name = process.argv[2];
if (!name) throw new Error("usage: shot.mjs <name> [outdir]");
const out = process.argv[3] ?? "/tmp/owd-shots";
const height = Number(process.env.HEIGHT ?? 900);
const chrome = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const port = 9333 + Math.floor(Math.random() * 500);
mkdirSync(out, { recursive: true });

const proc = spawn(chrome, [
  "--headless=new", `--remote-debugging-port=${port}`, "--use-gl=angle", "--use-angle=swiftshader",
  "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--hide-scrollbars", "--user-data-dir=" + `/tmp/owd-shot-${port}`, "about:blank",
], { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
try {
  let target;
  for (let i = 0; i < 50 && !target; i++) {
    await sleep(200);
    try { target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === "page"); } catch {}
  }
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r) => (ws.onopen = r));
  let id = 0;
  const pending = new Map();
  ws.onmessage = (m) => { const d = JSON.parse(m.data); pending.get(d.id)?.(d.result); };
  const send = (method, params = {}) => new Promise((r) => { pending.set(++id, r); ws.send(JSON.stringify({ id, method, params })); });
  await send("Emulation.setDeviceMetricsOverride", { width: 1440, height, deviceScaleFactor: 1, mobile: false });
  if (process.env.OS_DARK) await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
  for (const theme of process.env.OS_DARK ? ["auto"] : ["light", "dark"]) {
    await send("Page.navigate", { url: `http://localhost:1420/mock.html?theme=${theme}${process.env.STATE ? `&state=${process.env.STATE}` : ""}` });
    await sleep(9000);
    const shot = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(`${out}/${name}-${theme === "auto" ? "os-dark" : theme}.png`, Buffer.from(shot.data, "base64"));
  }
  ws.close();
  console.log(`${out}/${name}-light.png ${out}/${name}-dark.png`);
} finally {
  proc.kill();
}
