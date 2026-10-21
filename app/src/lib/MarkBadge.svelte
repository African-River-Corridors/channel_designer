<script lang="ts">
  import type { Mark } from "./api";

  /** Who made an input: a word and a shape, never colour alone. */
  let { mark, input = "" }: { mark: Mark | undefined; input?: string } = $props();
  const SHAPE: Record<Mark, string> = { default: "◇", edited: "✎", user: "■" };
</script>

{#if mark}
  <span class="badge {mark}" data-testid="mark-{input}" data-mark={mark}><span aria-hidden="true">{SHAPE[mark]}</span> {mark}</span>
{:else}
  <span class="badge none" data-testid="mark-{input}" data-mark="">no record</span>
{/if}

<style>
  .badge { display: inline-flex; gap: 4px; align-items: center; padding: 0 var(--sp-2); font-size: var(--fs-12); font-weight: 600; border: var(--lw-thin) solid currentColor; border-radius: var(--radius); white-space: nowrap; }
  .default { color: var(--ink-2); background: var(--surface-2); }
  .edited { color: var(--warn); background: var(--warn-soft); }
  .user { color: var(--pass); background: var(--pass-soft); }
  .none { color: var(--ink-3); font-weight: 400; border-style: dashed; }
</style>
