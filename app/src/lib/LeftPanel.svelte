<script lang="ts">
  import type { Choices, Snapshot } from "./api";
  import DataSummary from "./DataSummary.svelte";
  import SizingForm from "./SizingForm.svelte";
  import StageList from "./StageList.svelte";

  let {
    snap,
    choices,
    onSizing,
  }: {
    snap: Snapshot | null;
    choices: Choices | null;
    onSizing: (ruleset: string, vessel: string, draught: string) => void;
  } = $props();
</script>

<div class="left">
  <StageList {snap} />
  <DataSummary {snap} />
  <SizingForm {snap} {choices} onChange={onSizing} />
</div>

<style>
  .left { padding: 10px 12px; display: flex; flex-direction: column; gap: 14px; }
</style>
