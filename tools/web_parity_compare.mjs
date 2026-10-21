// The document comparison of tools/web_parity.mjs, apart so it can be tested (web_parity_compare.test.mjs).

// ADR 0005 / tools/design_impact.py: two numbers differ when they are further apart than this.
export const REL_TOL = 1e-9;
export const ABS_TOL = 1e-9;

/** Compare two JSON values. Returns {numbers, worst, problems[]}. */
export function compare(a, b) {
  const out = { numbers: 0, worst: 0, problems: [] };
  const walk = (x, y, path) => {
    if (typeof x === "number" && typeof y === "number") {
      out.numbers++;
      const d = Math.abs(x - y);
      const limit = Math.max(ABS_TOL, REL_TOL * Math.max(Math.abs(x), Math.abs(y)));
      out.worst = Math.max(out.worst, d / limit);
      if (d > limit) out.problems.push(`${path}: ${x} vs ${y}`);
    } else if (Array.isArray(x) && Array.isArray(y)) {
      if (x.length !== y.length) out.problems.push(`${path}: length ${x.length} vs ${y.length}`);
      for (let i = 0; i < Math.min(x.length, y.length); i++) walk(x[i], y[i], `${path}[${i}]`);
    } else if (x && y && typeof x === "object" && typeof y === "object" && !Array.isArray(x) && !Array.isArray(y)) {
      const keys = new Set([...Object.keys(x), ...Object.keys(y)]);
      for (const k of keys) {
        if (!(k in x) || !(k in y)) out.problems.push(`${path}.${k}: only in ${k in x ? "native" : "wasm"}`);
        else walk(x[k], y[k], `${path}.${k}`);
      }
    } else if (x !== y) out.problems.push(`${path}: ${JSON.stringify(x)} vs ${JSON.stringify(y)}`);
  };
  walk(a, b, "$");
  return out;
}

