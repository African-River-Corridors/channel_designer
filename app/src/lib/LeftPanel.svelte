<script lang="ts">
  import type { Choices, InputRole, RunMethod, Snapshot, StageName } from "./api";
  import DataSummary from "./DataSummary.svelte";
  import SizingForm from "./SizingForm.svelte";
  import StageList from "./StageList.svelte";

  let {
    snap,
    choices,
    onSizing,
    busy,
    samples,
    dropTarget,
    onRun,
    onCancel,
    onLoad,
    onFetch,
    onWater,
  }: {
    snap: Snapshot | null;
    choices: Choices | null;
    onSizing: (ruleset: string, vessel: string, draught: string) => void;
    busy: StageName | null;
    samples: number;
    dropTarget: InputRole | null;
    onRun: (stage: StageName, method?: RunMethod) => void;
    onCancel: () => void;
    onLoad: (role: InputRole) => void;
    onFetch: () => void;
    onWater: (level: string, fall: string) => void;
  } = $props();
</script>

<div class="left">
  <StageList {snap} {busy} {samples} {onRun} {onCancel} />
  <DataSummary {snap} busy={busy !== null} {dropTarget} {onLoad} {onFetch} {onWater} />
  <SizingForm {snap} {choices} onChange={onSizing} />
</div>

<style>
  .left { padding: var(--sp-3) var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-5); }
</style>
