<script lang="ts">
  /**
   * The global flipnote viewer: a modal overlay opened from Files, chat and wiki file references.
   *
   * The host (App.svelte) owns the package read, the audio session and the lifecycle edges that
   * close the viewer (navigation, lock, Deafen, leaving the channel). This component owns only
   * presentation and the "where is the picture" question, which has two answers because sound
   * and silence have different clocks:
   *
   * - SOUND ON: pressing Play asks the host to start audio from the held frame's time. While the
   *   host reports "playing", the canvas follows `audioState.elapsedMs`, so the picture can never
   *   drift from the synth that is playing its markers. The host also owns looping in this mode.
   * - SOUND OFF (toggle, Deafen, or a package with nothing to hear): a local timer chain advances
   *   elapsed time from the injected `now()` clock. Nothing is sent to the audio host, so a
   *   deafened member can still watch the animation.
   *
   * Stepping (first/prev/next/last, thumbnails, arrow keys) always stops whichever source is
   * running and holds the chosen frame: a picture that keeps moving under a "go to frame 3"
   * request would make the request meaningless, and for the sound source the audio would keep
   * playing markers for frames that are no longer on screen.
   */
  import { onDestroy, untrack } from "svelte";
  import type { Pixa } from "./pixa.ts";
  import { pixaFrameAt } from "./pixa-media.ts";
  import { decodePix, pixToRgba } from "./pix.ts";
  import type { StudioAudioState } from "./studio-audio.ts";
  import PixaCanvas from "./PixaCanvas.svelte";
  import {
    PIXA_VIEWER_THUMB_LIMIT, PIXA_VIEWER_TICK_MS, clampFrame, formatSeconds, frameOffsetMs, frameProbeMs,
    isSilentPixa, packageDurationMs, sfxCountByFrame, viewerFacts, viewerProvenance, visualElapsedAt,
  } from "./pixa-viewer.ts";

  /** The host/viewer contract. Every callback is host-owned; see the integration notes in the handoff. */
  type Props = {
    /** File name shown as the title and used as the canvas label. */
    name: string;
    /** The decoded, validated package; null while the host is still reading it. */
    pixa: Pixa | null;
    /** Non-empty when the read or validation failed; replaces the canvas with an alert. */
    error?: string;
    /** Listing size in bytes for the facts line; 0 when unknown. */
    bytes?: number;
    /** Provenance for the head row; each is omitted when empty. */
    author?: string;
    place?: string;
    expiry?: string;
    /** The host's audio session state; followed only while this viewer armed the sound source. */
    audioState: StudioAudioState;
    /** Sound cannot start (call Deafen). The visual source still works. */
    deafened?: boolean;
    /** Start host audio and frames from `offsetMs`; resolves false when it could not start. */
    onplay: (offsetMs: number, loop: boolean) => Promise<boolean>;
    /** Stop host audio. Called whenever this viewer leaves the sound source while it is armed. */
    onstop: () => void;
    /** Download the original bytes; the button is hidden when absent. */
    ondownload?: () => void;
    /** Close request (scrim, Escape, close buttons). The host unmounts and stops audio. */
    onclose: () => void;
    /** Clock for the visual-only loop; injectable so tests can step it deterministically. */
    now?: () => number;
  };
  let {
    name, pixa = null, error = "", bytes = 0, author = "", place = "", expiry = "", audioState, deafened = false,
    onplay, onstop, ondownload, onclose, now = () => performance.now(),
  }: Props = $props();

  // --- Local state ---------------------------------------------------------------------------------
  let card = $state<HTMLDivElement | null>(null);
  /** The frame shown while nothing is running, and the start of the next run. */
  let heldFrame = $state(0);
  /** Loop defaults on, like the inline previews; a short flipnote that stops dead feels broken. */
  let loop = $state(true);
  /** The user's sound preference; the effective source also honours Deafen and silent packages. */
  let soundOn = $state(true);
  /**
   * Set when this viewer asked the host to start audio, cleared when it stops it or the host
   * reports a stop. Gating on it means host audio this viewer did not start (nothing should be
   * playing when the host opens the viewer, but the state is shared) can never show as "playing".
   */
  let soundArmed = $state(false);
  /** Between pressing Play and `onplay` resolving; the status chip says "preparing". */
  let soundPending = $state(false);
  /** Inline explanation after a refused sound start; cleared by the next Play. */
  let soundNotice = $state("");
  /** The visual-only loop's live position and whether its timer chain is running. */
  let visualRunning = $state(false);
  let visualElapsedMs = $state(0);
  /** A non-loop run that reached its last frame: the next Play restarts from the first frame. */
  let ended = $state(false);

  // Plain (non-reactive) bookkeeping read only from handlers, the timer and untracked effects.
  let visualRun: { baseMs: number; originMs: number } | null = null;
  let visualTimer: ReturnType<typeof setTimeout> | null = null;
  /** Bumped by every stop/start so a late `onplay` resolution cannot act on a newer state. */
  let playGeneration = 0;
  /** The last frame the sound source showed; what the picture holds when the host stops it. */
  let lastSoundFrame = 0;
  let soundWasRunning = false;
  let seenPixa: Pixa | null = null;

  // --- Derived view ---------------------------------------------------------------------------------
  const frameCount = $derived(pixa?.frames.length ?? 0);
  const durationMs = $derived(pixa ? packageDurationMs(pixa) : 0);
  const silent = $derived(pixa !== null && isSilentPixa(pixa));
  const soundAvailable = $derived(!deafened && !silent);
  const source = $derived<"sound" | "visual">(soundOn && soundAvailable ? "sound" : "visual");
  /** The sound source counts as running from the Play press until it, or the host, stops it. */
  const soundRunning = $derived(source === "sound" && soundArmed &&
    (soundPending || audioState.state === "playing" || audioState.state === "preparing"));
  /**
   * Follow the host clock only once it reports "playing". While "preparing" the session reports
   * elapsed 0 (it resets on stop), so following it would flash the first frame before an offset
   * start; the held frame is the truthful picture until sound actually begins.
   */
  const audioLive = $derived(soundRunning && audioState.state === "playing");
  const running = $derived(soundRunning || visualRunning);
  const preparing = $derived(soundRunning && !audioLive);
  const displayedFrame = $derived(!pixa ? 0
    : audioLive ? pixaFrameAt(pixa, audioState.elapsedMs, loop)
    : visualRunning ? pixaFrameAt(pixa, visualElapsedMs)
    : clampFrame(heldFrame, frameCount));
  /** What drives `PixaCanvas`; a held frame is probed at its midpoint (see `frameProbeMs`). */
  const canvasElapsedMs = $derived(!pixa ? 0
    : audioLive ? audioState.elapsedMs
    : visualRunning ? visualElapsedMs
    : frameProbeMs(displayedFrame, pixa.fps));
  /**
   * Readout time agrees with `displayedFrame`: under a looping run the pictures wrap on their own
   * clock even while the host's audio runs on through a long sound tail (`pixaFrameAt` with
   * `loop`), so the readout wraps too; a one-shot run holds the end and the readout clamps.
   */
  const readoutMs = $derived(!pixa ? 0
    : audioLive ? (loop ? audioState.elapsedMs % durationMs : Math.min(audioState.elapsedMs, durationMs))
    : visualRunning ? visualElapsedMs
    : frameOffsetMs(displayedFrame, pixa.fps));
  const sfxByFrame = $derived(sfxCountByFrame(pixa));
  const sfxHere = $derived(sfxByFrame.get(displayedFrame) ?? 0);
  /** See `PIXA_VIEWER_THUMB_LIMIT`: above the cap no strip is shown rather than a partial one. */
  const thumbs = $derived(pixa && pixa.frames.length <= PIXA_VIEWER_THUMB_LIMIT ? pixa.frames : []);
  const facts = $derived(viewerFacts(pixa, bytes));
  const provenance = $derived(viewerProvenance(author, place, expiry));
  const status = $derived(preparing ? "preparing" : running ? "playing" : "paused");

  // --- Lifecycle ------------------------------------------------------------------------------------
  $effect(() => {
    // The dialog takes focus so Escape, Space and the arrow keys work without a click first.
    card?.focus();
  });
  $effect(() => {
    // A different package (or the package arriving after the loading state) starts at frame 0
    // with nothing running. Guarded by identity: prop updates that leave `pixa` untouched (an
    // audio tick, a Deafen flip) must not reset the picture or stop a run.
    const value = pixa;
    untrack(() => {
      if (value === seenPixa) return;
      seenPixa = value;
      if (visualRunning) stopVisual();
      if (soundArmed) stopSound();
      heldFrame = 0; ended = false; soundNotice = "";
    });
  });
  $effect(() => {
    // Track the frame the sound source shows, and hold it when that source stops for a reason
    // other than this viewer's own request: the host reaching the end with loop off, the engine
    // refusing or ending, or the source flipping to visual (Deafen, package change). A stop this
    // viewer requested already held its target frame and cleared `soundWasRunning` first, so it
    // cannot be overwritten here with the frame the audio happened to be on.
    const live = soundRunning;
    const frame = live ? displayedFrame : -1;
    const following = audioLive;
    untrack(() => {
      if (live) {
        lastSoundFrame = frame; soundWasRunning = true;
        // The host reporting "playing" is proof the start succeeded, so the promise's answer is
        // redundant from here: a host stop that lands before `onplay` resolves (a very short
        // track) must end the run and hold the audio's last frame, not wait on the promise.
        if (following) soundPending = false;
        return;
      }
      if (!soundWasRunning) return;
      soundWasRunning = false; soundArmed = false; soundPending = false; playGeneration++;
      heldFrame = lastSoundFrame;
      // A host that reached the last frame without looping is a natural end; any other stop
      // (engine, scope) keeps the held frame as the resume point.
      ended = !loop && lastSoundFrame === frameCount - 1;
      // Leaving the sound source while the host still plays (Deafen flipped under a run, or a
      // package swap) must silence it, or the UI would say "paused" over audible playback.
      if (audioState.state !== "stopped") onstop();
    });
  });
  onDestroy(() => {
    // The host stops its audio when it closes the viewer; this only drops the local timer and
    // disowns any `onplay` still in flight so its late answer cannot touch a dead component.
    stopVisual(); playGeneration++;
  });

  // --- Playback control -----------------------------------------------------------------------------
  /** Stop whatever is driving the picture and hold `frame`. Every step and pause goes through here. */
  function holdAt(frame: number) {
    const target = clampFrame(frame, frameCount);
    if (visualRunning) stopVisual();
    if (soundArmed) stopSound();
    heldFrame = target; ended = false;
  }
  function stopSound() {
    playGeneration++; soundWasRunning = false; soundArmed = false; soundPending = false;
    onstop();
  }
  function stopVisual() {
    if (visualTimer !== null) { clearTimeout(visualTimer); visualTimer = null; }
    visualRunning = false; visualRun = null;
  }
  function play() {
    if (!pixa || running) return;
    soundNotice = "";
    // After a non-loop run finished, Play means "again", like every player; a held frame chosen
    // by stepping is a deliberate resume point and is kept.
    if (ended) { heldFrame = 0; ended = false; }
    if (source === "sound") void playSound(pixa); else playVisual(pixa);
  }
  async function playSound(value: Pixa) {
    const generation = ++playGeneration;
    soundArmed = true; soundPending = true;
    let started = false;
    try { started = await onplay(frameOffsetMs(clampFrame(heldFrame, value.frames.length), value.fps), loop); }
    catch { started = false; }
    // A pause, step, close or package change while the host was preparing already settled the
    // picture; the late answer belongs to a run that no longer exists.
    if (generation !== playGeneration) return;
    soundPending = false;
    if (!started) {
      soundWasRunning = false; soundArmed = false;
      soundNotice = "Sound could not start. Press Play to try again.";
    }
  }
  function playVisual(value: Pixa) {
    visualRun = { baseMs: frameOffsetMs(clampFrame(heldFrame, value.frames.length), value.fps), originMs: now() };
    visualElapsedMs = visualRun.baseMs; visualRunning = true;
    armVisual();
  }
  function armVisual() {
    if (visualTimer !== null) return;
    visualTimer = setTimeout(tickVisual, PIXA_VIEWER_TICK_MS);
  }
  function tickVisual() {
    visualTimer = null;
    const value = pixa, run = visualRun;
    if (!visualRunning || !value || !run) return;
    // `loop` is read live so flipping it mid-run takes effect on the next tick.
    const position = visualElapsedAt({ ...run, loop }, now(), packageDurationMs(value));
    if (position.ended) { stopVisual(); heldFrame = value.frames.length - 1; ended = true; return; }
    visualElapsedMs = position.elapsedMs;
    armVisual();
  }
  function togglePlay() {
    if (!pixa) return;
    if (running) holdAt(displayedFrame); else play();
  }
  function step(delta: number) { if (pixa) holdAt(displayedFrame + delta); }
  function first() { if (pixa) holdAt(0); }
  function last() { if (pixa) holdAt(frameCount - 1); }
  /** Switching sources mid-run stops the current one (silencing the host) and holds the frame. */
  function toggleSound() {
    if (!soundAvailable) return;
    if (running) holdAt(displayedFrame);
    soundOn = !soundOn;
  }
  function onKey(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const interactive = !!target && (target.tagName === "BUTTON" || target.tagName === "INPUT" ||
      target.tagName === "TEXTAREA" || !!target.isContentEditable);
    switch (e.key) {
      case "Escape": e.preventDefault(); e.stopPropagation(); onclose(); return;
      case "ArrowLeft": step(-1); break;
      case "ArrowRight": step(1); break;
      case "Home": first(); break;
      case "End": last(); break;
      case " ":
        // A focused button activates on Space natively; toggling play as well would make, for
        // example, Space on "next frame" both step and start playback.
        if (interactive) { e.stopPropagation(); return; }
        togglePlay(); break;
      default: return;
    }
    // Handled keys stay inside the dialog: the app's own shortcuts must not also see them.
    e.preventDefault(); e.stopPropagation();
  }

  // --- Thumbnails -----------------------------------------------------------------------------------
  let scratch: HTMLCanvasElement | null = null;
  /**
   * Action: decode one frame into a 64×48 tile. Frames were validated by `decodePixa`, so a decode
   * failure here cannot happen for an admitted package; the guard only keeps a blank tile from
   * taking the whole strip down. One shared scratch canvas holds the full-size raster so the
   * downscale goes through `drawImage` with smoothing off (pixel art must stay crisp).
   */
  function thumb(node: HTMLCanvasElement, frame: Uint8Array) {
    const draw = (bytes: Uint8Array) => {
      const ctx = node.getContext("2d");
      if (!ctx) return;
      ctx.imageSmoothingEnabled = false;
      try {
        const image = decodePix(bytes);
        if (!scratch) scratch = document.createElement("canvas");
        scratch.width = image.w; scratch.height = image.h;
        const stage = scratch.getContext("2d");
        if (!stage) return;
        stage.putImageData(new ImageData(pixToRgba(image), image.w, image.h), 0, 0);
        ctx.drawImage(scratch, 0, 0, node.width, node.height);
      } catch { /* leave the tile blank; the frame number still identifies it */ }
    };
    draw(frame);
    return { update: draw };
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
<div class="pixa-viewer" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }} onkeydown={onKey}>
  <div class="card" role="dialog" aria-modal="true" aria-label="Flipnote viewer" tabindex="-1" bind:this={card}>
    <header class="head">
      <svg class="film" viewBox="0 0 16 16" aria-hidden="true"><rect x="1.5" y="3" width="13" height="10" rx="1"></rect><path d="M4.5 3v10M11.5 3v10M1.5 6.5h3M11.5 6.5h3M1.5 9.5h3M11.5 9.5h3"></path></svg>
      <h2 class="title" title={name}>{name}</h2>
      <span class="micro facts" title={facts}>{facts}</span>
      {#if pixa}<span class="chip ok" title="Read once and validated in full before anything plays"><i></i><span class="micro">validated</span></span>{/if}
      <span class="grow"></span>
      {#if provenance}<span class="micro prov" title={provenance}>{provenance}</span>{/if}
      <button type="button" class="st-btn ghost x" aria-label="Close flipnote viewer" title="Close (Esc)" onclick={() => onclose()}><svg viewBox="0 0 16 16" class="ol" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8"></path></svg></button>
    </header>

    <div class="body">
      {#if error}
        <p class="note alert" role="alert">{error}</p>
      {:else if pixa}
        <div class="stage">
          <PixaCanvas {pixa} elapsedMs={canvasElapsedMs} {loop} label={name} />
          {#if sfxHere}<span class="micro sfx-chip" title={`${sfxHere} sound marker${sfxHere === 1 ? "" : "s"} on this frame`}>♪ {sfxHere}</span>{/if}
        </div>
      {:else}
        <p class="note" role="status">Loading and validating animation…</p>
      {/if}
    </div>

    <div class="transport">
      <button type="button" class="st-tool sm" aria-label="First frame" title="First frame (Home)" disabled={!pixa} onclick={first}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M8 3 2 8l6 5zM14 3 8 8l6 5z"></path></svg></button>
      <button type="button" class="st-tool sm" aria-label="Previous frame" title="Previous frame (Left)" disabled={!pixa} onclick={() => step(-1)}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3 3h1.5v10H3zM12.5 3 6 8l6.5 5z"></path></svg></button>
      <!-- The visible text is exactly "Play" or "Pause": scripts and screen readers rely on it. -->
      <button type="button" class="st-tool play" class:on={running} disabled={!pixa} title={running ? "Pause (Space)" : "Play (Space)"} onclick={togglePlay}>{#if running}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3.5 2.5h3.5v11H3.5zM9 2.5h3.5v11H9z"></path></svg>{:else}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M4 2.5 13 8 4 13.5z"></path></svg>{/if}<span class="vh">{running ? "Pause" : "Play"}</span></button>
      <button type="button" class="st-tool sm" aria-label="Next frame" title="Next frame (Right)" disabled={!pixa} onclick={() => step(1)}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3.5 3 10 8l-6.5 5zM11.5 3H13v10h-1.5z"></path></svg></button>
      <button type="button" class="st-tool sm" aria-label="Last frame" title="Last frame (End)" disabled={!pixa} onclick={last}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M2 3l6 5-6 5zM8 3l6 5-6 5z"></path></svg></button>
      <!-- Loop is fixed for the length of a run: the host received it at start and the two sources must agree. -->
      <button type="button" class="st-tool sm ol" class:on={loop} aria-pressed={loop} disabled={!pixa || running} title={loop ? "Loop: on (stop to change)" : "Loop: off (stop to change)"} onclick={() => (loop = !loop)}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8a5 5 0 015-5h4M13 8a5 5 0 01-5 5H4"></path><path d="M10.5 1.5 12 3l-1.5 1.5M5.5 11.5 4 13l1.5 1.5"></path></svg><span class="vh">Loop</span></button>
      <button type="button" class="st-tool sm ol" class:on={source === "sound"} aria-pressed={source === "sound"} disabled={!pixa || !soundAvailable}
        title={deafened ? "Sound: off while deafened" : silent ? "Sound: this flipnote has no sound" : source === "sound" ? "Sound: on" : "Sound: off (picture only)"}
        onclick={toggleSound}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 12.5V4l6-1.5V11"></path><circle cx="4.5" cy="12.5" r="1.8"></circle><circle cx="10.5" cy="11" r="1.8"></circle></svg><span class="vh">Sound</span></button>
      <span class="mono strong" aria-live="off">{pixa ? displayedFrame + 1 : "·"} / {frameCount}</span>
      <span class="micro">{formatSeconds(readoutMs)} s of {formatSeconds(durationMs)} s</span>
      <span class="grow"></span>
      <span class="chip" class:info={running} role="status"><i></i><span class="micro">{status}</span></span>
    </div>
    {#if deafened || soundNotice}
      <p class="hint" role="status">{deafened ? "Turn off Deafen to hear it." : soundNotice}</p>
    {/if}

    {#if thumbs.length}
      <div class="strip" aria-label="Frames">
        {#each thumbs as frame, i (i)}
          <button type="button" class="thumb" class:cur={i === displayedFrame} aria-label={`Frame ${i + 1}`} aria-current={i === displayedFrame ? "true" : undefined} title={`Frame ${i + 1}`} onclick={() => holdAt(i)}>
            <canvas width="64" height="48" use:thumb={frame}></canvas>
            {#if sfxByFrame.get(i)}<span class="pill" aria-hidden="true">♪</span>{/if}
          </button>
        {/each}
      </div>
    {/if}

    <footer class="foot">
      <span class="grow"></span>
      {#if ondownload}<button type="button" class="st-btn" onclick={() => ondownload?.()}>download .pixa</button>{/if}
      <button type="button" class="st-btn ghost" onclick={() => onclose()}>close</button>
    </footer>
  </div>
</div>

<style>
  /* Overlay and card. z-index 60 sits above the app's ordinary overlays (50) and below toasts. */
  .pixa-viewer { position: fixed; inset: 0; z-index: 60; display: flex; align-items: center; justify-content: center; padding: 16px; background: color-mix(in oklab, var(--bg-0) 55%, transparent); color: var(--text); font-family: var(--ui); font-size: 0.84rem; }
  .card { display: flex; flex-direction: column; width: min(680px, 100%); height: min(600px, 100%); background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); box-shadow: 0 18px 44px color-mix(in oklab, var(--bg-0) 60%, transparent); overflow: hidden; outline: none; }
  .card:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
  .grow { flex: 1; }

  /* Operator-terminal grammar shared with the Studio (see Studio.svelte). */
  .micro { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .mono { font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); white-space: nowrap; }
  .mono.strong { color: var(--text); }
  .chip { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 2px 9px; white-space: nowrap; flex: none; }
  .chip i { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); }
  .chip.ok { border-color: var(--ok-brd); background: var(--ok-dim); }
  .chip.ok .micro { color: var(--ok); }
  .chip.ok i { background: var(--ok); }
  .chip.info { border-color: color-mix(in oklab, var(--info) 40%, var(--panel)); }
  .chip.info .micro { color: var(--info); }
  .chip.info i { background: var(--info); }
  .st-btn { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.06em; text-transform: uppercase; white-space: nowrap; cursor: pointer; }
  .st-btn.ghost { background: transparent; border-color: var(--border-soft); color: var(--muted); }
  .st-btn:disabled { opacity: 0.45; cursor: default; }
  .st-btn.x { width: 26px; padding: 0; justify-content: center; flex: none; }
  .st-btn.x svg { width: 12px; height: 12px; }
  .st-tool { width: 26px; height: 26px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); display: inline-flex; align-items: center; justify-content: center; position: relative; padding: 0; flex: none; cursor: pointer; }
  .st-tool svg { width: 11px; height: 11px; }
  .st-tool.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-tool:disabled { opacity: 0.4; cursor: default; }
  .st-tool.play { width: 34px; height: 30px; border-radius: 999px; background: var(--accent); border-color: var(--accent); color: var(--on-accent); margin: 0 4px; }
  .st-tool.play svg { width: 14px; height: 14px; }
  .st-tool.play.on { background: var(--accent-dim); color: var(--accent-hi); }
  svg.solid { fill: currentColor; stroke: none; }
  .st-tool.ol svg, svg.ol { fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; width: 14px; height: 14px; }

  /* Head row */
  .head { display: flex; align-items: center; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--border); min-width: 0; }
  .film { width: 14px; height: 14px; flex: none; fill: none; stroke: var(--accent); stroke-width: 1.5; stroke-linejoin: round; }
  .title { margin: 0; font-size: 0.95rem; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; flex: 0 1 auto; }
  .facts, .prov { overflow: hidden; text-overflow: ellipsis; min-width: 0; flex: 0 1 auto; }

  /* Picture. The stage is sized from the body's height so the pixel grid fills what is left
     after the chrome, capped at 512×384 (4:3 of the 192×144 source at an integer-ish scale). */
  .body { flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; padding: 12px; background: var(--bg-0); }
  .stage { position: relative; height: 100%; max-height: 384px; max-width: 100%; aspect-ratio: 4 / 3; border: 1px solid var(--border); line-height: 0; background: var(--bg-0); }
  .stage :global(canvas) { image-rendering: pixelated; }
  .sfx-chip { position: absolute; right: 8px; top: 6px; padding: 2px 5px; border-radius: 3px; background: color-mix(in oklab, var(--bg-0) 70%, transparent); color: var(--accent-hi); line-height: 1.4; pointer-events: none; }
  .note { margin: 0; color: var(--muted); text-align: center; }
  .note.alert { color: var(--warn); }

  /* Transport */
  .transport { display: flex; align-items: center; gap: 8px; padding: 8px 14px; border-top: 1px solid var(--border); flex-wrap: wrap; }
  .hint { margin: 0; padding: 0 14px 8px; font-size: 0.74rem; color: var(--muted); }

  /* Thumbnail strip; the outline needs the top padding so the current tile's ring is not clipped. */
  .strip { display: flex; gap: 6px; align-items: flex-start; padding: 3px 14px 8px; overflow-x: auto; scrollbar-gutter: stable; }
  .thumb { position: relative; flex: none; width: 64px; height: 48px; padding: 0; border: 1px solid var(--border); border-radius: 3px; overflow: hidden; background: var(--bg-0); line-height: 0; color: inherit; cursor: pointer; }
  .thumb canvas { display: block; width: 64px; height: 48px; image-rendering: pixelated; }
  .thumb.cur { outline: 2px solid var(--accent); outline-offset: 1px; }
  .pill { position: absolute; left: 3px; bottom: 3px; height: 10px; padding: 0 4px; border-radius: 999px; background: var(--accent); color: var(--on-accent); font-family: var(--mono); font-size: 0.46rem; line-height: 10px; }

  /* Footer */
  .foot { display: flex; align-items: center; gap: 8px; padding: 8px 14px; border-top: 1px solid var(--border); background: var(--bg-elev); }

  /* Visually hidden button text: present for scripts and assistive tech, invisible on screen. */
  .vh { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip: rect(0 0 0 0); clip-path: inset(50%); white-space: nowrap; border: 0; }
</style>
