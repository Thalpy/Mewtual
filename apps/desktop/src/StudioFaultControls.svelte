<script lang="ts">
  import { untrack } from "svelte";
  import type { StudioSession } from "./studio-session.ts";
  import type { FaultScope, FaultView } from "./studio-fault.ts";
  let { session, object, revision }: { session: StudioSession; object: string | null; revision: number } = $props();
  let scope = $state<FaultScope>("source"), observation = $state.raw<FaultView | null>(null), selected = $state("");
  let busy = $state(false), error = $state(""), message = $state("");
  let generation = 0;
  let readGeneration = 0;
  async function refresh() {
    const gen = ++generation, read = ++readGeneration, target = object, requested = scope;
    busy = true; selected = ""; error = "";
    try { const value = await session.readFault(target, requested); if (gen === generation && read === readGeneration) observation = value; }
    catch (e) { if (gen === generation && read === readGeneration) { observation = null; error = String(e); } }
    finally { if (gen === generation) busy = false; }
  }
  async function repair() {
    if (!observation || !selected || busy) return;
    const gen = ++generation, evidence = observation, choice = selected, target = object, requested = scope;
    busy = true; error = ""; selected = "";
    // Clear the decision before IPC. Even an uncertain response requires a fresh observation;
    // neither a late reply nor the old radio choice authorizes a second decision.
    observation = null;
    try {
      const result = await session.repairFault(target, requested, evidence, choice);
      if (gen === generation && result) message = result === "scheduled" ? "Repair scheduled; refresh to see its actual outcome." : "A repair is already running; refresh its status.";
    } catch (e) { if (gen === generation) error = `${String(e)} · Refresh the actual status before choosing again.`; }
    finally { if (gen === generation) busy = false; }
  }
  $effect(() => {
    void object; void scope;
    untrack(() => { generation++; busy = false; observation = null; selected = ""; error = ""; message = ""; });
    return () => { generation++; };
  });
  $effect(() => {
    void revision;
    // A notification only invalidates evidence. It must never select or issue a repair.
    untrack(() => { readGeneration++; if (observation) message = "History changed; refresh repair status."; observation = null; selected = ""; });
  });
</script>

<details class="fault-controls">
  <summary>Owner repair</summary>
  <p>Choose between the actual conflicting receipts. Repair preserves displaced edits in recovery; scheduling alone does not resolve the fault.</p>
  <label>Fault scope <select bind:value={scope} disabled={busy}><option value="source">This document</option><option value="registry">Its Registry bucket</option></select></label>
  <button type="button" disabled={busy} onclick={refresh}>{busy ? "Working…" : "Refresh repair status"}</button>
  {#if error}<p role="alert">{error}</p>{/if}
  {#if message}<p role="status">{message}</p>{/if}
  {#if observation}
    <p>{observation.scope.kind === "source" ? "Document" : `Registry bucket ${observation.scope.bucket}`} · {observation.phase}{#if observation.blockedBy} · {observation.blockedBy}{/if}</p>
    {#if observation.lastOutcome}<p>Last local attempt: {observation.lastOutcome}{#if observation.lastDetail} · {observation.lastDetail}{/if}</p>{/if}
    <p>{observation.preservedOperations} operations would be preserved · {observation.waiting} further pairs waiting</p>
    {#each observation.candidates as candidate (candidate.receipt)}
      <label class="candidate"><input type="radio" name={`repair-${object ?? "index"}-${scope}`} value={candidate.receipt} bind:group={selected} disabled={!observation.mayDecide || busy} />
        <span>Epoch {candidate.closedEpoch}{candidate.locallyInstalled ? " · locally installed" : ""}<br /><code>{candidate.receipt}</code><br />Close <code>{candidate.closeRecord}</code><br />Seed <code>{candidate.seedChange}</code></span>
      </label>
    {/each}
    <button type="button" disabled={busy || !observation.mayDecide || !selected} onclick={repair}>Repair using selected receipt</button>
  {/if}
</details>

<style>
  .fault-controls { font-size: 11px; line-height: 1.5; margin: 10px 0; }
  p { overflow-wrap: anywhere; } select, button { max-width: 100%; margin: 4px 0; font: inherit; }
  .candidate { display: flex; align-items: start; gap: 5px; margin: 8px 0; } code { overflow-wrap: anywhere; word-break: break-all; }
</style>
