// Contrast check for the OWD tokens (docs/design/style-guide.md, section 3).
// Reads the tokens from a CSS file (default src/tokens.css), checks every pair in the light and the dark
// theme (text 4.5:1, lines 3:1), and checks that the six stage colours stay apart from each other
// (delta E 20) and from the status colours (delta E 15). Prints a table, exits 1 on any failure.
// It also checks that the OS-dark block and the data-theme="dark" block hold the same values, and that
// website/tokens.css is the same file as src/tokens.css (one token file for the app and the website).
// Usage: node scripts/contrast.mjs [tokens.css] [--markdown]
import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const OPTS = [1, 2, 3, 4, 5, 6].map((i) => `opt-${i}`);

// [foreground, background, kind, themes]
export const PAIRS = [
  ...["ink", "ink-2", "ink-3"].flatMap((fg) =>
    ["paper", "surface", "surface-2", "accent-soft"].map((bg) => [fg, bg, "text"])),
  ["accent-ink", "accent", "text"],
  ["pass", "pass-soft", "text"], ["fail", "fail-soft", "text"], ["warn", "warn-soft", "text"],
  ["pass", "surface", "text"], ["fail", "surface", "text"], ["warn", "surface", "text"], ["stale", "surface", "text"],
  ["src-ruleset", "surface", "text"],
  ["sheet-ink", "sheet", "text"], ["sheet-ink-2", "sheet", "text"],
  ...OPTS.map((o) => ["plate-ink", `${o}-wash`, "text"]),
  ["rule-strong", "surface", "line"], ["rule-strong", "paper", "line"], ["ink", "paper", "line"],
  ...OPTS.flatMap((o) => [[o, "paper", "line"], [o, "surface", "line"]]),
  ...OPTS.map((o) => [o, "sheet", "line", "light"]),
];
const TARGET = { text: 4.5, line: 3.0 };

function declarations(body) {
  const out = {};
  for (const m of body.matchAll(/--([a-z0-9-]+)\s*:\s*([^;]+);/g)) out[m[1]] = m[2].trim();
  return out;
}

// Blocks whose selector matches `selector` exactly, merged in file order.
function blocks(css, selector) {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const out = {};
  const re = new RegExp("(^|[}\\s])" + selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "\\s*\\{([^}]*)\\}", "g");
  for (const m of stripped.matchAll(re)) Object.assign(out, declarations(m[2]));
  return out;
}

export function parseTokens(css) {
  return {
    light: blocks(css, ":root"),
    darkAttr: blocks(css, ':root[data-theme="dark"]'),
    darkMedia: blocks(css, ':root:not([data-theme="light"])'),
  };
}

function resolveValue(map, name, seen = new Set()) {
  const v = map[name];
  if (v === undefined || seen.has(name)) return undefined;
  seen.add(name);
  const ref = v.match(/^var\(--([a-z0-9-]+)\)$/);
  return ref ? resolveValue(map, ref[1], seen) : v;
}

function rgb(hex) {
  const h = hex.replace("#", "");
  if (!/^[0-9a-fA-F]{6}$/.test(h)) throw new Error(`not a #rrggbb colour: ${hex}`);
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
}
function linear(c) {
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}
function luminance(hex) {
  const [r, g, b] = rgb(hex).map(linear);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
export function ratio(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}
function lab(hex) {
  const [r, g, b] = rgb(hex).map(linear);
  const X = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
  const Y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  const Z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
  const f = (t) => (t > 0.008856 ? Math.cbrt(t) : 7.787 * t + 16 / 116);
  return [116 * f(Y) - 16, 500 * (f(X) - f(Y)), 200 * (f(Y) - f(Z))];
}
export function deltaE(a, b) {
  const [p, q] = [lab(a), lab(b)];
  return Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
}

export function check(css) {
  const { light, darkAttr, darkMedia } = parseTokens(css);
  const themes = { light, dark: { ...light, ...darkAttr } };
  const rows = [];
  const problems = [];
  for (const [fg, bg, kind, only] of PAIRS) {
    const row = { fg, bg, kind };
    for (const [name, map] of Object.entries(themes)) {
      if (only && only !== name) continue;
      const a = resolveValue(map, fg), b = resolveValue(map, bg);
      if (!a || !b) { problems.push(`${name}: missing --${a ? bg : fg}`); continue; }
      const r = ratio(a, b);
      row[name] = r;
      if (r < TARGET[kind]) problems.push(`${name}: ${fg} on ${bg} is ${r.toFixed(2)}, needs ${TARGET[kind]}`);
    }
    rows.push(row);
  }
  for (const [name, map] of Object.entries(themes)) {
    OPTS.forEach((a, i) => {
      for (const b of OPTS.slice(i + 1)) {
        const d = deltaE(resolveValue(map, a), resolveValue(map, b));
        if (d < 20) problems.push(`${name}: ${a} and ${b} are too close (delta E ${d.toFixed(1)}, needs 20)`);
      }
      for (const s of ["pass", "warn", "fail"]) {
        const d = deltaE(resolveValue(map, a), resolveValue(map, s));
        if (d < 15) problems.push(`${name}: ${a} and ${s} are too close (delta E ${d.toFixed(1)}, needs 15)`);
      }
    });
  }
  for (const [k, v] of Object.entries(darkAttr)) {
    if (darkMedia[k] !== v) problems.push(`dark blocks disagree on --${k}: ${darkMedia[k]} and ${v}`);
  }
  for (const k of Object.keys(darkMedia)) {
    if (!(k in darkAttr)) problems.push(`--${k} is in the OS-dark block only`);
  }
  return { rows, problems };
}

function main() {
  const args = process.argv.slice(2);
  const markdown = args.includes("--markdown");
  const file = resolve(args.find((a) => !a.startsWith("--")) ?? resolve(here, "../src/tokens.css"));
  const css = readFileSync(file, "utf8");
  const { rows, problems } = check(css);
  const site = resolve(here, "../../website/tokens.css");
  if (!args.some((a) => !a.startsWith("--")) && existsSync(site) && readFileSync(site, "utf8") !== css) {
    problems.push("website/tokens.css differs from app/src/tokens.css; copy the app's file over it");
  }
  const fmt = (v) => (v === undefined ? "–" : v.toFixed(2));
  if (markdown) {
    console.log("| Foreground on background | Kind | Light | Dark |\n|---|---|---|---|");
    for (const r of rows) console.log(`| \`${r.fg}\` on \`${r.bg}\` | ${r.kind} | ${fmt(r.light)} | ${fmt(r.dark)} |`);
  } else {
    for (const r of rows) console.log(`${r.kind.padEnd(4)} ${r.fg.padStart(12)} on ${r.bg.padEnd(12)} ${fmt(r.light).padStart(6)} ${fmt(r.dark).padStart(6)}`);
  }
  if (problems.length) {
    console.error(`\n${problems.length} problem(s):\n` + problems.map((p) => `  ${p}`).join("\n"));
    process.exit(1);
  }
  if (!markdown) console.log(`\nAll ${rows.length} pairs pass in both themes; stages stay apart.`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
