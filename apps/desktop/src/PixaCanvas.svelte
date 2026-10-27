<script lang="ts">
  import type { Pixa } from "./pixa.ts";
  import { pixaFrameAt } from "./pixa-media.ts";
  import { decodePix, pixToRgba } from "./pix.ts";
  let { pixa, elapsedMs = 0, loop = false, label = "Animation" }: { pixa: Pixa; elapsedMs?: number; loop?: boolean; label?: string } = $props();
  let canvas: HTMLCanvasElement;
  const frame = $derived(pixaFrameAt(pixa, elapsedMs, loop));
  $effect(() => {
    // Only the selected encoded frame expands. No 999-frame raster collection is retained.
    const image = decodePix(pixa.frames[frame]);
    canvas?.getContext("2d")?.putImageData(new ImageData(pixToRgba(image), image.w, image.h), 0, 0);
  });
</script>

<canvas bind:this={canvas} width="192" height="144" aria-label={`${label}, frame ${frame + 1} of ${pixa.frames.length}`} data-pixa-frame={frame}></canvas>

<style>
  canvas { display: block; width: 100%; height: 100%; max-height: 100%; object-fit: contain; image-rendering: pixelated; }
</style>
