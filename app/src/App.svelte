<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "./lib/api";
  import type { Choices, Snapshot } from "./lib/api";
  import LeftPanel from "./lib/LeftPanel.svelte";

  // Display state only: the channel design, its stage status and its results all come from Rust.
  let snap = $state<Snapshot | null>(null);
  let choices = $state<Choices | null>(null);
  let error = $state<string | null>(null);

  let leftWidth = $state(340);
  let bottomHeight = $state(200);

  // Sizing recomputes live, after a pause in typing (about 150 ms); nothing else reruns by itself.
  const PAUSE_MS = 150;
  let pending: ReturnType<typeof setTimeout> | undefined;
  let queued: (() => Promise<Snapshot>) | undefined;

  async function guard<T>(work: () => Promise<T>): Promise<T | undefined> {
    try {
      error = null;
      return await work();
    } catch (e) {
      error = String(e);
      return undefined;
    }
  }

  /** Send the queued sizing edit now. A refusal keeps the last good result and shows the reason. */
  async function flush() {
    clearTimeout(pending);
    pending = undefined;
    const send = queued;
    queued = undefined;
    if (!send) return;
    // A refused choice keeps the last good channel shape, shows the reason, and leaves the form as typed.
    const next = await guard(send);
    if (next) snap = next;
  }

  function onSizing(ruleset: string, vessel: string, draught: string) {
    queued = () => api.setSizing(ruleset, vessel, draught);
    clearTimeout(pending);
    pending = setTimeout(() => flush(), PAUSE_MS);
  }

  async function fileAction(action: () => Promise<Snapshot | null>) {
    await flush();
    const next = await guard(action);
    if (next) snap = next;
  }

  function onKey(e: KeyboardEvent) {
    if (!(e.metaKey || e.ctrlKey)) return;
    const k = e.key.toLowerCase();
    if (k === "s") {
      e.preventDefault();
      fileAction(e.shiftKey ? api.saveAs : api.save);
    } else if (k === "o") {
      e.preventDefault();
      fileAction(api.openFile);
    } else if (k === "n") {
      e.preventDefault();
      fileAction(api.newFile);
    }
  }

  onMount(() => {
    guard(async () => {
      choices = await api.sizingChoices();
      snap = await api.snapshot();
    });
  });

  function drag(axis: "x" | "y") {
    return (down: PointerEvent) => {
      const target = down.currentTarget as HTMLElement;
      target.setPointerCapture(down.pointerId);
      const move = (e: PointerEvent) => {
        if (axis === "x") leftWidth = Math.min(Math.max(e.clientX, 220), window.innerWidth - 300);
        else bottomHeight = Math.min(Math.max(window.innerHeight - e.clientY, 80), window.innerHeight - 200);
      };
      const up = () => {
        target.removeEventListener("pointermove", move);
        target.removeEventListener("pointerup", up);
      };
      target.addEventListener("pointermove", move);
      target.addEventListener("pointerup", up);
    };
  }

  let anyStale = $derived(snap?.stages.some((s) => s.status === "stale") ?? false);
</script>

<svelte:window onkeydown={onKey} />

<div class="shell">
  <header>
    <button onclick={() => fileAction(api.newFile)}>New</button>
    <button onclick={() => fileAction(api.openFile)}>Open</button>
    <button onclick={() => fileAction(api.save)}>Save</button>
    <button onclick={() => fileAction(api.saveAs)}>Save as</button>
    <button disabled={!anyStale} onclick={() => fileAction(api.runStale)} title="Rerun every stale stage">Run stale</button>
    <span class="grow"></span>
    {#if snap}
      <span class="meta" data-testid="file-label">{snap.file_name}{snap.dirty ? " (unsaved)" : ""}</span>
      <span class="meta" data-testid="study-title">{snap.document.project.title}</span>
      <span class="meta" data-testid="engine-version">Engine {snap.engine_version}</span>
    {/if}
  </header>

  {#if error}
    <div class="banner error" role="alert" data-testid="error">
      {error}
      <button onclick={() => (error = null)}>Dismiss</button>
    </div>
  {/if}
  {#each snap?.warnings ?? [] as w}
    <div class="banner warn" role="status" data-testid="warning">{w}</div>
  {/each}

  <div class="body" style:grid-template-columns="{leftWidth}px 6px 1fr" style:grid-template-rows="1fr 6px {bottomHeight}px">
    <section class="left"><LeftPanel {snap} {choices} {onSizing} /></section>
    <div class="handle v" role="separator" aria-orientation="vertical" onpointerdown={drag("x")}></div>
    <section class="main">
      <p class="slot">The plan map, long section and cross-section arrive with the later tickets of spec 0003.</p>
    </section>
    <div class="handle h" role="separator" aria-orientation="horizontal" onpointerdown={drag("y")}></div>
    <section class="bottom">
      <p class="slot">The option comparison table and the checks list arrive with the later tickets of spec 0003.</p>
    </section>
  </div>

  <footer data-testid="status-line">Screening — not for construction</footer>
</div>

<style>
  .shell { display: flex; flex-direction: column; height: 100vh; }
  header { display: flex; gap: 6px; align-items: center; padding: 6px 10px; border-bottom: 1px solid var(--line); background: var(--bar); }
  .grow { flex: 1; }
  .meta { color: var(--dim); font-size: 12px; margin-left: 12px; }
  .banner { padding: 6px 10px; font-size: 13px; }
  .banner.error { background: #fbe4e1; color: #7a1d12; }
  .banner.warn { background: #fdf1d3; color: #6b4a00; }
  .body { flex: 1; display: grid; min-height: 0; }
  .body > section { overflow: auto; min-width: 0; min-height: 0; }
  .left { grid-column: 1; grid-row: 1 / 4; }
  .handle.v { grid-column: 2; grid-row: 1 / 4; cursor: col-resize; background: var(--line); }
  .main { grid-column: 3; grid-row: 1; }
  .handle.h { grid-column: 3; grid-row: 2; cursor: row-resize; background: var(--line); }
  .bottom { grid-column: 3; grid-row: 3; }
  .handle:hover { background: var(--gold); }
  .slot { color: var(--dim); padding: 16px; font-style: italic; }
  footer { padding: 4px 10px; background: var(--navy); color: var(--cream); font-size: 12px; letter-spacing: 0.04em; }
</style>
