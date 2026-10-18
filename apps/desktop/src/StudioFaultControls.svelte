<script lang="ts">
  import { untrack } from "svelte";
  import type { StudioSession } from "./studio-session.ts";
  import type { FaultScope, FaultView } from "./studio-fault.ts";
  let { session, object, revision }: { session: Pick<StudioSession, "readFault" | "repairFault">; object: string | null; revision: number } = $props();
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
      if (gen === generation && result) message = result === "scheduled" ? "Repair started. Check status for the result." : "Repair is already running.";
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

<section class="fault-controls" aria-label="History status">
  <label>Check <select bind:value={scope} disabled={busy}><option value="source">Document history</option><option value="registry">Shared history directory</option></select></label>
  <button type="button" disabled={busy} onclick={refresh}>{busy ? "Checking…" : "Check history"}</button>
  {#if error}<p role="alert">{error}</p>{/if}
  {#if message}<p role="status">{message}</p>{/if}
  {#if observation}
    {#if observation.repairing}<p>Repair is in progress.</p>
    {:else if !observation.candidates.length}<p>{observation.phase === "fault" ? "Couldn’t read the conflicting versions. Check again." : "No repair needed."}</p>
    {:else}<h4>Conflicting history</h4><p>Choose the version to keep.</p>{/if}
    {#if observation.candidates.length && observation.blockedBy === "notOwner"}<p>Only the server owner can resolve this.</p>{:else if observation.blockedBy === "tenureUnverified" || observation.blockedBy === "tenureUnobserved"}<p>This device is missing the owner history needed for repair. Try from a device with that history.</p>{/if}
    {#if observation.lastDetail}<details><summary>Last repair details</summary><p>{observation.lastDetail}</p></details>{/if}
    {#each observation.candidates as candidate, i (candidate.receipt)}
      <label class="candidate"><input type="radio" name={`repair-${object ?? "index"}-${scope}`} value={candidate.receipt} bind:group={selected} disabled={!observation.mayDecide || busy} />
        <span>{candidate.locallyInstalled ? "Version on this device" : `Other version ${i + 1}`}<details><summary>Details</summary><p>History {candidate.closedEpoch}</p><code>{candidate.receipt}</code><br />Close <code>{candidate.closeRecord}</code><br />Seed <code>{candidate.seedChange}</code></details></span>
      </label>
    {/each}
    {#if observation.candidates.length}<button type="button" disabled={busy || !observation.mayDecide || !selected} onclick={repair}>Keep selected version</button>{/if}
  {/if}
</section>

<style>
  .fault-controls { font-size: .85rem; line-height: 1.5; margin: 10px 0; }
  p { overflow-wrap: anywhere; } select, button { max-width: 100%; margin: 4px 0; font: inherit; }
  .candidate { display: flex; align-items: start; gap: 5px; margin: 8px 0; } code { overflow-wrap: anywhere; word-break: break-all; }
</style>
