<script lang="ts">
  import type { ComponentProps } from "svelte";
  import StudioSound from "../../src/StudioSound.svelte";
  type Props = ComponentProps<typeof StudioSound>;
  let { frame: initialFrame, model: initialModel, ...rest } = $props<Props>();
  // The test intentionally seeds parent-owned reactive props once, then drives real prop updates:
  // a frame change exercises the component's reset effect and its async fences; a model change
  // stands in for the host re-reading the document after a save lands.
  // svelte-ignore state_referenced_locally
  let frame = $state(initialFrame);
  // svelte-ignore state_referenced_locally
  let model = $state(initialModel);
  export function setFrame(next: string) { frame = next; }
  export function setModel(next: Props["model"]) { model = next; }
</script>
<StudioSound {...rest} {frame} {model} />
