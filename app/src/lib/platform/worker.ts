// The design session, in a Web Worker: a stage runs to its end here without freezing the page. The page
// sends {id, method, args}; the worker answers {id, ok, result} or {id, ok: false, error}, and posts
// {progress} while a stage runs. Cancel is the page ending this worker (wasm cannot unwind out of a stage).
import init, { Session } from "../../../wasm-pkg/owd_web.js";
import wasmUrl from "../../../wasm-pkg/owd_web_bg.wasm?url";

let session: Session | undefined;
// The handler below is set before any await, so no message sent while the module loads is lost.
const ready = init({ module_or_path: wasmUrl }).then(() => {
  session = new Session();
});

type Methods = Record<string, (...args: unknown[]) => unknown>;

/** The network for the reach steps: a synchronous request, which a Worker may make. `start` is -1 for
 * the whole file. A host that ignores the range sends the whole file, and the range is cut from it. */
function syncFetch(url: string, start: number, len: number): Uint8Array {
  const x = new XMLHttpRequest();
  x.open("GET", url, false);
  x.responseType = "arraybuffer";
  if (start >= 0) x.setRequestHeader("Range", `bytes=${start}-${start + len - 1}`);
  x.send();
  if (x.status !== 200 && x.status !== 206) throw new Error(`HTTP ${x.status} from ${url}`);
  const all = new Uint8Array(x.response as ArrayBuffer);
  return start >= 0 && x.status === 200 ? all.slice(start, start + len) : all;
}

/** The methods that reach the network take the fetch as their last argument. */
const NETWORK = new Set(["reach_route", "reach_step", "reach_search", "reach_river", "qa_overlay"]);

self.onmessage = async (e: MessageEvent<{ id: number; method: string; args: unknown[] }>) => {
  const { id, method, args } = e.data;
  try {
    await ready;
    const s = session as unknown as Methods;
    if (method === "ping") return self.postMessage({ id, ok: true, result: true });
    if (typeof s[method] !== "function") throw new Error(`no such session method: ${method}`);
    const given = method === "run_stage" ? [args[0], args[1], (n: number) => self.postMessage({ progress: n })] : NETWORK.has(method) ? [...args, syncFetch] : args;
    self.postMessage({ id, ok: true, result: s[method].apply(session, given) });
  } catch (err) {
    self.postMessage({ id, ok: false, error: String(err) });
  }
};
