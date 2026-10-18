import { decodePix, pixToRgba } from "./pix.ts";
import { mayFetchPixa, pixaFrameAt, PixaFileReader } from "./pixa-media.ts";
import type { Pixa } from "./pixa.ts";

export type PixaPreviewFile = Readonly<{ cid: string; name: string; size: number }>;
export type PixaPreviewScope = Readonly<{ server: number; channel: string; generation: number; membership: number; surface: string }>;
type AudioState = Readonly<{ state: string; elapsedMs: number }>;
type Card = {
  host: HTMLElement; canvas: HTMLCanvasElement; status: HTMLElement; play: HTMLButtonElement;
  sound: HTMLButtonElement; load: HTMLButtonElement; token: object; file: PixaPreviewFile;
  scope: PixaPreviewScope; visible: boolean; manual: boolean; generation: number; audioGeneration: number;
  value: Pixa | null; loading: boolean; running: boolean; origin: number; held: number; painted: number;
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
  now?: () => number;
};

/**
 * One owner for every rich-text/file PIXA card in an App window. Only two visible packages
 * may be retained, one native read runs at a time, and there is one repaint timer. A message
 * containing hundreds of embeds therefore cannot start hundreds of downloads or synths.
 * DOM hosts are built from code; no peer-controlled HTML, URL or executable media is admitted.
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
    const canvas = document.createElement("canvas"); canvas.width = 192; canvas.height = 144;
    canvas.setAttribute("aria-label", file.name); canvas.hidden = true;
    const status = document.createElement("p"); status.className = "pixa-preview-status";
    const caption = document.createElement("figcaption"); caption.textContent = file.name;
    const controls = document.createElement("div"); controls.className = "pixa-preview-controls";
    const button = (text: string) => { const value = document.createElement("button"); value.type = "button"; value.textContent = text; value.className = "ghost small"; controls.append(value); return value; };
    const load = button("Play flipnote"), play = button("Pause"), sound = button("Play sound");
    play.hidden = true; sound.hidden = true;
    const card: Card = { host, canvas, status, play, sound, load, token: {}, file: { ...file }, scope: { ...scope },
      visible: false, manual: explicit, generation: 0, audioGeneration: 0, value: null, loading: false, running: true, origin: 0, held: 0, painted: -1 };
    load.onclick = () => {
      if (this.options.renewScope) {
        const next = this.options.renewScope(card.scope, card.file);
        if (!next || next.server !== card.scope.server || next.channel !== card.scope.channel) { status.textContent = "Open this file’s channel to play it."; return; }
        // reset() already retired old reads/audio. Renew only on this explicit click, without
        // restoring the old package or audio consent across a membership/session edge.
        card.scope = { ...next };
      }
      card.manual = true; this.register(card, true); card.visible = true; this.admit(card);
    };
    play.onclick = () => {
      if (!card.value) return;
      if (card.running) { card.audioGeneration++; card.held = Math.min(this.elapsed(card), card.value.frames.length * 1000 / card.value.fps - 1); card.running = false; this.options.stopAudio(card.token); }
      else { card.origin = this.now(); card.running = true; }
      play.textContent = card.running ? "Pause" : "Play"; this.arm();
    };
    sound.onclick = () => {
      const value = card.value, generation = card.generation, audioGeneration = ++card.audioGeneration;
      if (!value || !this.valid(card)) return;
      if (this.options.audioState(card.token)) { this.options.stopAudio(card.token); sound.textContent = "Play sound"; return; }
      status.textContent = ""; // a fresh attempt replaces the previous failure, not its caption
      card.held = 0; card.origin = this.now(); card.running = true; play.textContent = "Pause";
      // Call synchronously in this gesture; an async download or observer never resumes audio.
      void this.options.startAudio(card.token, value, card.scope).then(started => {
        if (generation !== card.generation || audioGeneration !== card.audioGeneration || !this.valid(card)) return;
        if (!started) status.textContent = "Sound unavailable. Check Deafen and try again.";
      }, () => { if (generation === card.generation && audioGeneration === card.audioGeneration && this.valid(card)) status.textContent = "Couldn’t play sound. Try again."; });
    };
    host.append(canvas, status, caption, controls);
    this.register(card, explicit);
    if (!mayFetchPixa(file.size)) { status.textContent = "This flipnote is too large to preview."; load.disabled = true; }
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
    this.active.add(card); card.running = true; card.held = 0; card.painted = -1;
    card.status.textContent = "Loading flipnote…"; card.load.hidden = true;
    void this.pump();
  }
  private release(card: Card): void {
    if (this.reading === card) this.options.reader.cancel();
    card.generation++; card.audioGeneration++; card.value = null; card.loading = false;
    this.options.stopAudio(card.token); this.active.delete(card);
    card.canvas.hidden = true; card.play.hidden = true; card.sound.hidden = true; card.load.hidden = false;
    if (mayFetchPixa(card.file.size)) card.status.textContent = "";
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
            card.value = value; card.origin = this.now(); card.held = 0; card.painted = -1;
            card.status.textContent = ""; card.canvas.hidden = false; card.play.hidden = false;
            card.sound.hidden = !value.sfx.length && !value.take.events.length;
            // Canvas has no native load event. Announce the one layout growth through the
            // chat's existing capture listener, which pins only readers already at the tail.
            card.host.dispatchEvent(new window.Event("load"));
            this.arm();
          }
        } catch {
          if (current()) { this.release(card); card.status.textContent = "Couldn’t load this flipnote. Try again."; }
        } finally { if (this.reading === card) this.reading = null; }
      }
    } finally { this.pumping = false; }
  }
  private elapsed(card: Card): number {
    const audio = this.options.audioState(card.token);
    if (audio?.state === "playing" || audio?.state === "preparing") return audio.elapsedMs;
    const value = card.value!;
    return (card.held + (card.running ? Math.max(0, this.now() - card.origin) : 0)) % (value.frames.length * 1000 / value.fps);
  }
  private arm(): void {
    if (this.timer || this.disposed || !this.active.size) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      for (const card of this.active) {
        if (!this.valid(card) || !card.visible || document.hidden) { this.release(card); continue; }
        if (!card.value) continue;
        const frame = pixaFrameAt(card.value, this.elapsed(card));
        if (frame !== card.painted) {
          const image = decodePix(card.value.frames[frame]);
          card.canvas.getContext("2d")?.putImageData(new ImageData(pixToRgba(image), image.w, image.h), 0, 0);
          card.canvas.dataset.pixaFrame = String(frame); card.painted = frame;
        }
        card.sound.textContent = this.options.audioState(card.token) ? "Mute" : "Play sound";
      }
      if ([...this.active].some(card => !!card.value && card.running)) this.arm();
    }, 40);
  }
}
