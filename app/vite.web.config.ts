import { defineConfig, type Plugin } from "vite";
import { readFileSync } from "node:fs";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The web build: static files for any static server. The page is served at /owd/app (the website build
// copies index.html to app.html) and its files live under /owd/app-files/ (a folder named "app" next to
// app.html would make GitHub Pages redirect /owd/app). Vite's base makes every URL in the built HTML and
// scripts an absolute /owd/app-files/... path, including the .wasm, the worker and the fonts.

/** The example the "Open the example" button loads, from examples/web, served under examples/. */
function exampleFiles(): Plugin {
  return {
    name: "owd-example-files",
    generateBundle() {
      for (const name of ["river-example.owd", "river-dem.tif"]) {
        this.emitFile({ type: "asset", fileName: `examples/${name}`, source: readFileSync(new URL(`../examples/web/${name}`, import.meta.url)) });
      }
    },
  };
}

export default defineConfig({
  plugins: [svelte(), exampleFiles()],
  base: "/owd/app-files/",
  define: { __OWD_WEB__: true },
  worker: { format: "es" },
  build: { outDir: "dist-web", emptyOutDir: true, assetsInlineLimit: 0 },
  server: { fs: { allow: [".."] } },
});
