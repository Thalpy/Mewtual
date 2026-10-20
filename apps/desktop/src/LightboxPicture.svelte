<script lang="ts">
  /**
   * The picture inside the full-screen image viewer: a toggle between "fit to window" and
   * "actual size".
   *
   * The host (App.svelte) owns the viewer chrome, the zoom flag and the lifecycle, and keys this
   * component on the open viewer, so every opening gets a fresh instance and its own copy.
   *
   * For a large still image the fit view is a canvas holding a copy resampled once, at mount, to
   * the most pixels the screen can show (see `lightbox-image.ts` for why: an earlier version made
   * the GPU shrink the full original again on every switch, and a Linux tester's Intel GPU hung on
   * it). Both elements stay mounted for the whole opening and the zoom flag only hides one of them,
   * so switching back and forth never repeats the resample and never reloads the original.
   *
   * The paint runs on the main thread when the viewer opens and includes a full-size decode of the
   * source, so its cost grows with the source's pixels. The source is the PNG the native media
   * path produced, which admits at most 25 megapixels; a large photo costs roughly 0.1 to 0.4 s
   * once per opening on a fast desktop (measured in Chromium; see `paintFitCopy`). An earlier version awaited `img.decode()` first; review measured that
   * Chromium's draw repeats the decode anyway, so the wait only left the fit view blank and
   * unclickable, and was removed.
   *
   * Everything else (small images, formats that may animate, an undecoded source, a browser that
   * will not give a 2D context or fails the draw) shows the original `<img>` in both modes, as
   * before. The one change every image gets is in app.css: no rounded corners or shadow at actual
   * size.
   */
  import { paintFitCopy, planFitCopy, releaseCanvas, viewerPixelBox, type PixelSize } from "./lightbox-image.ts";

  type Props = {
    /** The address the embed already loads from; the original shown at actual size. */
    url: string;
    /** Accessible name for whichever element is showing. */
    alt: string;
    /** The decoded `<img>` the viewer was opened from. Its pixels are what the copy is drawn
     *  from; null when the opener had no element, which simply means no copy. */
    source: HTMLImageElement | null;
    /** The file index's declared MIME, or "" when the file is not in the index. */
    mime: string;
    /** True for actual size, false for fit to window. */
    zoomed: boolean;
    /** Click on the picture: the host flips `zoomed`. */
    ontoggle: () => void;
  };
  let { url, alt, source, mime, zoomed, ontoggle }: Props = $props();

  /**
   * Bumped by the source's `load` and `error` events. `complete` and the natural size are plain DOM
   * properties that Svelte cannot track, so without this an image opened while still loading kept
   * the original's fit path for the whole opening, however long ago it had loaded (found in
   * review). The listeners go with the component, so a load that lands after the viewer closed
   * (Escape, another image, the vault lock) reaches nothing.
   */
  let sourceTick = $state(0);
  $effect(() => {
    const img = source;
    if (!img || img.complete) return;
    const bump = () => {
      sourceTick++;
    };
    img.addEventListener("load", bump);
    img.addEventListener("error", bump);
    return () => {
      img.removeEventListener("load", bump);
      img.removeEventListener("error", bump);
    };
  });

  /**
   * The first plan this opening made, kept for the rest of it. `mime` follows the live file index,
   * and an owner can unlist and relist a file while it is open here; re-planning on every such
   * change dropped the copy (back to the original's fit path) and resampled it again on relisting
   * (found in review). An index that arrives late still gets its one chance: nothing is kept until
   * a plan exists. A plain variable rather than state, because it only ever goes from null to set.
   * Writing it inside `$derived.by` relies on deriveds not being evaluated speculatively; that
   * holds while `svelte.config.js` leaves `experimental.async` off, and needs a second look if it
   * is turned on.
   */
  let kept: PixelSize | null = null;

  /** The copy's size, or null to show the original in both modes. */
  const plan = $derived.by((): PixelSize | null => {
    if (kept) return kept;
    void sourceTick; // re-run when the source finishes loading or fails
    // `complete` with a zero natural size means a broken image; planFitCopy refuses zero sizes.
    if (!source || !source.complete) return null;
    const box = viewerPixelBox(
      { width: window.screen?.width ?? 0, height: window.screen?.height ?? 0 },
      { width: window.innerWidth, height: window.innerHeight },
      window.devicePixelRatio,
    );
    kept = planFitCopy({ mime, naturalWidth: source.naturalWidth, naturalHeight: source.naturalHeight, box });
    return kept;
  });

  /** Set when painting the copy failed; the original then takes over the fit view. */
  let failed = $state(false);
  const showCopy = $derived(plan !== null && !failed);

  /**
   * Paint once, synchronously, when the canvas mounts, so the copy has pixels in the same flush
   * that shows it; free its buffer when it unmounts. There is deliberately no `update`: the host
   * remounts this component for a new image, and within one opening the plan, once made, never
   * changes (`kept`), so the canvas mounts at most once.
   */
  function fitCopy(canvas: HTMLCanvasElement, job: { source: HTMLImageElement; size: PixelSize }) {
    if (!paintFitCopy(canvas, job.source, job.size)) failed = true;
    return {
      destroy() {
        releaseCanvas(canvas);
      },
    };
  }
</script>

<!-- The button is named after the image, so its accessible name is the same whichever element is
     showing: the copy is a canvas, which has no alt, and is hidden from assistive tech. -->
<button
  class="lightbox-img"
  type="button"
  aria-label={alt}
  title={zoomed ? "Click to fit to the window" : "Click to view at actual size"}
  onclick={ontoggle}
>
  {#if showCopy && plan && source}
    <canvas class="lightbox-fit" aria-hidden="true" hidden={zoomed} use:fitCopy={{ source, size: plan }}></canvas>
  {/if}
  <img src={url} {alt} hidden={showCopy && !zoomed} />
</button>
