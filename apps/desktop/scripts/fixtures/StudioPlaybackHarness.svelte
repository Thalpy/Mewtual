<script lang="ts">
  import Studio from "../../src/Studio.svelte";
  import type { ComponentProps } from "svelte";
  import type { StudioAudioState } from "../../src/studio-audio.ts";
  let { props }: { props: ComponentProps<typeof Studio> } = $props();
  let audioState = $state<StudioAudioState>({ state: "stopped", elapsedMs: 0 });
  export function updateAudio(value: StudioAudioState) { audioState = value; }
</script>
<Studio {...props} {audioState} />
