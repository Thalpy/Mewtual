<script lang="ts">
  import {onDestroy} from "svelte";
  // Test-only: the first production callback must still retire its actual
  // App-function fixture when a later real Svelte teardown throws.
  let {retire,fail}:{retire:()=>void;fail:()=>void}=$props();
  onDestroy(retire);
  onDestroy(fail);
</script>
