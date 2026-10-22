<script lang="ts">
  import { untrack } from "svelte";
  import { checkCustomVessel, customFields } from "./api";
  import type { Choices, ChoiceSource, CustomCheck, CustomFields, RuleSetChoice, SetInputChoice, Snapshot } from "./api";

  let {
    snap,
    choices,
    onRuleSets,
    onVessel,
    onCustomVessel,
  }: {
    snap: Snapshot | null;
    choices: Choices | null;
    /** Send the chosen Rule Sets (empty: keep them) and the set inputs the user chose. */
    onRuleSets: (ids: string[], inputs: Record<string, string>) => void;
    /** Send the vessel class and the draught as typed (empty: the vessel class's own). */
    onVessel: (vessel: string, draught: string) => void;
    /** Size for a custom vessel, from its fields as typed (spec 0015). */
    onCustomVessel: (fields: CustomFields) => void;
  } = $props();

  /** The picker's last entry: a vessel of the user's own (spec 0015). */
  const CUSTOM = "__custom__";

  let settings = $derived(snap?.document.stages.sizing.settings ?? null);
  let shape = $derived(snap?.document.stages.sizing.results ?? null);
  const OWD = "owd";

  // What the form shows. It follows the document; a choice not yet complete (a set whose input
  // has no value) stays here until the user completes it.
  let sets = $state<string[]>([]);
  let values = $state<Record<string, string>>({});
  let vessel = $state("");
  let draught = $state("");
  /** What the vessel search matches: the id, the name, the family or the region. */
  let vesselSearch = $state("");
  /** The custom vessel's fields as typed, while "Custom vessel…" is chosen. */
  let custom = $state<CustomFields | null>(null);
  /** The check of the typed fields: a refusal turns the Size button off; a warning does not. */
  let check = $state<CustomCheck | null>(null);
  let checking = $state(false);

  // Take the choices from the document whenever it changes (open, new, a sent choice).
  $effect(() => {
    const stored = settings?.rule_sets?.filter((s) => s.id !== OWD).map((s) => s.id) ?? [];
    sets = stored.length ? stored : [choices?.rule_sets[0]?.id ?? ""].filter(Boolean);
    values = Object.fromEntries((settings?.set_inputs ?? []).map((i) => [i.name, i.value]));
    const storedCustom = settings?.custom_vessel;
    vessel = storedCustom ? CUSTOM : (settings?.vessel_standard_id ?? choices?.vessels[0]?.id ?? "");
    custom = storedCustom ? customFields(storedCustom) : null;
    check = null;
    const doc = settings?.draught_m;
    // Not tracked: typing alone must not rerun this, only a new document.
    const typed = untrack(() => draught).trim();
    if (doc == null ? typed !== "" : Number(typed) !== doc) draught = doc == null ? "" : String(doc);
  });

  // The vessel classes by family (scheme), CEMT, RWS and CN first, in catalogue order within a
  // family; the search keeps the classes it matches, and always the one chosen.
  const FIRST = ["CEMT", "RWS", "CN"];
  const rank = (family: string) => {
    const i = FIRST.findIndex((f) => family.startsWith(f));
    return i < 0 ? FIRST.length : i;
  };
  let vesselGroups = $derived.by(() => {
    const q = vesselSearch.trim().toLowerCase();
    const shown = (choices?.vessels ?? []).filter(
      (v) => v.id === vessel || !q || [v.id, v.label, v.family, v.region].some((t) => t?.toLowerCase().includes(q)),
    );
    const groups = new Map<string, typeof shown>();
    for (const v of shown) groups.set(v.family, [...(groups.get(v.family) ?? []), v]);
    return [...groups.entries()].sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b));
  });
  let vesselCount = $derived(vesselGroups.reduce((n, [, vs]) => n + vs.length, 0));

  const setChoice = (id: string): RuleSetChoice | undefined => choices?.rule_sets.find((r) => r.id === id);
  /** The set inputs the chosen sets read, each once. */
  let inputs = $derived(
    sets
      .flatMap((id) => setChoice(id)?.inputs ?? [])
      .filter((i, k, all) => all.findIndex((j) => j.name === i.name) === k),
  );
  /** An input with no value and no default: the user must choose it before the sets can be sent. */
  let missing = $derived(inputs.filter((i) => !values[i.name] && i.default == null));
  let spare = $derived(choices?.rule_sets.filter((r) => !sets.includes(r.id)) ?? []);

  /** The document's mark for a set or an input; `null` while the choice is only in the form. */
  function setSource(id: string): ChoiceSource | null {
    return settings?.rule_sets?.find((s) => s.id === id)?.source ?? null;
  }
  function inputSource(name: string): ChoiceSource | null {
    const stored = settings?.set_inputs?.find((i) => i.name === name);
    return stored && stored.value === values[name] ? stored.source : null;
  }

  /** The inputs the user typed: the ones the document marks typed, and `changed`. A default stays a default. */
  function typedInputs(changed?: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const i of inputs) {
      const v = values[i.name];
      if (v && (i.name === changed || settings?.set_inputs?.some((s) => s.name === i.name && s.source === "typed"))) out[i.name] = v;
    }
    return out;
  }

  /** The sets the document holds, in order, without `owd`. */
  const storedSets = () => settings?.rule_sets?.filter((s) => s.id !== OWD).map((s) => s.id) ?? [];
  const sameAsStored = () => {
    const stored = storedSets();
    return stored.length === sets.length && stored.every((s, k) => s === sets[k]);
  };

  function sendSets() {
    if (missing.length || sets.some((s) => !s) || sameAsStored()) return;
    onRuleSets([...sets], typedInputs());
  }
  function chooseSet(k: number, id: string) {
    sets = sets.map((s, i) => (i === k ? id : s));
    sendSets();
  }
  function addSet() {
    if (!spare.length) return;
    sets = [...sets, spare[0].id];
    sendSets();
  }
  function removeSet(k: number) {
    sets = sets.filter((_, i) => i !== k);
    sendSets();
  }
  function chooseInput(i: SetInputChoice, value: string) {
    values = { ...values, [i.name]: value };
    if (missing.length) return;
    // A set just added goes with its input; otherwise the sets stay as they are.
    onRuleSets(sameAsStored() ? [] : [...sets], typedInputs(i.name));
  }
  function sendVessel() {
    if (vessel === CUSTOM) return;
    onVessel(vessel, draught);
  }

  /** "Custom vessel…": the fields, filled from the class that was selected. Nothing is sent yet. */
  function chooseVessel(next: string) {
    if (next !== CUSTOM) {
      custom = null;
      vessel = next;
      sendVessel();
      return;
    }
    // Chosen again (a second change event): keep the fields as typed.
    if (vessel === CUSTOM && custom) return;
    const from = choices?.vessels.find((v) => v.id === vessel) ?? choices?.vessels[0];
    vessel = CUSTOM;
    const t = (x: number | null | undefined) => (x == null || !Number.isFinite(x) ? "" : String(x));
    custom = {
      name: from ? `${from.id} (custom)` : "Custom vessel",
      loa_m: t(from?.loa_m?.[1]),
      beam_m: t(from?.beam_m),
      draught_m: draught.trim() || t(from?.draught_nominal_m),
      formation: from?.formation === "pushed_convoy" ? "convoy" : "motor",
      n_abreast: "", n_in_line: "", gb_class: "", air_draught_m: "", deadweight_t: "",
    };
    recheck();
  }

  let checkTimer: ReturnType<typeof setTimeout> | undefined;
  /** Check the fields a moment after the last key, so the refusal follows the typing. */
  function recheck() {
    clearTimeout(checkTimer);
    checking = true;
    checkTimer = setTimeout(async () => {
      const fields = custom;
      if (!fields) return;
      try {
        check = await checkCustomVessel({ ...fields });
      } finally {
        checking = false;
      }
    }, 150);
  }
  function setField(k: keyof CustomFields, v: string) {
    if (!custom) return;
    custom = { ...custom, [k]: v };
    recheck();
  }
  const refusedOn = (k: keyof CustomFields) => (check?.refusal?.field === k ? check.refusal.message : null);
  let gbSelected = $derived(sets.some((id) => id.startsWith("gb50139")));
  let canSize = $derived(!!custom && !checking && !!check && !check.refusal);
  function sizeCustom() {
    if (custom && canSize) onCustomVessel({ ...custom });
  }

  const SHAPE: Record<ChoiceSource, string> = { default: "◇", typed: "■" };
  const fmt = (x: number, digits = 1) => x.toFixed(digits);
  const num = (x: number) => (Math.abs(x) >= 100 ? x.toFixed(0) : x.toFixed(2));
