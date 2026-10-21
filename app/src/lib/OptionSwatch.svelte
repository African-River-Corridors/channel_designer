<script lang="ts">
  import { optionVar } from "./theme";

  /** The option's place in the design's list (from 0): its colour and its hatch angle. */
  let { index }: { index: number } = $props();
  const n = $derived((index % 6) + 1);
</script>

<!-- Colour and hatch together, so an option is never told apart by colour alone. -->
<span class="swatch" style:--c={optionVar(index)} style:--wash={optionVar(index, "wash")} style:--angle="calc(var(--opt-{n}-hatch) * 1deg)" aria-hidden="true"></span>

<style>
  .swatch {
    display: inline-block;
    width: 28px;
    height: 12px;
    flex: none;
    vertical-align: middle;
    border: var(--lw-thin) solid var(--c);
    background: repeating-linear-gradient(var(--angle), var(--wash) 0 3px, var(--c) 3px 4px);
  }
  /* Dark theme: the wash is a pastel line on dusk, so the hatch is the line alone. */
  :global(:root[data-theme="dark"]) .swatch { background: repeating-linear-gradient(var(--angle), transparent 0 3px, var(--c) 3px 4px); }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .swatch { background: repeating-linear-gradient(var(--angle), transparent 0 3px, var(--c) 3px 4px); }
  }
</style>
