<script lang="ts">
  import type { ComponentProps } from "svelte";
  import LightboxPicture from "../../src/LightboxPicture.svelte";
  type Props = ComponentProps<typeof LightboxPicture>;
  let { ...initial } = $props<Props>();
  // The test seeds the props once, then drives real reactive prop updates the way App.svelte does
  // when the zoom flag flips or the file index arrives late. `$state.raw` keeps the source <img>
  // out of a deep proxy, as App's own state does for an element.
  // svelte-ignore state_referenced_locally
  let live = $state.raw<Props>({ ...initial });
  export function set(next: Partial<Props>) { live = { ...live, ...next }; }
</script>
<LightboxPicture {...live} />
