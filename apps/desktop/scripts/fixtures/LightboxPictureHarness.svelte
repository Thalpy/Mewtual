<script lang="ts">
  import type { ComponentProps } from "svelte";
  import LightboxPicture from "../../src/LightboxPicture.svelte";
  type Props = ComponentProps<typeof LightboxPicture>;
  let { ...initial } = $props<Props>();
  // One state per prop, as App.svelte binds them: each is its own expression there
  // (`lightbox.source`, `lightboxFile?.mime`, `lightboxZoom`), so a zoom flip does not touch the
  // source. A single replaced props object would invalidate every prop on every `set()` and hide
  // a component that only reacts because something unrelated changed (found in review).
  // `$state.raw` keeps the source <img> out of a deep proxy, as App's own state does.
  // svelte-ignore state_referenced_locally
  let url = $state.raw(initial.url);
  // svelte-ignore state_referenced_locally
  let alt = $state.raw(initial.alt);
  // svelte-ignore state_referenced_locally
  let source = $state.raw(initial.source);
  // svelte-ignore state_referenced_locally
  let mime = $state.raw(initial.mime);
  // svelte-ignore state_referenced_locally
  let zoomed = $state.raw(initial.zoomed);
  // svelte-ignore state_referenced_locally
  let ontoggle = $state.raw(initial.ontoggle);
  export function set(next: Partial<Props>) {
    if ("url" in next) url = next.url!;
    if ("alt" in next) alt = next.alt!;
    if ("source" in next) source = next.source!;
    if ("mime" in next) mime = next.mime!;
    if ("zoomed" in next) zoomed = next.zoomed!;
    if ("ontoggle" in next) ontoggle = next.ontoggle!;
  }
</script>
<LightboxPicture {url} {alt} {source} {mime} {zoomed} {ontoggle} />
