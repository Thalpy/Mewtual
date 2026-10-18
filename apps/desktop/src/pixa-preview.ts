import { decodePix, pixToRgba } from "./pix.ts";
import { mayFetchPixa, pixaFrameAt, PixaFileReader } from "./pixa-media.ts";
import type { Pixa } from "./pixa.ts";

export type PixaPreviewFile = Readonly<{ cid: string; name: string; size: number }>;
export type PixaPreviewScope = Readonly<{ server: number; channel: string; generation: number; membership: number; surface: string }>;
type AudioState = Readonly<{ state: string; elapsedMs: number }>;
/**
 * The card's presentational phase, mirrored onto `figure[data-state]` so the stylesheet can draw
 * the dashed idle border, the info-tinted fetch veil and the warn-tinted error sentence. It is
 * cosmetic only: every bound and lifecycle decision reads the Card fields, never this attribute.
 */
type CardState = "idle" | "loading" | "ready" | "error" | "blocked";
/**
 * A transport-strip button: an inline SVG glyph plus a visually hidden text label. The label span
 * is the only text node inside the button, so `button.textContent` is exactly the action name
 * ("Pause", "Play sound", "Open"). Tests, assistive technology and the hover tooltip all read that
 * text; the glyph is `aria-hidden` and contributes none, which is what lets the icon change without
 * the accessible name drifting away from the action.
 */
type IconButton = { button: HTMLButtonElement; label: HTMLSpanElement; glyph: SVGPathElement };
type Card = {
  host: HTMLElement; canvas: HTMLCanvasElement; status: HTMLElement; load: HTMLButtonElement;
  // Transport strip pieces. `fill` is the progress bar's inner <i>; `strip` hides as one unit.
  strip: HTMLElement; play: IconButton; sound: IconButton; frame: HTMLElement; fill: HTMLElement; time: HTMLElement;
  // Facts line pieces that change once the package is known.
  facts: HTMLElement; sounds: HTMLElement;
  token: object; file: PixaPreviewFile;
  scope: PixaPreviewScope; visible: boolean; manual: boolean; generation: number; audioGeneration: number;
  value: Pixa | null; loading: boolean; running: boolean; origin: number; held: number; painted: number;
  /** The tenth of a second the elapsed readout and progress bar last showed; -1 forces a write. */
  tenth: number;
};
export type PixaPreviewOptions = {
  reader: Pick<PixaFileReader, "read" | "cancel">;
  current: (scope: PixaPreviewScope, file: PixaPreviewFile) => boolean;
  autoLoad: (scope: PixaPreviewScope, file: PixaPreviewFile) => boolean;
  /** A fresh gesture may renew membership/session observation for the same channel's file.
   * Returning null refuses it. Never silently rebind a retained host to another server. */
  renewScope?: (previous: PixaPreviewScope, file: PixaPreviewFile) => PixaPreviewScope | null;
  startAudio: (token: object, value: Pixa, scope: PixaPreviewScope) => Promise<boolean>;
  stopAudio: (token: object) => void;
  audioState: (token: object) => AudioState | null;
  /**
   * Opens the file in the host's viewer. The card hands over its own scope (the one it was created
   * or last renewed under), not whatever the host considers current, so the host can compare the
   * two and refuse a card left behind by an earlier channel, membership or session instead of
   * opening a stale reference under the present one. When absent the card shows no open controls.
   */
  open?: (file: PixaPreviewFile, scope: PixaPreviewScope) => void;
  /**
   * Host wording for the file's expiry ("expires 30d") on the facts line. Evaluated once when the
   * card is built, which is also when the host re-renders rich text, so it cannot go stale for
   * longer than the message it sits in. An empty string omits the fact.
   */
  expiryText?: (file: PixaPreviewFile) => string;
  now?: () => number;
};

const SVG_NS = "http://www.w3.org/2000/svg";
/**
 * Glyph outlines on a 16 x 16 grid: filled shapes for the transport, stroked outlines for the
 * secondary actions, matching the approved mockup. These are fixed strings from code; nothing
 * peer-controlled ever reaches an attribute (the file name is only ever written as text).
 */
const GLYPH = {
  pause: "M3.5 2.5h3.5v11H3.5zM9 2.5h3.5v11H9z",
  play: "M4.5 2.5l9 5.5-9 5.5z",
  sound: "M6 12.5V4l6-1.5V11M2.7 12.5a1.8 1.8 0 1 0 3.6 0a1.8 1.8 0 1 0-3.6 0M8.7 11a1.8 1.8 0 1 0 3.6 0a1.8 1.8 0 1 0-3.6 0",
  open: "M3 9v4h4M13 7V3H9M13 3 9 7M3 13l4-4",
} as const;