</script>

{#snippet mark(source: ChoiceSource | null, what: string)}
  {#if source}
    <span class="mark {source}" data-testid="source-{what}" data-source={source}><span aria-hidden="true">{SHAPE[source]}</span> {source}</span>
  {:else}
    <span class="mark none" data-testid="source-{what}" data-source="">not sent yet</span>
  {/if}
{/snippet}

{#snippet field(k: keyof CustomFields, label: string, mode: "text" | "decimal" | "numeric", f: CustomFields)}
  <label>
    {label}
    <input aria-label={label} data-testid="custom-{k}" inputmode={mode === "text" ? undefined : mode} value={f[k]}
      aria-invalid={refusedOn(k) ? "true" : undefined} oninput={(e) => setField(k, e.currentTarget.value)} />
    {#if refusedOn(k)}<span class="refused" role="alert" data-testid="custom-error-{k}">{refusedOn(k)}</span>{/if}
  </label>
{/snippet}

{#snippet customForm(f: CustomFields)}
  <fieldset class="custom" data-testid="custom-vessel">
    <legend>Custom vessel</legend>
    {@render field("name", "Name", "text", f)}
    {@render field("loa_m", "Length overall (m)", "decimal", f)}
    {@render field("beam_m", "Beam (m)", "decimal", f)}
    {@render field("draught_m", "Draught (m)", "decimal", f)}
    <label>
      Formation
      <select aria-label="Formation" data-testid="custom-formation" value={f.formation} onchange={(e) => setField("formation", e.currentTarget.value)}>
        <option value="motor">motor vessel</option>
        <option value="convoy">pushed convoy</option>
      </select>
      {#if refusedOn("formation")}<span class="refused" role="alert" data-testid="custom-error-formation">{refusedOn("formation")}</span>{/if}
    </label>
    {#if f.formation === "convoy"}
      {@render field("n_abreast", "Barges abreast", "numeric", f)}
      {@render field("n_in_line", "Barges in line", "numeric", f)}
    {/if}
    {#if gbSelected}
      <label>
        GB 50139 class
        <select aria-label="GB 50139 class" data-testid="custom-gb_class" value={f.gb_class} onchange={(e) => setField("gb_class", e.currentTarget.value)}>
          <option value="">none (width and depth not evaluated)</option>
          {#each ["I", "II", "III", "IV", "V", "VI", "VII"] as c}<option value={c}>{c}</option>{/each}
        </select>
      </label>
    {/if}
    <details>
      <summary>More</summary>
      {@render field("air_draught_m", "Air draught (m, stored only)", "decimal", f)}
      {@render field("deadweight_t", "Deadweight (t, stored only)", "decimal", f)}
    </details>
    <button class="primary" data-testid="custom-size" disabled={!canSize} onclick={sizeCustom}>Size</button>
    {#each check?.warnings ?? [] as w}<p class="warn" data-testid="custom-warning">{w}</p>{/each}
  </fieldset>
{/snippet}

<section aria-label="Rule Sets and vessel">
  <h3>Rule Sets and vessel</h3>
  <fieldset class="sets" data-testid="rule-set-picker">
    <legend>Rule Sets</legend>
    {#each sets as id, k (k)}
      <div class="row">
        <label class="grow">
          <span>Rule Set {k + 1}</span>
          <select aria-label="Rule Set {k + 1}" data-testid="rule-set-{k}" value={id} onchange={(e) => chooseSet(k, e.currentTarget.value)}>
            {#each choices?.rule_sets ?? [] as r}
              <option value={r.id} disabled={r.id !== id && sets.includes(r.id)}>{r.label}</option>
            {/each}
          </select>
        </label>
        {@render mark(setSource(id), `set-${k}`)}
        {#if sets.length > 1}
          <button class="small" data-testid="remove-rule-set-{k}" aria-label="Remove Rule Set {k + 1}" onclick={() => removeSet(k)}>Remove</button>
        {/if}
      </div>
    {/each}
    {#if sets.length < 2 && spare.length}
      <button class="small" data-testid="add-rule-set" onclick={addSet}>Add a second Rule Set</button>
    {/if}
    {#each inputs as i (i.name)}
      <div class="row">
        <label class="grow">
          <span>{i.label}</span>
          <select aria-label={i.label} data-testid="set-input-{i.name}" value={values[i.name] ?? ""} onchange={(e) => chooseInput(i, e.currentTarget.value)}>
            {#if !values[i.name]}<option value="" disabled>Choose…</option>{/if}
            {#each i.values as v}<option value={v.value}>{v.label}</option>{/each}
          </select>
        </label>
        {@render mark(inputSource(i.name), `input-${i.name}`)}
      </div>
    {/each}
    {#if missing.length}
      <p class="hint" role="status" data-testid="rule-set-missing">Choose {missing.map((i) => i.label.toLowerCase()).join(" and ")} to size with these Rule Sets.</p>
    {/if}
    {#each choices?.always ?? [] as a}
      <p class="dim" data-testid="always-{a.id}">Always applied: {a.label}.</p>
    {/each}
  </fieldset>
  <label>
    Vessel class
    <input type="search" aria-label="Search vessel classes" data-testid="vessel-search" placeholder="search: id, name, family or region" bind:value={vesselSearch} />
    <select aria-label="Vessel class" value={vessel} onchange={(e) => chooseVessel(e.currentTarget.value)}>
      {#each vesselGroups as [family, vs] (family)}
        <optgroup label={family}>
          {#each vs as v (v.id)}<option value={v.id}>{v.label}</option>{/each}
        </optgroup>
      {/each}
      <option value={CUSTOM} data-testid="custom-vessel-option">Custom vessel…</option>
    </select>
    {#if vesselSearch.trim()}<span class="dim" data-testid="vessel-search-count">{vesselCount} of {choices?.vessels.length ?? 0} classes</span>{/if}
  </label>
  {#if custom}
    {@render customForm(custom)}
  {:else}
    <label>
      Draught (m)
      <input aria-label="Draught" inputmode="decimal" placeholder="vessel default" bind:value={draught} oninput={sendVessel} />
    </label>
  {/if}

  {#if shape}
    <table class="shape" data-testid="channel-shape">
      <caption>Channel shape</caption>
      <tbody>
        <tr><th>Rule Sets</th><td data-testid="shape-rule-sets">{shape.rule_sets}</td></tr>
        <tr><th>Bottom width</th><td data-testid="bottom-width">{fmt(shape.bottom_width_straight_m)} m</td></tr>
        <tr><th>Minimum bend radius</th><td data-testid="min-radius">{fmt(shape.min_bend_radius_m, 0)} m</td></tr>
        <tr><th>Design depth</th><td data-testid="design-depth">{fmt(shape.design_depth_m, 2)} m</td></tr>
        <tr><th>Vessel</th><td data-testid="shape-vessel">{#if shape.vessel_label}{shape.vessel_label}{:else}{fmt(shape.loa_m)} x {fmt(shape.beam_m)} m, draught {fmt(shape.draught_m, 2)} m{/if}</td></tr>
      </tbody>
    </table>
    {#each shape.warnings as w}<p class="warn">{w}</p>{/each}
    {#if shape.design_basis?.length}
      <table class="basis" data-testid="design-basis">
        <caption>Design basis</caption>
        <thead>
          <tr><th scope="col">Quantity</th><th scope="col">Value</th><th scope="col">Governing Rule</th></tr>
        </thead>
        <tbody>
          {#each shape.design_basis as b}
            <tr data-testid="basis-row" data-quantity={b.quantity}>
              <th scope="row">{b.quantity.replace(/-/g, " ")}{#if b.waterway}<span class="sub">{b.waterway.replace(/-/g, " ")}</span>{/if}</th>
              <td class="num">{num(b.value)} {b.unit}</td>
              <td class="rule">
                <code>{b.rule}</code>
                <span class="sub">Sets: {b.rule_sets.join(", ")}</span>
                <span class="sub">{b.citation}</span>
                <span class="tags">
                  <span class="tag origin" data-testid="basis-origin">origin: {b.origin}</span>
                  <span class="tag status {b.status}" data-testid="basis-status"><span aria-hidden="true">{b.status === "verified" ? "✓" : "?"}</span> {b.status}</span>
                </span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
    {#if shape.rules_notice}<p class="notice" data-testid="rules-notice"><span aria-hidden="true">?</span> {shape.rules_notice}</p>{/if}
  {:else}
    <p class="dim" data-testid="no-shape">Choose the Rule Sets and a vessel class to see the channel shape.</p>
  {/if}
</section>

<style>
  label { display: flex; flex-direction: column; gap: 3px; margin: var(--sp-2) 0; font-size: var(--fs-12); color: var(--ink-2); }
  fieldset { border: var(--lw-thin) solid var(--rule); border-radius: var(--radius); margin: var(--sp-2) 0; padding: var(--sp-1) var(--sp-2) var(--sp-2); }
  legend { font: 600 var(--fs-11) var(--font-ui); letter-spacing: var(--track-caps); text-transform: uppercase; color: var(--ink-2); padding: 0 var(--sp-1); }
  .row { display: flex; gap: var(--sp-2); align-items: flex-end; }
  .row .grow { flex: 1; min-width: 0; margin-bottom: var(--sp-1); }
  .row select { width: 100%; }
  .row .mark, .row button { margin-bottom: var(--sp-2); }
  button.small { font-size: var(--fs-12); padding: 2px var(--sp-2); }
  .mark { display: inline-flex; gap: 4px; align-items: center; padding: 0 var(--sp-2); font-size: var(--fs-12); font-weight: 600; border: var(--lw-thin) solid currentColor; border-radius: var(--radius); white-space: nowrap; }
  .mark.default { color: var(--ink-2); background: var(--surface-2); }
  .mark.typed { color: var(--pass); background: var(--pass-soft); }
  .mark.none { color: var(--ink-3); font-weight: 400; border-style: dashed; }
  .hint { margin: var(--sp-1) 0; font-size: var(--fs-12); color: var(--warn); }
  .shape, .basis { width: 100%; border-collapse: collapse; margin-top: var(--sp-4); }
  caption { text-align: left; font: 600 var(--fs-11) var(--font-ui); letter-spacing: var(--track-caps); text-transform: uppercase; color: var(--ink-2); padding-bottom: var(--sp-1); border-bottom: var(--lw-thin) solid var(--ink); }
  th { text-align: left; font-weight: 500; color: var(--ink); padding: var(--sp-1) 0; font-size: var(--fs-12); border-bottom: var(--lw-thin) solid var(--rule); vertical-align: top; }
  td { text-align: right; padding: var(--sp-1) 0; font-size: var(--fs-12); color: var(--ink-2); font-variant-numeric: tabular-nums; border-bottom: var(--lw-thin) solid var(--rule); vertical-align: top; }
  .basis thead th { font: 600 var(--fs-11) var(--font-ui); color: var(--ink-2); }
  .basis th, .basis td { padding-right: var(--sp-2); }
  .basis td.num { white-space: nowrap; }
  .basis td.rule { text-align: left; }
  .basis code { font-size: var(--fs-11); word-break: break-all; color: var(--ink); }
  .sub { display: block; font-size: var(--fs-11); color: var(--ink-3); font-weight: 400; }
  .tags { display: flex; flex-wrap: wrap; gap: var(--sp-1); margin-top: 2px; }
  .tag { font: 600 var(--fs-11) var(--font-ui); padding: 0 6px; border: var(--lw-thin) solid currentColor; border-radius: var(--radius); }
  .tag.origin { color: var(--ink-2); background: var(--surface-2); }
  .tag.status { color: var(--warn); background: var(--warn-soft); }
  .tag.status.verified { color: var(--pass); background: var(--pass-soft); }
  .warn { background: var(--warn-soft); color: var(--warn); padding: var(--sp-1) var(--sp-2); font-size: var(--fs-12); }
  .warn::before { content: "! "; font-weight: 700; }
  .notice { margin: var(--sp-2) 0 0; padding: var(--sp-1) var(--sp-2); font-size: var(--fs-12); color: var(--warn); background: var(--warn-soft); }
  .dim { color: var(--ink-3); }
  .refused { color: var(--fail); font-size: var(--fs-12); }
  input[aria-invalid="true"] { border-color: var(--fail); }
  details summary { font-size: var(--fs-12); color: var(--ink-2); cursor: pointer; margin: var(--sp-1) 0; }
  fieldset .dim { margin: var(--sp-1) 0 0; font-size: var(--fs-12); }
</style>
