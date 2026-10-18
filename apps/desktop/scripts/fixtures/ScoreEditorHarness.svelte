<script lang="ts">
  // Test fixture: mounts the score composer behind reactive props so a test can replace the model
  // mid-gesture (the way a remote edit replaces the working projection under a drag) or turn the
  // editor read-only and back while a press is held.
  import ScoreEditor from "../../src/ScoreEditor.svelte";
  import type { ComponentProps } from "svelte";
  import type { ScoreModel } from "../../src/studio-native.ts";
  let { props }: { props: Omit<ComponentProps<typeof ScoreEditor>, "model"> & { model: ScoreModel | null } } = $props();
  // svelte-ignore state_referenced_locally
  let model = $state<ScoreModel | null>(props.model);
  // svelte-ignore state_referenced_locally
  let readonlyReason = $state<string>(props.readonlyReason);
  export function update(next: ScoreModel | null) { model = next; }
  export function setReadonly(reason: string) { readonlyReason = reason; }
</script>
<ScoreEditor {...props} {model} {readonlyReason} />
