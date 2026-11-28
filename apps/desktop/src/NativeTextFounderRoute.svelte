<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  // Twice MAX_NATIVE_TEXT_INVITE_PACKET_BYTES in the native packet codec.
  const maxInviteHex = 6254;
  const maxHost = 45;
  let { server, context, isUiLive }: { server: number; context: string; isUiLive: () => boolean } = $props();
  let invite = $state("");
  let host = $state("");
  let busy = $state(false);
  let status = $state("");
  let canRemember = $derived(invite.length <= maxInviteHex && host.length <= maxHost
    && invite.trim().length > 0 && host.trim().length > 0);
  let disposed = false;
  onDestroy(() => { disposed = true; });

  async function changeAddress(action: "remember" | "forget") {
    if (disposed || busy || !isUiLive()) return;
    const selection = { server, expectedContext: context };
    if (action === "remember" && !canRemember) return;
    const current = () => !disposed && isUiLive() && server === selection.server && context === selection.expectedContext;
    busy = true;
    status = "";
    try {
      if (action === "remember") {
        await invoke("remember_native_chat_founder_route", {
          ...selection, inviteHex: invite.trim(), approvedHost: host.trim(),
        });
      } else {
        await invoke("forget_native_chat_founder_route", selection);
      }
      if (!current()) return;
      invite = host = "";
      status = action === "remember" ? "Founder address saved on this device." : "Founder address cleared.";
    } catch {
      if (current()) status = action === "remember"
        ? "Couldn’t save the founder address. Check the invite and IP address."
        : "Couldn’t clear the founder address. Try again.";
    } finally {
      if (!disposed) busy = false;
    }
  }
</script>

<details>
  <summary>Automatic founder reconnect</summary>
  <form onsubmit={(event) => { event.preventDefault(); void changeAddress("remember"); }}>
    <label>Original invite code <input type="text" bind:value={invite} maxlength={maxInviteHex} disabled={busy} spellcheck="false" autocomplete="off" /></label>
    <label>Founder IP address <input type="text" bind:value={host} maxlength={maxHost} disabled={busy} spellcheck="false" autocomplete="off" /></label>
    <button type="submit" class="ghost small" disabled={busy || !canRemember}>Connect and remember</button>
  </form>
  <button type="button" class="ghost small" disabled={busy} onclick={() => void changeAddress("forget")}>Forget founder address</button>
  {#if status}<p class="small" role="status">{status}</p>{/if}
</details>

<style>
  details { margin-block: 0.75em; }
  form { display: grid; gap: 0.35em; margin-block: 0.45em; }
  input { display: block; min-width: 0; width: 100%; }
  summary { cursor: pointer; }
</style>