/** Sizes use the Studio's lowercase kib/mib style so the facts line matches the editor's. */
function formatSize(bytes: number): string {
  return bytes >= 1024 * 1024 ? `${(bytes / (1024 * 1024)).toFixed(1)} mib` : `${(bytes / 1024).toFixed(1)} kib`;
}
/** One silent loop of the package, in milliseconds. Timing and progress are both measured in it. */
function loopMs(value: Pixa): number { return value.frames.length * 1000 / value.fps; }
/** Build a strip button. `outline` selects the stroked glyph style; see IconButton for the label. */
function iconButton(text: string, glyph: string, outline: boolean): IconButton {
  const button = document.createElement("button"); button.type = "button"; button.className = outline ? "pixa-tb ol" : "pixa-tb";
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 16 16"); svg.setAttribute("aria-hidden", "true"); svg.setAttribute("focusable", "false");
  const path = document.createElementNS(SVG_NS, "path"); path.setAttribute("d", glyph); svg.append(path);
  const label = document.createElement("span"); label.className = "pixa-sr";
  button.append(svg, label);
  const target = { button, label, glyph: path }; relabel(target, text); return target;
}
/**
 * Change a strip button's name and, optionally, its glyph. Writes only on an actual change: the
 * repaint timer calls this every 40 ms for the sound button, and rewriting identical text would
 * still invalidate layout and spam attribute mutations into the chat's MutationObserver.
 */
function relabel(target: IconButton, text: string, glyph?: string): void {
  if (target.label.textContent !== text) { target.label.textContent = text; target.button.title = text; }
  if (glyph && target.glyph.getAttribute("d") !== glyph) target.glyph.setAttribute("d", glyph);
}
/** A facts-line fact in the mono micro style. */
function micro(text: string, className = "micro"): HTMLSpanElement {
  const span = document.createElement("span"); span.className = className; span.textContent = text; return span;
}

/**
 * One owner for every rich-text/file PIXA card in an App window. Only two visible packages
 * may be retained, one native read runs at a time, and there is one repaint timer. A message
 * containing hundreds of embeds therefore cannot start hundreds of downloads or synths.
 * DOM hosts are built from code; no peer-controlled HTML, URL or executable media is admitted.
 *
 * Each host is a `figure.pixa-preview[data-state]` holding a `div.pixa-stage` (the pixels, the
 * veil text, the manual load button and the overlaid transport strip) and a `figcaption.pixa-facts`
 * line. The stylesheet's `.pixa-*` rules and the integrator's lookups depend on that shape; the
 * tests pin the button names and the status sentences.
 */
export class PixaPreviewManager {
  private readonly cards = new Set<Card>();
  private readonly cardsByHost = new WeakMap<Element, Card>();
  private readonly active = new Set<Card>();
  private readonly options: PixaPreviewOptions;
  private readonly now: () => number;
  private readonly intersection: IntersectionObserver;
  private readonly mutation: MutationObserver;
  private reading: Card | null = null;
  private pumping = false;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private disposed = false;

