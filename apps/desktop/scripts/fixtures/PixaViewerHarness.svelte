<script lang="ts">
  import type { ComponentProps } from "svelte";
  import PixaViewer from "../../src/PixaViewer.svelte";
  type Props = ComponentProps<typeof PixaViewer>;
  let { ...initial } = $props<Props>();
  // The test seeds the props once, then drives real reactive prop updates the way App.svelte
  // does after an async package read, an audio tick or a Deafen flip. `$state.raw` with whole
  // object replacement keeps the Pixa (typed arrays inside) out of a deep proxy.
  // svelte-ignore state_referenced_locally
  let live = $state.raw<Props>({ ...initial });
  export function set(next: Partial<Props>) { live = { ...live, ...next }; }
</script>
<PixaViewer {...live} />
