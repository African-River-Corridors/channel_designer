// The desktop adapter: the Tauri commands, unchanged.
import { invoke } from "@tauri-apps/api/core";
import type { Platform } from "./types";

export const tauriPlatform: Platform = {
  kind: "tauri",
  call: (command, args) => invoke(command, args),
  // Outside the Tauri window (a browser, a test) these have nothing to listen to and reject.
  onProgress: async (handler) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<number>("stage-progress", (e) => handler(e.payload));
  },
  onFileDrop: async (handler) => {
    const { getCurrentWebview } = await import("@tauri-apps/api/webview");
    return getCurrentWebview().onDragDropEvent((e) => {
      const p = e.payload;
      // Tauri gives physical pixels.
      const css = (v: number) => v / window.devicePixelRatio;
      if (p.type === "leave") handler({ type: "leave" });
      else if (p.type === "over") handler({ type: "over", x: css(p.position.x), y: css(p.position.y) });
      else if (p.type === "drop" && p.paths[0]) handler({ type: "drop", x: css(p.position.x), y: css(p.position.y), name: p.paths[0], file: p.paths[0] });
    });
  },
};