  constructor(options: PixaPreviewOptions) {
    this.options = options; this.now = options.now ?? (() => performance.now());
    this.intersection = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const card = this.cardsByHost.get(entry.target);
        if (!card) continue;
        card.visible = entry.isIntersecting;
        if (card.visible && !document.hidden && (card.manual || options.autoLoad(card.scope, card.file))) this.admit(card);
        else if (!card.visible) this.release(card);
      }
    });
    // Rich-text replacement does not run Svelte component teardown for these imperative hosts.
    // Observe child changes only; painting and frame attributes never cause recursive cleanup.
    this.mutation = new MutationObserver(() => this.prune());
    this.mutation.observe(document.body, { childList: true, subtree: true });
    document.addEventListener("visibilitychange", this.visibility);
  }

  create(file: PixaPreviewFile, scope: PixaPreviewScope, explicit = false): HTMLElement {
    const host = document.createElement("figure"); host.className = "pixa-preview";
    host.setAttribute("aria-label", `Flipnote: ${file.name}`);
    // The stage: pixels at 1.5x with the veil centred over them and the transport strip pinned to
    // the bottom edge. The veil (status sentence + manual load button) is ordinary flow content so
    // the two stack without a wrapper; the stylesheet lifts the canvas and the strip out of flow.
    const stage = document.createElement("div"); stage.className = "pixa-stage";
    const canvas = document.createElement("canvas"); canvas.width = 192; canvas.height = 144;
    canvas.setAttribute("aria-label", file.name); canvas.hidden = true;
    const status = document.createElement("p"); status.className = "pixa-preview-status";
    const load = document.createElement("button"); load.type = "button"; load.className = "ghost small pixa-load"; load.textContent = "Play flipnote";
    // Transport strip: play/pause, "frame / frames", progress, elapsed, sound, and (when the host
    // can open files) an open button. Hidden as a unit until a package has arrived, so a card that
    // is idle, fetching or errored offers only the veil's affordances.
    const strip = document.createElement("div"); strip.className = "pixa-tr"; strip.hidden = true;
    const play = iconButton("Pause", GLYPH.pause, false), sound = iconButton("Play sound", GLYPH.sound, true);
    const frame = document.createElement("span"); frame.className = "pixa-frame";
    const progress = document.createElement("span"); progress.className = "pixa-prog";
    const fill = document.createElement("i"); progress.append(fill);
    const time = document.createElement("span"); time.className = "pixa-time";
    sound.button.hidden = true;
    strip.append(play.button, frame, progress, time, sound.button);
    // Facts line: name, then "frames · fps · size" (just the size until the package is known), then
    // the sound facts (hidden until known), the host's expiry wording, and an "open" text link.
    // The name keeps its real case: it is peer-chosen, and uppercasing it in CSS could make two
    // different names look alike.
    const caption = document.createElement("figcaption"); caption.className = "pixa-facts";
    const name = micro(file.name, "pixa-name");
    const facts = micro(formatSize(file.size));
    const sounds = micro("", "micro pixa-sounds"); sounds.hidden = true;
    caption.append(name, facts, sounds);
    const expiry = this.options.expiryText?.(file) ?? "";
    if (expiry) caption.append(micro(expiry));
    const card: Card = { host, canvas, status, load, strip, play, sound, frame, fill, time, facts, sounds, token: {}, file: { ...file }, scope: { ...scope },
      visible: false, manual: explicit, generation: 0, audioGeneration: 0, value: null, loading: false, running: true, origin: 0, held: 0, painted: -1, tenth: -1 };
    if (this.options.open) {
      // Both open affordances share one handler and hand over this card's scope, which may have
      // been renewed by a later explicit Play; see PixaPreviewOptions.open for why the host wants it.
      const open = () => this.options.open?.(card.file, card.scope);
      const icon = iconButton("Open", GLYPH.open, true); icon.button.onclick = open; strip.append(icon.button);
      const link = document.createElement("button"); link.type = "button"; link.className = "pixa-link"; link.textContent = "open"; link.onclick = open;
      caption.append(link);
    }
    load.onclick = () => {
      if (this.options.renewScope) {
        const next = this.options.renewScope(card.scope, card.file);
        if (!next || next.server !== card.scope.server || next.channel !== card.scope.channel) {
          status.textContent = "Open this file’s channel to play it."; this.phase(card, "blocked"); return;
        }
        // reset() already retired old reads/audio. Renew only on this explicit click, without
        // restoring the old package or audio consent across a membership/session edge.
        card.scope = { ...next };
      }
      card.manual = true; this.register(card, true); card.visible = true; this.admit(card);
    };
    play.button.onclick = () => {
      if (!card.value) return;
      if (card.running) { card.audioGeneration++; card.held = Math.min(this.elapsed(card), loopMs(card.value) - 1); card.running = false; this.options.stopAudio(card.token); }
      else { card.origin = this.now(); card.running = true; }
      this.showRunning(card); this.arm();
    };
    sound.button.onclick = () => {
      const value = card.value, generation = card.generation, audioGeneration = ++card.audioGeneration;
      if (!value || !this.valid(card)) return;
      if (this.options.audioState(card.token)) { this.options.stopAudio(card.token); this.showSound(card, false); return; }
      status.textContent = ""; // a fresh attempt replaces the previous failure, not its caption
      card.held = 0; card.origin = this.now(); card.running = true; this.showRunning(card);
      // Call synchronously in this gesture; an async download or observer never resumes audio.
      void this.options.startAudio(card.token, value, card.scope).then(started => {
        if (generation !== card.generation || audioGeneration !== card.audioGeneration || !this.valid(card)) return;
        if (!started) status.textContent = "Sound unavailable. Check Deafen and try again.";
      }, () => { if (generation === card.generation && audioGeneration === card.audioGeneration && this.valid(card)) status.textContent = "Couldn’t play sound. Try again."; });
    };
    stage.append(canvas, status, load, strip); host.append(stage, caption);
    this.register(card, explicit);
    if (mayFetchPixa(file.size)) this.phase(card, "idle");
    else { status.textContent = "This flipnote is too large to preview."; load.disabled = true; this.phase(card, "blocked"); }
    return host;
  }

  /** A navigation/trust/session edge revokes packages and consent, including pending reads. */
  reset(): void {
    this.options.reader.cancel();
    for (const card of this.cards) { this.release(card); card.manual = false; }
    this.prune();
  }
  reconcile(): void {
    this.prune();
    for (const card of this.cards) {
      if (!this.valid(card) || document.hidden || !card.visible || (!card.manual && !this.options.autoLoad(card.scope, card.file))) this.release(card);
    }
  }
  destroy(): void {
    this.disposed = true; this.reset(); this.intersection.disconnect(); this.mutation.disconnect();
    document.removeEventListener("visibilitychange", this.visibility); this.cards.clear();
  }
  private readonly visibility = () => {
    if (document.hidden) this.reset();
    else for (const card of this.cards) if (card.visible && this.options.autoLoad(card.scope, card.file)) this.admit(card);
  };
  private valid(card: Card): boolean { return !this.disposed && card.host.isConnected && this.options.current(card.scope, card.file); }
  private phase(card: Card, state: CardState): void { card.host.dataset.state = state; }
  /**
   * Mirror running/paused onto the play button and the figure. The stylesheet may fade the strip
   * only while the loop runs unhovered; `data-paused` keeps it on screen so a paused card never
   * hides its own Play button from a pointer user who has moved away.
   */
  private showRunning(card: Card): void {
    relabel(card.play, card.running ? "Pause" : "Play", card.running ? GLYPH.pause : GLYPH.play);
    if (card.running) delete card.host.dataset.paused; else card.host.dataset.paused = "true";
  }
  /** Mirror whether this card owns the audio session onto the sound button's name and tint. */
  private showSound(card: Card, sounding: boolean): void {
    relabel(card.sound, sounding ? "Mute" : "Play sound");
    if (card.sound.button.classList.contains("on") !== sounding) card.sound.button.classList.toggle("on", sounding);
  }
  /**
   * Push the strip's readouts for the frame now on the canvas and the loop position `elapsed`.
   * The counter follows the painted frame, so it never claims a frame the pixels do not show. The
   * elapsed text and progress width move at most ten times a second: the timer ticks every 40 ms
   * and a bar that changed on every tick would relayout the whole chat column 25 times a second
   * for a change no one can see.
   */
  private readout(card: Card, value: Pixa, elapsed: number): void {
    const frame = card.painted < 0 ? 0 : card.painted;
    const counter = `${frame + 1} / ${value.frames.length}`;
    if (card.frame.textContent !== counter) card.frame.textContent = counter;
    const tenth = Math.floor(elapsed / 100);
    if (tenth !== card.tenth) {
      card.tenth = tenth; card.time.textContent = `${(tenth / 10).toFixed(1)} s`;
      // Audio-driven elapsed may run past the loop while the last frame holds; clamp the bar there.
      card.fill.style.width = `${Math.min(100, elapsed * 100 / loopMs(value)).toFixed(1)}%`;
    }
  }
  /** Fill the facts line from the decoded package. Content behind a cid is immutable, so these
   * facts stay true after the package is released and need no reset. */
  private describe(card: Card, value: Pixa): void {
    card.facts.textContent = `${value.frames.length} frames · ${value.fps} fps · ${formatSize(card.file.size)}`;
    const parts: string[] = [];
    if (value.sfx.length) parts.push(`♪ ${value.sfx.length} sounds`);
    if (value.take.events.length) parts.push("♫ music");
    card.sounds.textContent = parts.join(" · "); card.sounds.hidden = !parts.length;
  }
  private register(card: Card, explicit: boolean): void {
    if (this.cards.has(card)) return;
    // The existing message window bounds DOM, but a hostile body can contain many markers.
    // Keep observer/lookup work independently bounded; extra cards remain explicit-load buttons.
    if (this.cards.size >= 128) {
      if (!explicit) return;
      const victim = [...this.cards].find(value => !this.active.has(value)) ?? this.cards.values().next().value!;
      this.release(victim); this.intersection.unobserve(victim.host); this.cards.delete(victim); this.cardsByHost.delete(victim.host);
    }
    this.cards.add(card); this.cardsByHost.set(card.host, card); this.intersection.observe(card.host);
  }
  private admit(card: Card): void {
    if (!this.valid(card) || !card.visible || document.hidden || !mayFetchPixa(card.file.size)) return;
    if (this.active.has(card)) return;
    while (this.active.size >= 2) this.release(this.active.values().next().value!);
    this.active.add(card); card.running = true; card.held = 0; card.painted = -1; card.tenth = -1;
    // The veil names the declared size being fetched: the read is bounded by it, and the sentence
    // tells the reader what a slow fetch is waiting on.
    card.status.textContent = `fetching ${formatSize(card.file.size)}`; card.load.hidden = true; this.phase(card, "loading");
    void this.pump();
  }
  private release(card: Card): void {
    if (this.reading === card) this.options.reader.cancel();
    card.generation++; card.audioGeneration++; card.value = null; card.loading = false;
    this.options.stopAudio(card.token); this.active.delete(card);
    card.canvas.hidden = true; card.strip.hidden = true; card.sound.button.hidden = true; card.load.hidden = false;
    delete card.host.dataset.paused;
    // A too-large card keeps its sentence and its blocked phase; anything else returns to idle.
    if (mayFetchPixa(card.file.size)) { card.status.textContent = ""; this.phase(card, "idle"); }
    if (!this.active.size && this.timer) { clearTimeout(this.timer); this.timer = null; }
  }
  private prune(): void {
    for (const card of this.cards) if (!card.host.isConnected) {
      this.release(card); this.intersection.unobserve(card.host); this.cards.delete(card);
      this.cardsByHost.delete(card.host);
    }
  }
  private async pump(): Promise<void> {
    if (this.pumping || this.disposed) return;
    this.pumping = true;
    try {
      for (;;) {
        const card = [...this.active].find(value => !value.value && !value.loading);
        if (!card) break;
        const generation = card.generation; card.loading = true; this.reading = card;
        const current = () => generation === card.generation && this.active.has(card) && this.valid(card) && card.visible && !document.hidden;
        try {
          const value = await this.options.reader.read(card.scope.server, card.file.cid, card.file.size, current);
          if (current() && value) {
            card.value = value; card.origin = this.now(); card.held = 0; card.painted = -1; card.tenth = -1;
            card.status.textContent = ""; card.canvas.hidden = false; card.strip.hidden = false;
            card.sound.button.hidden = !value.sfx.length && !value.take.events.length;
            // Prime the strip at the loop's start so it never shows a previous package's numbers
            // during the 40 ms before the first paint; the timer corrects it from then on.
            this.showRunning(card); this.showSound(card, false); this.readout(card, value, 0); this.describe(card, value);
            this.phase(card, "ready");
            // Canvas has no native load event. Announce the one layout growth through the
            // chat's existing capture listener, which pins only readers already at the tail.
            card.host.dispatchEvent(new window.Event("load"));
            this.arm();
          }
        } catch {
          if (current()) { this.release(card); card.status.textContent = "Couldn’t load this flipnote. Try again."; this.phase(card, "error"); }
        } finally { if (this.reading === card) this.reading = null; }
      }
    } finally { this.pumping = false; }
  }
  private elapsed(card: Card): number {
    const audio = this.options.audioState(card.token);
    if (audio?.state === "playing" || audio?.state === "preparing") return audio.elapsedMs % loopMs(card.value!);
    const value = card.value!;
    return (card.held + (card.running ? Math.max(0, this.now() - card.origin) : 0)) % loopMs(value);
  }
  private arm(): void {
    if (this.timer || this.disposed || !this.active.size) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      for (const card of this.active) {
        if (!this.valid(card) || !card.visible || document.hidden) { this.release(card); continue; }
        if (!card.value) continue;
        const elapsed = this.elapsed(card), frame = pixaFrameAt(card.value, elapsed);
        if (frame !== card.painted) {
          const image = decodePix(card.value.frames[frame]);
          card.canvas.getContext("2d")?.putImageData(new ImageData(pixToRgba(image), image.w, image.h), 0, 0);
          card.canvas.dataset.pixaFrame = String(frame); card.painted = frame;
        }
        // Readouts after the paint so the counter reports the frame now on screen. While paused,
        // elapsed() returns the held position, so nothing here changes and nothing is written.
        this.readout(card, card.value, elapsed);
        this.showSound(card, !!this.options.audioState(card.token));
      }
      if ([...this.active].some(card => !!card.value && card.running)) this.arm();
    }, 40);
  }
}
