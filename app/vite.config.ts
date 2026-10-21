import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed dev port and does not want Vite to clear the screen.
export default defineConfig({
  // The desktop bundle holds the Tauri adapter only (the web build sets __OWD_WEB__ to true).
  define: { __OWD_WEB__: false },
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
});
