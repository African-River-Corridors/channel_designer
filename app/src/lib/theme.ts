// Reads the brand tokens (src/tokens.css) where CSS cannot: a map layer or a canvas needs a resolved colour.

/** The number of design line options the palette has. */
export const OPTION_COUNT = 6;

/** A token's current value, for example `token("ink")`. Resolved for the theme now showing. */
export function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(`--${name}`).trim();
}

/** The CSS reference to an option's line colour, by its place in the design's list (from 0). */
export const optionVar = (i: number, kind: "" | "wash" | "map" = "") =>
  `var(--opt-${(i % OPTION_COUNT) + 1}${kind ? `-${kind}` : ""})`;

/** The resolved line colour of an option, for canvas and map code. */
export const optionColour = (i: number, kind: "" | "wash" | "map" = "") =>
  token(`opt-${(i % OPTION_COUNT) + 1}${kind ? `-${kind}` : ""}`);

/** The hatch angle of an option, in degrees. */
export const optionHatch = (i: number) => Number(token(`opt-${(i % OPTION_COUNT) + 1}-hatch`));

/** Call `cb` when the theme changes: the OS setting, or `data-theme` on the root. Returns the unsubscribe. */
export function onThemeChange(cb: () => void): () => void {
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  mq.addEventListener("change", cb);
  const mo = new MutationObserver(cb);
  mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  return () => {
    mq.removeEventListener("change", cb);
    mo.disconnect();
  };
}

/** A colour as canvas code can use it: a `var(--name)` reference resolved now, anything else unchanged. */
export function resolve(colour: string): string {
  const m = /^var\(--([a-z0-9-]+)\)$/.exec(colour.trim());
  return m ? token(m[1]) : colour;
}

export type ThemeChoice = "auto" | "light" | "dark";
const KEY = "owd-theme";

/** The choice now in force: `auto` follows the OS (no `data-theme`), the others pin it. */
export function currentTheme(): ThemeChoice {
  const t = document.documentElement.dataset.theme;
  return t === "light" || t === "dark" ? t : "auto";
}

/** Pin a theme or return to the OS setting, and remember it. Storage may be unavailable: then the choice lasts for the session. */
export function setTheme(choice: ThemeChoice) {
  if (choice === "auto") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = choice;
  try {
    if (choice === "auto") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, choice);
  } catch {
    /* storage is blocked: nothing to remember */
  }
}

/** Apply the remembered choice at start-up, before anything is drawn. */
export function applyStoredTheme() {
  try {
    const t = localStorage.getItem(KEY);
    if (t === "light" || t === "dark") document.documentElement.dataset.theme = t;
  } catch {
    /* storage is blocked: follow the OS */
  }
}
