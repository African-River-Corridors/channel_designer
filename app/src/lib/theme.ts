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
