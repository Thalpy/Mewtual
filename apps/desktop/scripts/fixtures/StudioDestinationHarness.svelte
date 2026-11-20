<script lang="ts">
  import Studio from "../../src/Studio.svelte";
  import type { ComponentProps } from "svelte";
  let { props }: { props: ComponentProps<typeof Studio> } = $props();
  // svelte-ignore state_referenced_locally
  let server = $state(props.server);
  // svelte-ignore state_referenced_locally
  let channel = $state(props.channel);
  export function destination(nextServer: typeof server, nextChannel: string) {
    server = nextServer; channel = nextChannel;
  }
</script>
<Studio {...props} {server} {channel} />
