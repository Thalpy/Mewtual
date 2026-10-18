<script lang="ts">
  // Test fixture: mounts the score composer behind a reactive model so a test can replace the
  // model mid-gesture, the way a remote edit replaces the working projection under a drag.
  import ScoreEditor from "../../src/ScoreEditor.svelte";
  import type { ComponentProps } from "svelte";
  import type { ScoreModel } from "../../src/studio-native.ts";
  let { props }: { props: Omit<ComponentProps<typeof ScoreEditor>, "model"> & { model: ScoreModel | null } } = $props();
  // svelte-ignore state_referenced_locally
  let model = $state<ScoreModel | null>(props.model);
  export function update(next: ScoreModel | null) { model = next; }
</script>
<ScoreEditor {...props} {model} />
