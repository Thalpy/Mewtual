<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import StudioFaultControls from "./StudioFaultControls.svelte";
  import { readStudioFault, repairStudioFault } from "./studio-fault.ts";
  import { indexModel, isHex32, studioList, type RecoveryTargetArgs, type StudioIpc } from "./studio-native.ts";
  import type { StudioSession } from "./studio-session.ts";
  let { server, channels, initialChannel, owner, ipc = { invoke, listen } }: {
    server: number; channels: readonly { id: string; name: string }[]; initialChannel: string; owner: boolean; ipc?: StudioIpc;
  } = $props();
  // Settings reads its own typed observations. Changing this target must not reset the
  // editor's shared StudioSession or discard its pending/uncertain saves.
  // svelte-ignore state_referenced_locally
  let channel = $state(initialChannel), kind = $state<"index" | "flipnote" | "score">("index"), object = $state("");
  let entries = $state<{ id: string; title: string; kind: string }[]>([]), listError = $state(""), revision = $state(0);
  let alive = true, listGeneration = 0;
  const target = $derived<RecoveryTargetArgs | null>(kind === "index" ? { server, channel, object: null }
    : isHex32(object) ? { server, channel, object, ...(kind === "score" ? { objectKind: "score" as const } : {}) } : null);
  const key = $derived(`${server}/${channel}/${kind}/${object}/${owner}`);
  async function list() {
    const requested = channel, selectedServer = server, generation = ++listGeneration;
    try {
      const view = await studioList(ipc, { server: selectedServer, channel: requested });
      if (alive && generation === listGeneration && requested === channel && selectedServer === server) { entries = indexModel(view.content).entries; listError = ""; }
    } catch { if (alive && generation === listGeneration && requested === channel && selectedServer === server) { entries = []; listError = "Couldn’t read the document list. You can enter a document ID below."; } }
  }
  $effect(() => { void channel; void server; untrack(() => { revision++; entries = []; object = ""; void list(); }); });
  $effect(() => { void kind; untrack(() => { object = ""; revision++; }); });
  const client: Pick<StudioSession, "readFault" | "repairFault"> = {
    readFault: async (object, scope) => {
      const requested = target, generation = revision;
      if (!requested || object !== requested.object) return null;
      const observation = await readStudioFault(ipc, requested, scope);
      return alive && requested === target && generation === revision ? observation : null;
    },
    repairFault: async (object, scope, observation, selected) => {
      const requested = target, generation = revision;
      if (!requested || object !== requested.object || !owner) return null;
      const result = await repairStudioFault(ipc, requested, scope, observation, selected);
      return alive && requested === target && generation === revision ? result : null;
    },
  };
  onMount(() => {
    const handles: (() => void)[] = [];
    for (const event of ["studio-updated", "settlement-changed"]) void ipc.listen<{server: number; channel: string}>(event, value => {
      if (value.payload.server === server && value.payload.channel === channel) revision++;
    }).then(off => { if (alive) handles.push(off); else off(); });
    return () => { alive = false; revision++; for (const off of handles) off(); };
  });
</script>

<section class="studio-settings">
  <h3>Sync and history</h3>
  <p>History updates automatically. Conflicting versions can be reviewed here.</p>
  <label>Channel <select aria-label="History channel" bind:value={channel}>{#each channels as item}<option value={item.id}>#{item.name}</option>{/each}</select></label>
  <label>History <select aria-label="History type" bind:value={kind}><option value="index">Channel library</option><option value="flipnote">Flipnote</option><option value="score">Music track</option></select></label>
  {#if kind !== "index"}
    <label>Document <select aria-label="History document" bind:value={object}><option value="">Choose a document…</option>{#each entries.filter(entry => entry.kind === kind) as entry}<option value={entry.id}>{entry.title || "Untitled"}</option>{/each}</select></label>
    <details><summary>Enter a document ID</summary><input aria-label="Document ID" bind:value={object} maxlength="32" placeholder="32-character document ID" /></details>
    {#if listError}<p>{listError}</p>{/if}
  {/if}
  {#if target}{#key key}<StudioFaultControls session={client} object={target.object} {revision} />{/key}{/if}
</section>
<style>
  .studio-settings { max-width: 520px; } label { display: flex; flex-direction: column; gap: 5px; margin: 12px 0; }
  select,input { font: inherit; padding: 8px; width: 100%; box-sizing: border-box; } p { color: var(--muted); }
  details { margin-top: 8px; } summary { cursor: pointer; color: var(--muted); }
</style>
