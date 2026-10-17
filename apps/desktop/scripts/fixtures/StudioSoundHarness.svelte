<script lang="ts">
  import type { ComponentProps } from "svelte";
  import StudioSound from "../../src/StudioSound.svelte";
  let { frame: initialFrame, ...rest } = $props<ComponentProps<typeof StudioSound>>();
  // The test intentionally seeds a parent-owned reactive frame once, then drives real prop updates.
  // svelte-ignore state_referenced_locally
  let frame = $state(initialFrame);
  export function setFrame(next: string) { frame = next; }
</script>
<StudioSound {...rest} {frame} />
