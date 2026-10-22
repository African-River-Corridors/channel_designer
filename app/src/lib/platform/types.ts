// What the app needs from the place it runs in. Every command the front end calls goes through `call`,
// under the same names the desktop app's Tauri commands have (spec 0011).

/** Files dropped on the window. Positions are CSS pixels. `file` is a path on the desktop, a File in a browser. */
export type FileDropEvent =
  | { type: "leave" }
  | { type: "over"; x: number; y: number }
  | { type: "drop"; x: number; y: number; name: string; file: string | File };

/** What a long network step is doing (#320): step `done` of `total` (0: not counted), in words. */
export interface ReachProgress {
  done: number;
  total: number;
  what: string;
}

export interface Platform {
  kind: "tauri" | "web";
  /** Run one command. Rejects with the session's reason as a string. */
  call<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  /** The terrain reads a running stage has made so far. Resolves to an unsubscribe. */
  onProgress(handler: (samples: number) => void): Promise<() => void>;
  /** Progress of the network step that is running (route, inputs, level, DEM). Resolves to an unsubscribe. */
  onReachProgress(handler: (p: ReachProgress) => void): Promise<() => void>;
  /** Files dropped on the window. Resolves to an unsubscribe. */
  onFileDrop(handler: (event: FileDropEvent) => void): Promise<() => void>;
}
