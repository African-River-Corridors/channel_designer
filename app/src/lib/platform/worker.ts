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

self.onmessage = async (e: MessageEvent<{ id: number; method: string; args: unknown[] }>) => {
  const { id, method, args } = e.data;
  try {
    await ready;
    const s = session as unknown as Methods;
    if (method === "ping") return self.postMessage({ id, ok: true, result: true });
    if (typeof s[method] !== "function") throw new Error(`no such session method: ${method}`);
    const given = method === "run_stage" ? [args[0], args[1], (n: number) => self.postMessage({ progress: n })] : args;
    self.postMessage({ id, ok: true, result: s[method].apply(session, given) });
  } catch (err) {
    self.postMessage({ id, ok: false, error: String(err) });
  }
};
