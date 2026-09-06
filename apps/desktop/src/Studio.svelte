<script lang="ts">
  // The flipnote editor surface (design-creative-suite.md 2.2, 2.10; mockups "Flipnote Editor",
  // "Timeline States", "Art / Sound / Music"). Everything on screen runs against the in-memory
  // studio store; the shared behaviours (save/load through P1, claims on the draw channel,
  // settlement and recovery actions, blob fetch, .pix/.pixa export) are the boundary and say so
  // where a button would otherwise pretend.
  import { onDestroy, onMount } from "svelte";
  import { decodePix, pixToRgba } from "./pix.ts";
  import {
    CLEAR,
    PixRaster,
    UndoStack,
    brushCells,
    clampBrush,
    eraseValue,
    floodFill,
    mirrored,
    paintDot,
    paintShape,
    paintStroke,
    pressureSize,
    shapeFilled,
    shapeOutline,
    stampBitmap,
    textBitmap,
    type Bitmap,
    type Cell,
    type Mirror,
    type Shape,
    type Tool,
  } from "./pix-canvas.ts";
  import {
    BRUSH_MAX,
    BRUSH_MIN,
    CLAIM_RESEND_MS,
    FLIPNOTE_FPS_MAX,
    FLIPNOTE_FPS_MIN,
    FLIPNOTE_FRAME_BYTES_PROMISE,
    FLIPNOTE_MAX_FRAMES,
    FLIPNOTE_MAX_PATCHES,
    LAYER_NAMES,
    PIX_ROLE_NAMES,
    randomElementId,
    type PixPaletteEntry,
  } from "./studio-contract.ts";
  import { DEFAULT_PALETTE, PALETTE_LABELS, StudioError } from "./studio-store.ts";
  import { bump, ensureStudio, fixtureColor, fixtureName, studio } from "./studio-state.svelte.ts";

  let { me, nameOf, colorOf, adapt = true, onnotice } = $props<{
    me: string;
    nameOf: (id: string) => string;
    colorOf: (id: string) => string;
    adapt?: boolean;
    onnotice: (text: string, kind: "info" | "warn" | "error") => void;
  }>();

  // Built once per mount (state may be written during init, never inside a derived); the
  // surface remounts with the tab, and the identity is settled by the time the tab can open.
  // svelte-ignore state_referenced_locally
  const store = ensureStudio(me);
  const objectId = $derived(studio.selected);
  const root = $derived.by(() => { void studio.rev; return store.objects.get(objectId) ?? null; });
  const isScore = $derived.by(() => { void studio.rev; return store.index.objects[objectId]?.kind === "score"; });
  const settlement = $derived.by(() => { void studio.rev; return store.settlement.get(objectId) ?? null; });
  const recovery = $derived.by(() => { void studio.rev; return store.recovery.get(objectId) ?? null; });
  const overCap = $derived.by(() => { void studio.rev; return root ? store.overCap(root) : new Set<string>(); });

  function who(id: string): string { return id === me ? "you" : fixtureName(id) ?? nameOf(id); }
  function tint(id: string): string { return id === me ? "var(--accent)" : fixtureColor(id) ?? colorOf(id); }

  // --- Editor state ------------------------------------------------------------------------------
  let frameId = $state("");
  let raster = $state.raw<PixRaster | null>(null);
  const undo = new UndoStack();
  let tool = $state<Tool>("pen");
  let shape = $state<Shape>("ellipse");
  let shapeFill = $state(false);
  let brush = $state(3);
  // Pen pressure drives the brush size (the chosen size is the maximum). Never in the document:
  // a frame is still just pixels, so a viewer without a tablet sees exactly what was drawn.
  let pressureOn = $state(true);
  let penSeen = $state(false);
  let mirror = $state<Mirror>({ h: false, v: false });
  let color = $state(1); // palette index
  let layer = $state(2);
  let layerVisible = $state([true, true, true]);
  let onion = $state(true);
  let grid = $state(true);
  // The prop seeds the toggle; the surface then owns it (a per-viewer preference, 2.1).
  // svelte-ignore state_referenced_locally
  let adaptOn = $state(adapt);
  let inspectorTab = $state<"art" | "sound" | "music">("art");
  let stampIdx = $state(0);
  let textDraft = $state("nya~");
  let playing = $state(false);
  let loop = $state(true);
  const ZOOM_MIN = 1, ZOOM_MAX = 6;
  let zoom = $state(3);
  function setZoom(z: number) { zoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, Math.round(z))); }
  let dirty = $state(false);
  let tick = $state(0); // 1 Hz for countdowns
  let paintRev = $state(0); // bumped after every raster edit so overlays repaint

  // A copy, re-read on every store revision: inserts splice the root's list in place, and a
  // derived that returned the same array reference would never tell the timeline.
  const frames = $derived.by(() => { void studio.rev; return [...(root?.frames ?? [])]; });
  const frameIndex = $derived(frames.indexOf(frameId));
  const frameRec = $derived(root && frameId ? root.frame[frameId] : null);
  const claim = $derived.by(() => { void studio.rev; void tick; return frameId ? store.claimOn(frameId) : null; });
  const claimLeft = $derived.by(() => { void tick; void studio.rev; return frameId ? store.claimSecondsLeft(frameId) : 0; });
  const conflict = $derived.by(() => { void studio.rev; return frameId ? store.conflicts.get(frameId) ?? null : null; });
  const frameState = $derived.by(() => { void studio.rev; return frameId ? store.frameState.get(frameId) ?? "held" : "held"; });
  const sfxHere = $derived.by(() => { void studio.rev; return root ? Object.entries(root.sfx).filter(([, s]) => s.fr === frameId) : []; });
  const allSfx = $derived.by(() => { void studio.rev; return root ? Object.entries(root.sfx).sort((a, b) => frames.indexOf(a[1].fr) - frames.indexOf(b[1].fr)) : []; });
  const totalBytes = $derived.by(() => { void studio.rev; return root ? store.frameBytesTotal(root) : 0; });
  const scoreTitle = $derived.by(() => { void studio.rev; return root?.score ? store.index.objects[root.score]?.title ?? "" : ""; });
  const patchCount = $derived.by(() => { void studio.rev; return root ? Object.keys(root.patches).length : 0; });
  const canEdit = $derived(!!root && !isScore && overCap.size === 0 && frameState === "held" && settlement?.gate !== "fault");

  // Built-in stamps (the emoji/ folder's pix stamps are the real source once C1 lands): palette
  // indices straight from the default palette, CLEAR where the stamp leaves the frame alone.
  const STAMPS: { name: string; bm: Bitmap }[] = [
    { name: "heart", bm: bits(8, [".##..##.", "########", "########", ".######.", "..####..", "...##...", "........", "........"], 8) },
    { name: "star", bm: bits(8, ["...#....", "...#....", "..###...", "#######.", ".#####..", "..###...", ".##.##..", "#.....#."], 9) },
    { name: "paw", bm: bits(8, [".#..#...", "#..#..#.", ".....#..", "..###...", ".#####..", ".#####..", "..###...", "........"], 1) },
    { name: "moon", bm: bits(8, ["...###..", "..##....", ".##.....", ".##.....", ".##.....", ".##.....", "..##....", "...###.."], 1) },
    { name: "note", bm: bits(8, ["....##..", "....#.#.", "....#...", "....#...", "....#...", "..###...", ".####...", "..##...."], 4) },
  ];
  function bits(w: number, rows: string[], value: number): Bitmap {
    const pixels = new Uint8Array(w * rows.length).fill(CLEAR);
    rows.forEach((r, y) => { for (let x = 0; x < w; x++) if (r[x] === "#") pixels[y * w + x] = value; });
    return { w, h: rows.length, pixels };
  }

  // --- Theme adaptation: role entries take the viewer's tokens ---------------------------------
  function cssRgb(name: string): [number, number, number] | null {
    const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    const m = /^#([0-9a-f]{6})$/i.exec(v);
    if (!m) return null;
    const n = parseInt(m[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  let roleLut = $state.raw<Record<number, [number, number, number]>>({});
  function refreshRoles() {
    const bg = cssRgb("--bg-0");
    const out: Record<number, [number, number, number]> = {};
    const set = (role: number, c: [number, number, number] | null) => { if (c) out[role] = c; };
    set(1, bg);
    set(2, cssRgb("--text"));
    set(3, cssRgb("--accent"));
    set(4, cssRgb("--muted"));
    set(5, bg ? [Math.round(bg[0] * 0.6), Math.round(bg[1] * 0.6), Math.round(bg[2] * 0.6)] : null);
    set(6, cssRgb("--border"));
    set(7, cssRgb("--faint"));
    set(8, cssRgb("--text-2"));
    roleLut = out;
  }
  function resolve(e: PixPaletteEntry): readonly [number, number, number] {
    if (adaptOn && e.role !== 0) { const c = roleLut[e.role]; if (c) return c; }
    return [e.r, e.g, e.b];
  }
  function wellCss(e: PixPaletteEntry): string {
    const [r, g, b] = resolve(e);
    return `rgb(${r}, ${g}, ${b})`;
  }

  // --- Frame open / commit --------------------------------------------------------------------
  function openFrame(id: string, force = false) {
    if (!root) return;
    commit();
    if (frameId && frameId !== id) store.releaseClaim(frameId);
    const other = store.claimOn(id);
    if (other && other.by !== me && !force) {
      // Advisory: opening warns, never blocks. The frame card offers "open anyway".
      pendingOpen = id;
      frameId = id;
      raster = null;
      return;
    }
    pendingOpen = "";
    frameId = id;
    const bytes = store.frameBytes(objectId, id);
    const r = new PixRaster(root.w, root.h, (bytes ? decodePix(bytes).palette : DEFAULT_PALETTE.map((e) => ({ ...e }))));
    if (bytes) r.loadFlat(decodePix(bytes).pixels);
    raster = r;
    undo.clear();
    dirty = false;
    if (!other) store.claim(id, me);
    bump();
    paintRev++;
  }
  let pendingOpen = $state("");

  /// Write the raster back as a replace_frame op if anything changed (2.9: an op only when a
  /// value changed; bursts are coalesced by committing on stroke end, not per cell).
  function commit() {
    if (!raster || !frameId || !dirty || !root) return;
    try {
      const changed = store.replaceFrame(objectId, frameId, raster.encode());
      if (changed) store.renewClaim(frameId);
    } catch (e) {
      onnotice(e instanceof StudioError ? e.reason : String(e), "warn");
    }
    dirty = false;
    bump();
  }

  function edited(changedCells: number) {
    if (!changedCells) return;
    dirty = true;
    paintRev++;
  }

  // --- Pointer input on the canvas ----------------------------------------------------------------
  let rootEl = $state<HTMLDivElement | null>(null);
  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let overlayEl = $state<HTMLCanvasElement | null>(null);
  let hover = $state<Cell | null>(null);
  let anchor: Cell | null = null;
  let last: Cell | null = null;
  let stroking = false;
  let previewCells = $state.raw<Cell[]>([]);

  function cellAt(e: PointerEvent): Cell | null {
    if (!canvasEl || !root) return null;
    const rect = canvasEl.getBoundingClientRect();
    const x = Math.floor(((e.clientX - rect.left) / rect.width) * root.w);
    const y = Math.floor(((e.clientY - rect.top) / rect.height) * root.h);
    return [x, y];
  }

  function paintValue(): number {
    return tool === "eraser" && raster ? eraseValue(raster, layer) : color;
  }

  function sizeFor(e: PointerEvent): number {
    if (e.pointerType === "pen") penSeen = true;
    return pressureSize(brush, e.pressure, e.pointerType, pressureOn);
  }

  function onDown(e: PointerEvent) {
    if (!raster || !canEdit || e.button === 2) return;
    const c = cellAt(e);
    if (!c) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
    if (tool === "pen" || tool === "eraser") {
      undo.push(raster);
      stroking = true;
      last = c;
      edited(paintDot(raster, layer, c[0], c[1], paintValue(), sizeFor(e), mirror));
    } else if (tool === "fill") {
      undo.push(raster);
      edited(floodFill(raster, layer, c[0], c[1], color));
      commit();
    } else if (tool === "shape") {
      anchor = c;
      previewCells = [];
    } else if (tool === "stamp") {
      undo.push(raster);
      const bm = STAMPS[stampIdx].bm;
      edited(stampBitmap(raster, layer, c[0] - (bm.w >> 1), c[1] - (bm.h >> 1), bm, mirror));
      commit();
    } else if (tool === "text") {
      undo.push(raster);
      const bm = textBitmap(textDraft, color);
      edited(stampBitmap(raster, layer, c[0], c[1], bm, mirror));
      commit();
    }
  }

  function onMove(e: PointerEvent) {
    const c = cellAt(e);
    hover = c;
    if (!raster || !c) return;
    if (stroking && last && (tool === "pen" || tool === "eraser")) {
      // The samples the browser coalesced since the last event, each with its own pressure,
      // so a fast pen stroke stays continuous and a lightening touch thins along the way.
      const samples = typeof e.getCoalescedEvents === "function" && e.getCoalescedEvents().length ? e.getCoalescedEvents() : [e];
      let changed = 0;
      for (const s of samples) {
        const sc = cellAt(s);
        if (!sc) continue;
        changed += paintStroke(raster, layer, last[0], last[1], sc[0], sc[1], paintValue(), sizeFor(s), mirror);
        last = sc;
      }
      edited(changed);
    } else if (anchor && tool === "shape") {
      const cells = shapeFill ? shapeFilled(shape, anchor[0], anchor[1], c[0], c[1]) : shapeOutline(shape, anchor[0], anchor[1], c[0], c[1]);
      previewCells = mirrored(cells, raster.w, raster.h, mirror);
    }
  }

  function onUp(e: PointerEvent) {
    if (!raster) return;
    const c = cellAt(e);
    if (stroking) {
      stroking = false;
      last = null;
      commit();
    } else if (anchor && tool === "shape" && c) {
      undo.push(raster);
      edited(paintShape(raster, layer, shape, anchor[0], anchor[1], c[0], c[1], color, brush, shapeFill, mirror));
      anchor = null;
      previewCells = [];
      commit();
    }
  }

  function onLeave() {
    hover = null;
    if (stroking) { stroking = false; last = null; commit(); }
  }

  function doUndo() { if (raster && undo.undo(raster)) { dirty = true; paintRev++; commit(); } }
  function doRedo() { if (raster && undo.redo(raster)) { dirty = true; paintRev++; commit(); } }

  // --- Rendering ---------------------------------------------------------------------------------
  let scratch: HTMLCanvasElement | null = null;
  function scratchCanvas(w: number, h: number): CanvasRenderingContext2D {
    if (!scratch) scratch = document.createElement("canvas");
    scratch.width = w;
    scratch.height = h;
    return scratch.getContext("2d")!;
  }

  function drawFrame() {
    if (!canvasEl || !root) return;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;
    const { w, h } = root;
    canvasEl.width = w * zoom;
    canvasEl.height = h * zoom;
    ctx.imageSmoothingEnabled = false;
    const bgc = resolve(raster?.palette[raster.bgIndex()] ?? DEFAULT_PALETTE[0]);
    ctx.fillStyle = `rgb(${bgc[0]}, ${bgc[1]}, ${bgc[2]})`;
    ctx.fillRect(0, 0, canvasEl.width, canvasEl.height);
    if (raster) {
      // Respect hidden layers in the view only: the frame's bytes always flatten everything.
      const flat = new Uint8Array(raster.w * raster.h);
      const base = raster.layers[0];
      flat.set(layerVisible[0] ? base : new Uint8Array(base.length).fill(raster.bgIndex()));
      for (let l = 1; l < raster.layers.length; l++) {
        if (!layerVisible[l]) continue;
        const src = raster.layers[l];
        for (let i = 0; i < src.length; i++) if (src[i] !== CLEAR) flat[i] = src[i];
      }
      const s = scratchCanvas(raster.w, raster.h);
      s.putImageData(new ImageData(pixToRgba({ w: raster.w, h: raster.h, palette: raster.palette, pixels: flat }, resolve), raster.w, raster.h), 0, 0);
      ctx.drawImage(scratch!, 0, 0, canvasEl.width, canvasEl.height);
    }
    // Onion: the previous frame ghosted OVER the current one (one frame back in v1), with its
    // paper cells cut out so only its marks show through. Under the frame it would be hidden by
    // the current frame's opaque paper.
    if (onion && frameIndex > 0) {
      const prev = store.frameBytes(objectId, frames[frameIndex - 1]);
      if (prev) {
        const img = decodePix(prev);
        const rgba = pixToRgba(img, resolve);
        const paper = 0;
        for (let i = 0; i < img.pixels.length; i++) if (img.pixels[i] === paper) rgba[i * 4 + 3] = 0;
        const s = scratchCanvas(img.w, img.h);
        s.putImageData(new ImageData(rgba, img.w, img.h), 0, 0);
        ctx.globalAlpha = 0.3;
        ctx.drawImage(scratch!, 0, 0, canvasEl.width, canvasEl.height);
        ctx.globalAlpha = 1;
      }
    }
  }

  function drawOverlay() {
    if (!overlayEl || !root) return;
    const ctx = overlayEl.getContext("2d");
    if (!ctx) return;
    const { w, h } = root;
    const z = zoom;
    overlayEl.width = w * z;
    overlayEl.height = h * z;
    ctx.clearRect(0, 0, overlayEl.width, overlayEl.height);
    const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() || "#977df2";
    const ink = "#000000";
    // The cell grid needs at least three screen pixels per cell to read; the 4th-cell grid always.
    if (grid) {
      ctx.lineWidth = 1;
      if (z >= 3) {
        ctx.strokeStyle = ink;
        ctx.globalAlpha = 0.1;
        ctx.beginPath();
        for (let x = 1; x < w; x++) { ctx.moveTo(x * z + 0.5, 0); ctx.lineTo(x * z + 0.5, h * z); }
        for (let y = 1; y < h; y++) { ctx.moveTo(0, y * z + 0.5); ctx.lineTo(w * z, y * z + 0.5); }
        ctx.stroke();
      }
      ctx.strokeStyle = accent;
      ctx.globalAlpha = 0.3;
      ctx.beginPath();
      for (let x = 4; x < w; x += 4) { ctx.moveTo(x * z + 0.5, 0); ctx.lineTo(x * z + 0.5, h * z); }
      for (let y = 4; y < h; y += 4) { ctx.moveTo(0, y * z + 0.5); ctx.lineTo(w * z, y * z + 0.5); }
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
    ctx.fillStyle = accent;
    ctx.globalAlpha = 0.55;
    for (const [x, y] of previewCells) if (x >= 0 && y >= 0 && x < w && y < h) ctx.fillRect(x * z, y * z, z, z);
    ctx.globalAlpha = 1;
    if (hover && canEdit && (tool === "pen" || tool === "eraser" || tool === "shape")) {
      ctx.strokeStyle = accent;
      ctx.lineWidth = 1;
      for (const [x, y] of mirrored(brushCells(hover[0], hover[1], tool === "shape" ? 1 : brush), w, h, mirror)) {
        if (x >= 0 && y >= 0 && x < w && y < h) ctx.strokeRect(x * z + 0.5, y * z + 0.5, z - 1, z - 1);
      }
    } else if (hover && canEdit && tool === "stamp") {
      const bm = STAMPS[stampIdx].bm;
      ctx.strokeStyle = accent;
      ctx.strokeRect((hover[0] - (bm.w >> 1)) * z + 0.5, (hover[1] - (bm.h >> 1)) * z + 0.5, bm.w * z, bm.h * z);
    }
  }

  $effect(() => {
    void paintRev; void onion; void adaptOn; void roleLut; void layerVisible; void frameIndex; void studio.rev; void zoom;
    drawFrame();
  });
  $effect(() => {
    void grid; void hover; void previewCells; void brush; void mirror; void tool; void stampIdx; void canEdit; void zoom;
    drawOverlay();
  });
  $effect(() => {
    void adaptOn;
    refreshRoles();
  });
  $effect(() => {
    // A new object, or a frame list that no longer holds the open frame: open the first frame.
    if (root && (!frameId || !root.frames.includes(frameId))) {
      const first = root.frames[0];
      if (first) openFrame(first);
      else { frameId = ""; raster = null; }
    }
  });

  /// Thumbnail action: decode the frame's bytes into a small canvas.
  function thumb(node: HTMLCanvasElement, cid: string) {
    const draw = (c: string) => {
      const bytes = store.blobs.get(c);
      const ctx = node.getContext("2d");
      if (!ctx) return;
      ctx.imageSmoothingEnabled = false;
      ctx.clearRect(0, 0, node.width, node.height);
      if (!bytes) return;
      const img = decodePix(bytes);
      const s = scratchCanvas(img.w, img.h);
      s.putImageData(new ImageData(pixToRgba(img, resolve), img.w, img.h), 0, 0);
      ctx.drawImage(scratch!, 0, 0, node.width, node.height);
    };
    draw(cid);
    return { update: draw };
  }

  // --- Timeline actions ------------------------------------------------------------------------
  function guard(fn: () => void) {
    try { fn(); bump(); } catch (e) { onnotice(e instanceof StudioError ? e.reason : String(e), "warn"); }
  }

  function addFrameAfter(copy: boolean) {
    if (!root) return;
    commit();
    guard(() => {
      const bytes = copy && frameId ? store.frameBytes(objectId, frameId) : null;
      const blank = new PixRaster(root.w, root.h, raster?.palette.map((e) => ({ ...e })) ?? DEFAULT_PALETTE.map((e) => ({ ...e }))).encode();
      const id = store.insertFrame(objectId, frameId || null, bytes ?? blank);
      openFrame(id);
    });
  }

  function deleteFrame() {
    if (!root || !frameId) return;
    const other = store.claimOn(frameId);
    if (other && other.by !== me) onnotice(`${who(other.by)} is drawing this frame; deleting it anyway (claims are advisory)`, "warn");
    const idx = frameIndex;
    const gone = frameId;
    guard(() => store.apply(objectId, { op: "remove_frame", frame: gone }));
    frameId = "";
    raster = null;
    const next = root.frames[Math.min(idx, root.frames.length - 1)];
    if (next) openFrame(next);
  }

  function step(delta: number) {
    if (!frames.length) return;
    const n = frames.length;
    let i = (frameIndex + delta + n) % n;
    // Playback skips frames that are over the cap or not held; stepping does not.
    if (playing) {
      let guardN = n;
      while (guardN-- && (overCap.has(frames[i]) || !store.frameBytes(objectId, frames[i]))) i = (i + delta + n) % n;
    }
    if (!loop && playing && i === 0 && delta > 0) { playing = false; return; }
    openFrame(frames[i], true);
  }

  let playTimer: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (playTimer) { clearInterval(playTimer); playTimer = null; }
    if (playing && root) playTimer = setInterval(() => step(1), 1000 / Math.max(FLIPNOTE_FPS_MIN, Math.min(FLIPNOTE_FPS_MAX, root.fps)));
    return () => { if (playTimer) clearInterval(playTimer); };
  });

  function setFps(v: number) { guard(() => store.apply(objectId, { op: "set_header", field: "fps", value: v })); }
  function setTitle(v: string) { if (root && v.trim() && v.trim() !== root.title) guard(() => store.apply(objectId, { op: "set_header", field: "title", value: v.trim() })); }

  // --- Sound tab ---------------------------------------------------------------------------------
  const NOTE_CHOICES = [["c4", 60], ["c5", 72], ["g5", 79], ["c2", 36]] as const;
  function noteName(n: number): string {
    const names = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    return `${names[n % 12]}${Math.floor(n / 12) - 1}`;
  }
  function addSfx(note = 72) {
    if (!root || !frameId) return;
    guard(() => {
      if (!root.patches.meow) store.apply(objectId, { op: "set_patch", patch: "meow", descriptor: { v: 1, name: "meow" } });
      store.apply(objectId, { op: "set_sfx", sfx: randomElementId(), frame: frameId, patch: "meow", note });
    });
  }
  function setSfxNote(sfx: string, note: number) {
    if (!root) return;
    const s = root.sfx[sfx];
    guard(() => store.apply(objectId, { op: "set_sfx", sfx, frame: s.fr, patch: s.p, note }));
  }
  function removeSfx(sfx: string) { guard(() => store.apply(objectId, { op: "remove_sfx", sfx })); }
  function unlinkScore() { guard(() => store.apply(objectId, { op: "set_header", field: "score", value: null })); }

  // --- Claims -------------------------------------------------------------------------------------
  function pass() {
    if (!frameId || !claim) return;
    const asker = studio.people?.mika ?? "";
    store.passClaim(frameId, asker);
    commit();
    onnotice(`passed frame ${frameIndex + 1} to ${who(asker)}`, "info");
    bump();
  }
  function askFor() {
    if (!frameId || !claim) return;
    onnotice(`asked ${who(claim.by)} for frame ${frameIndex + 1} (claim frames are not on the wire yet)`, "info");
  }
  function takeConflict(theirs: boolean) {
    if (!conflict || !root) return;
    guard(() => {
      if (theirs) store.apply(objectId, { op: "replace_frame", frame: conflict.frame, cid: conflict.theirs.cid, bytes: conflict.theirs.bytes });
      store.conflicts.delete(conflict.frame);
    });
    if (theirs) onnotice(`${who(conflict.by)}'s version is now frame ${frameIndex + 1}; its pixels arrive when blob fetch is connected`, "info");
    frameId = "";
  }
  function keepBoth() {
    if (!conflict || !root) return;
    guard(() => {
      store.apply(objectId, { op: "insert_frame", frame: randomElementId(), after: conflict.frame, cid: conflict.theirs.cid, bytes: conflict.theirs.bytes });
      store.conflicts.delete(conflict.frame);
    });
  }

  let claimTimer: ReturnType<typeof setInterval> | null = null;
  onMount(() => {
    claimTimer = setInterval(() => { tick++; if (frameId && dirty) store.renewClaim(frameId); }, 1000);
    const renew = setInterval(() => { if (frameId) store.renewClaim(frameId); }, CLAIM_RESEND_MS);
    return () => { if (claimTimer) clearInterval(claimTimer); clearInterval(renew); };
  });
  onDestroy(() => { commit(); if (frameId) store.releaseClaim(frameId); });

  // --- Keys --------------------------------------------------------------------------------------
  function onKey(e: KeyboardEvent) {
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
    const k = e.key;
    if ((e.ctrlKey || e.metaKey) && k.toLowerCase() === "z") { e.preventDefault(); if (e.shiftKey) doRedo(); else doUndo(); return; }
    if ((e.ctrlKey || e.metaKey) && k.toLowerCase() === "y") { e.preventDefault(); doRedo(); return; }
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const map: Record<string, () => void> = {
      b: () => (tool = "pen"), e: () => (tool = "eraser"), g: () => (tool = "fill"), l: () => (tool = "shape"),
      s: () => (tool = "stamp"), t: () => (tool = "text"), m: () => (mirror = { ...mirror, h: !mirror.h }),
      o: () => (onion = !onion), "#": () => (grid = !grid),
      "[": () => (brush = clampBrush(brush - 1)), "]": () => (brush = clampBrush(brush + 1)),
      "+": () => setZoom(zoom + 1), "=": () => setZoom(zoom + 1), "-": () => setZoom(zoom - 1),
      ",": () => step(-1), ".": () => step(1), " ": () => (playing = !playing),
    };
    if (map[k]) { e.preventDefault(); map[k](); return; }
    if (/^[0-9]$/.test(k) && raster) { e.preventDefault(); color = Math.min(raster.palette.length - 1, k === "0" ? 9 : Number(k) - 1); }
  }

  function fmtKib(n: number): string { return n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} mib` : `${(n / 1024).toFixed(1)} kib`; }
  function rel(ts: number): string {
    const d = Date.now() - ts;
    if (d < 60_000) return "just now";
    if (d < 3_600_000) return `${Math.round(d / 60_000)} min ago`;
    if (d < 86_400_000) return `${Math.round(d / 3_600_000)}h ago`;
    return `${Math.round(d / 86_400_000)}d ago`;
  }
  function settlementChip(): { text: string; tone: string } {
    const s = settlement;
    if (!s) return { text: "new · on this device only", tone: "warn" };
    if (s.gate === "fault") return { text: "history fault", tone: "danger" };
    if (s.label === "settled") return { text: `settled · epoch ${s.epoch}`, tone: "ok" };
    if (s.label === "rotating") return { text: "rotating", tone: "warn" };
    if (s.label.startsWith("current owner")) return { text: "owner has not confirmed history", tone: "warn" };
    return { text: s.label, tone: "warn" };
  }
</script>

<!-- Hotkeys apply while focus is inside the editor (the tool rail, the canvas, the timeline);
     typing in one of its inputs is exempt inside onKey. -->
<svelte:window onkeydown={(e) => { if (rootEl && rootEl.contains(document.activeElement)) onKey(e); }} />
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="studio" bind:this={rootEl} role="application" aria-label="Flipnote editor" tabindex="0">
  {#if !root}
    <div class="studio-empty">
      {#if isScore}
        <p class="muted">The score editor is not built yet. This entry lives in the studio index as a score; open a flipnote on the left to draw.</p>
      {:else}
        <p class="muted">Pick a flipnote on the left, or start a new one.</p>
      {/if}
    </div>
  {:else}
    <!-- Header -->
    <div class="st-head">
      <input class="st-title" value={root.title} onchange={(e) => setTitle(e.currentTarget.value)} aria-label="Flipnote title" />
      <span class="micro">flipnote · {root.w}×{root.h}</span>
      <label class="st-chip" title="Frames per second, 1 to 24">
        <input type="number" min={FLIPNOTE_FPS_MIN} max={FLIPNOTE_FPS_MAX} value={root.fps} onchange={(e) => setFps(Number(e.currentTarget.value))} />
        <span class="micro">fps</span>
      </label>
      <span class="st-chip {settlementChip().tone}" title={settlement?.label ?? "not saved to the studio yet"}><i></i><span class="micro">{settlementChip().text}</span></span>
      <span class="grow"></span>
      <button type="button" class="st-tg" class:on={adaptOn} onclick={() => (adaptOn = !adaptOn)} title="Role colours follow your theme; literals never move"><i></i><span class="micro">adapt to my theme</span></button>
      <button type="button" class="st-btn" onclick={() => (playing = !playing)}>{playing ? "pause" : "play"}</button>
      <button type="button" class="st-btn" title="Post this frame into the channel as a doodle (not connected yet)" disabled={!frameId} onclick={() => { commit(); onnotice(store.postToChat(objectId, frameId), "info"); }}>
        <svg viewBox="0 0 16 16" style="width: 11px; height: 11px"><path d="M2 8l12-6-4 12-2.5-4.5z"></path></svg>
        post to chat
      </button>
      <button type="button" class="st-btn primary" title="Not connected yet" onclick={() => onnotice(store.exportPixa(objectId), "info")}>export .pixa</button>
    </div>

    {#if overCap.size}
      <div class="st-banner warn">document full: {overCap.size} frame{overCap.size === 1 ? "" : "s"} past the {FLIPNOTE_MAX_FRAMES}-frame list or the 8 MiB promise. Editing is refused until they are trimmed; playback skips them.</div>
    {:else if settlement?.gate === "fault"}
      <div class="st-banner danger">history fault: conflicting owner receipts. This document is read-only until the owner repairs it.</div>
    {:else if settlement?.label === "rotating"}
      <div class="st-banner warn">rotating: local edits only until the owner settles this rotation.</div>
    {/if}

    <!-- Editor row -->
    <div class="st-row">
      <div class="st-tools">
        {#each [["pen", "b"], ["eraser", "e"], ["fill", "g"], ["shape", "l"], ["stamp", "s"], ["text", "t"]] as [t, key]}
          <button type="button" class="st-tool" class:on={tool === t} title="{t} [{key}]" onclick={() => (tool = t as Tool)}>
            {#if t === "pen"}<svg viewBox="0 0 16 16"><path d="M3 13l1-4 7-7 3 3-7 7-4 1z"></path><path d="M9.5 3.5l3 3"></path></svg>
            {:else if t === "eraser"}<svg viewBox="0 0 16 16"><path d="M9 3l4 4-6 6H4.5L2.5 11z"></path><path d="M6 8l3.5 3.5M3 14h10"></path></svg>
            {:else if t === "fill"}<svg viewBox="0 0 16 16"><path d="M6.5 2.5l6 6-4.5 4.5L2.5 7.5z"></path><path d="M2.5 7.5h8"></path><path d="M13.5 10.5c0 1-.8 2-1 2s-1-1-1-2 1-2 1-2 1 1 1 2z"></path></svg>
            {:else if t === "shape"}<svg viewBox="0 0 16 16"><path d="M2.5 13.5l5-11 5 11z"></path><circle cx="11.5" cy="11.5" r="2.5"></circle></svg>
            {:else if t === "stamp"}<svg viewBox="0 0 16 16"><path d="M5.5 8.5V5.5a2.5 2.5 0 015 0v3"></path><path d="M3 8.5h10v2.5H3z"></path><path d="M4.5 11v2h7v-2"></path></svg>
            {:else}<svg viewBox="0 0 16 16"><path d="M3 4h10M8 4v9M6 13h4"></path></svg>{/if}
            <span class="k">{key}</span>
          </button>
        {/each}
        <button type="button" class="st-tool" class:on={mirror.h} title="mirror [m]" onclick={() => (mirror = { ...mirror, h: !mirror.h })}><svg viewBox="0 0 16 16"><path d="M8 2v12" stroke-dasharray="2 2"></path><path d="M5.5 5l-3 3 3 3M10.5 5l3 3-3 3"></path></svg><span class="k">m</span></button>
        <span class="st-sep"></span>
        <button type="button" class="st-tool" class:on={onion} title="onion skin [o]" onclick={() => (onion = !onion)}><svg viewBox="0 0 16 16"><rect x="2" y="2" width="8" height="8" rx="1"></rect><rect x="6" y="6" width="8" height="8" rx="1"></rect></svg><span class="k">o</span></button>
        <button type="button" class="st-tool" class:on={grid} title="gridlines [#]" onclick={() => (grid = !grid)}><svg viewBox="0 0 16 16"><rect x="2" y="2" width="12" height="12" rx="1"></rect><path d="M6 2v12M10 2v12M2 6h12M2 10h12"></path></svg><span class="k">#</span></button>
        <span class="grow"></span>
        <button type="button" class="st-tool" title="undo [ctrl+z]" disabled={!undo.canUndo} onclick={doUndo}><svg viewBox="0 0 16 16"><path d="M6 4L3 7l3 3"></path><path d="M3 7h6.5a3.5 3.5 0 010 7H7"></path></svg></button>
        <button type="button" class="st-tool" title="redo [ctrl+shift+z]" disabled={!undo.canRedo} onclick={doRedo}><svg viewBox="0 0 16 16"><path d="M10 4l3 3-3 3"></path><path d="M13 7H6.5a3.5 3.5 0 000 7H9"></path></svg></button>
      </div>

      <div class="st-canvas-card">
        <div class="st-canvas-scroll">
        <div class="st-canvas-wrap" class:locked={!canEdit} style="width: {root.w * zoom}px; aspect-ratio: {root.w} / {root.h}" onwheel={(e) => { if (e.ctrlKey) { e.preventDefault(); setZoom(zoom + (e.deltaY < 0 ? 1 : -1)); } }}>
          <canvas bind:this={canvasEl} class="st-canvas"></canvas>
          <canvas
            bind:this={overlayEl}
            class="st-overlay"
            onpointerdown={onDown}
            onpointermove={onMove}
            onpointerup={onUp}
            onpointercancel={onUp}
            onpointerleave={onLeave}
            oncontextmenu={(e) => e.preventDefault()}
          ></canvas>
          {#if pendingOpen && claim && claim.by !== me}
            <div class="st-veil">
              <span class="who" style="--c: {tint(claim.by)}"></span>
              <span>{who(claim.by)} is drawing this frame · {claimLeft}s</span>
              <span class="micro">courtesy claim · not a lock</span>
              <span class="st-veil-acts">
                <button type="button" class="st-btn" onclick={askFor}>ask</button>
                <button type="button" class="st-btn" onclick={() => openFrame(pendingOpen, true)}>open anyway</button>
              </span>
            </div>
          {:else if frameState === "fetching"}
            <div class="st-veil"><span>fetching {fmtKib(frameRec?.bytes ?? 0)}</span><span class="micro">bounded by the declared size · blob fetch is not connected yet</span></div>
          {:else if frameState === "replaying"}
            <div class="st-veil soft"><span class="micro">replaying your edit after the owner's checkpoint left it out</span></div>
          {/if}
          <span class="st-readout left">{zoom}× · {hover ? `${hover[0]},${hover[1]}` : "…"}{#if tool === "shape"} · {shape}{/if}</span>
          <span class="st-readout right">layer · {LAYER_NAMES[layer]}{#if dirty} · unsaved{/if}</span>
        </div>
        </div>
        <div class="st-canvas-foot">
          <span class="micro">{tool === "shape" ? "drag to place" : tool === "text" ? "click to place the text" : tool === "stamp" ? "click to place the stamp" : "drag paints · [ ] size · 1 to 0 colours"}</span>
          <span class="grow"></span>
          <span class="st-zoom">
            <button type="button" class="st-tile txt" title="zoom out [-]" disabled={zoom <= ZOOM_MIN} onclick={() => setZoom(zoom - 1)}>-</button>
            <span class="mono">{zoom}×</span>
            <button type="button" class="st-tile txt" title="zoom in [+]" disabled={zoom >= ZOOM_MAX} onclick={() => setZoom(zoom + 1)}>+</button>
          </span>
        </div>
      </div>

      <!-- Inspector -->
      <div class="st-inspector">
        <div class="st-itabs">
          {#each ["art", "sound", "music"] as t}
            <button type="button" class="st-itab" class:on={inspectorTab === t} onclick={() => (inspectorTab = t as typeof inspectorTab)}>{t}</button>
          {/each}
        </div>
        <div class="st-ibody">
          {#if inspectorTab === "art"}
            <div class="st-sec">tool · {tool}</div>
            {#if tool === "shape"}
              <div class="st-opt"><span class="lb">shape</span>
                {#each ["line", "rect", "ellipse", "triangle"] as s}
                  <button type="button" class="st-tile" class:on={shape === s} onclick={() => (shape = s as Shape)} title={s}>
                    {#if s === "line"}<svg viewBox="0 0 16 16"><path d="M3 13L13 3"></path></svg>
                    {:else if s === "rect"}<svg viewBox="0 0 16 16"><rect x="2.5" y="3.5" width="11" height="9"></rect></svg>
                    {:else if s === "ellipse"}<svg viewBox="0 0 16 16"><ellipse cx="8" cy="8" rx="5.5" ry="4"></ellipse></svg>
                    {:else}<svg viewBox="0 0 16 16"><path d="M8 3l5.5 10h-11z"></path></svg>{/if}
                  </button>
                {/each}
              </div>
              <div class="st-opt"><span class="lb">fill</span>
                <button type="button" class="st-tile" class:on={!shapeFill} onclick={() => (shapeFill = false)} title="outline"><svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="5"></circle></svg></button>
                <button type="button" class="st-tile" class:on={shapeFill} onclick={() => (shapeFill = true)} title="filled"><svg viewBox="0 0 16 16" class="solid"><circle cx="8" cy="8" r="5.5"></circle></svg></button>
              </div>
            {:else if tool === "stamp"}
              <div class="st-opt wrap"><span class="lb">stamp</span>
                {#each STAMPS as s, i}
                  <button type="button" class="st-tile" class:on={stampIdx === i} onclick={() => (stampIdx = i)} title={s.name}>
                    <svg viewBox="0 0 8 8" shape-rendering="crispEdges" class="solid">
                      {#each Array.from(s.bm.pixels) as v, p}{#if v !== CLEAR}<rect x={p % 8} y={Math.floor(p / 8)} width="1" height="1" style="fill: {raster ? wellCss(raster.palette[v] ?? DEFAULT_PALETTE[v]) : 'currentColor'}"></rect>{/if}{/each}
                    </svg>
                  </button>
                {/each}
              </div>
              <p class="micro wrap">stamps from emoji/ arrive with the stamp editor; these are built in</p>
            {:else if tool === "text"}
              <div class="st-opt"><span class="lb">text</span><input class="st-input" bind:value={textDraft} maxlength="24" /></div>
              <p class="micro wrap">5×7 face · becomes pixels on place</p>
            {:else if tool === "pen" || tool === "eraser"}
              <div class="st-opt">
                <span class="lb">pen</span>
                <button type="button" class="st-tile txt" class:on={pressureOn} onclick={() => (pressureOn = !pressureOn)} title="A pen's pressure sets the size, up to the chosen size. A mouse always paints the chosen size.">pressure</button>
                <span class="micro">{penSeen ? "pen detected" : "no pen seen yet"}</span>
              </div>
            {/if}
            <div class="st-opt"><span class="lb">size</span>
              <span class="st-size">
                {#each Array.from({ length: BRUSH_MAX - BRUSH_MIN + 1 }, (_, i) => i + BRUSH_MIN) as s}
                  <button type="button" class:on={brush === s} style="height: {4 + s * 1.4}px" onclick={() => (brush = s)} aria-label="brush {s}"></button>
                {/each}
              </span>
              <span class="mono">{brush}</span>
              <span class="lb" style="width: auto; margin-left: 10px">mirror</span>
              <button type="button" class="st-tile txt" class:on={mirror.h} onclick={() => (mirror = { ...mirror, h: !mirror.h })}>h</button>
              <button type="button" class="st-tile txt" class:on={mirror.v} onclick={() => (mirror = { ...mirror, v: !mirror.v })}>v</button>
            </div>

            <div class="st-sec nxt">palette</div>
            <div class="st-wells">
              {#each raster?.palette ?? DEFAULT_PALETTE as e, i}
                <button type="button" class="st-well" class:sel={color === i} style="background: {wellCss(e)}" title="{i}: {e.role ? PIX_ROLE_NAMES[e.role] + ' role, follows your theme' : 'literal'} #{e.r.toString(16).padStart(2, '0')}{e.g.toString(16).padStart(2, '0')}{e.b.toString(16).padStart(2, '0')}" onclick={() => (color = i)}>
                  <span class="micro">{PALETTE_LABELS[i] ?? (e.role ? PIX_ROLE_NAMES[e.role] : i)}</span>
                </button>
              {/each}
            </div>
            <p class="micro wrap">server palette · roles recolour per viewer</p>

            <div class="st-sec nxt">layers</div>
            {#each [2, 1, 0] as l}
              <div class="st-layer" class:on={layer === l}>
                <button type="button" class="eye" class:off={!layerVisible[l]} title="show / hide (view only)" onclick={() => { const v = [...layerVisible]; v[l] = !v[l]; layerVisible = v; }}>
                  <svg viewBox="0 0 16 16"><path d="M1.5 8s2.5-4.5 6.5-4.5S14.5 8 14.5 8s-2.5 4.5-6.5 4.5S1.5 8 1.5 8z"></path><circle cx="8" cy="8" r="2"></circle>{#if !layerVisible[l]}<path d="M2 2l12 12"></path>{/if}</svg>
                </button>
                <!-- The whole row selects the layer: a stack glyph with this layer's sheet lit. -->
                <button type="button" class="pick" onclick={() => (layer = l)} title="draw on the {LAYER_NAMES[l]} layer">
                  <svg viewBox="0 0 16 16" class="stack">
                    <path d="M2 11.5l6 3 6-3" class:lit={l === 0}></path>
                    <path d="M2 8l6 3 6-3" class:lit={l === 1}></path>
                    <path d="M8 1.5l6 3-6 3-6-3z" class:lit={l === 2}></path>
                  </svg>
                  <span class="nm">{LAYER_NAMES[l]}</span>
                  <span class="what">{layer === l ? "drawing here" : l === 0 ? "sky · ground" : ""}</span>
                </button>
              </div>
            {/each}
            <p class="micro wrap">three local layers · flattened into one pix frame on save</p>

            <div class="st-sec nxt">frame {frameIndex + 1}</div>
            {#if conflict}
              <div class="st-card warn">
                <span>another version by <b style="color: {tint(conflict.by)}">{who(conflict.by)}</b></span>
                <span class="micro">both replaced this frame at once · nothing resolved silently</span>
                <span class="st-card-acts">
                  <button type="button" class="st-btn" onclick={() => takeConflict(false)}>keep mine</button>
                  <button type="button" class="st-btn" onclick={() => takeConflict(true)}>take theirs</button>
                  <button type="button" class="st-btn ghost" onclick={keepBoth}>keep both</button>
                </span>
              </div>
            {/if}
            {#if claim && claim.by === me}
              <div class="st-card accent">
                <span>You're editing{#if dirty} · unsaved{/if}</span>
                {#if claim.ask}<span class="sub">{who(studio.people?.mika ?? "")} asked for a turn</span>{/if}
                <span class="micro it">courtesy claim · not a lock</span>
                {#if claim.ask}<span class="st-card-acts"><button type="button" class="st-btn primary" onclick={pass}>pass</button></span>{/if}
              </div>
            {:else if claim}
              <div class="st-card" style="--c: {tint(claim.by)}">
                <span><b style="color: var(--c)">{who(claim.by)}</b> is drawing this frame · {claimLeft}s</span>
                <span class="micro it">courtesy claim · not a lock</span>
                <span class="st-card-acts">
                  <button type="button" class="st-btn" onclick={askFor}>ask</button>
                  {#if pendingOpen}<button type="button" class="st-btn" onclick={() => openFrame(pendingOpen, true)}>open anyway</button>{/if}
                </span>
              </div>
            {:else}
              <p class="micro">by {frameRec ? who(frameRec.author) : "nobody"} · {frameRec ? rel(frameRec.ts) : ""} · {fmtKib(frameRec?.bytes ?? 0)}</p>
            {/if}
          {:else if inspectorTab === "sound"}
            <div class="st-sec">frame {frameIndex + 1}</div>
            {#each sfxHere as [sid, s] (sid)}
              <div class="st-card accent row">
                <span class="mono">{s.p} · {noteName(s.n)}</span>
                <span class="micro">from :cat: · 180 ms</span>
                <span class="grow"></span>
                <button type="button" class="st-link danger" onclick={() => removeSfx(sid)}>remove</button>
              </div>
              <div class="st-opt"><span class="lb">note</span>
                {#each NOTE_CHOICES as [nm, n]}
                  <button type="button" class="st-tile txt" class:on={s.n === n} onclick={() => setSfxNote(sid, n)}>{nm}</button>
                {/each}
              </div>
            {:else}
              <p class="muted small">No sound on this frame.</p>
              <button type="button" class="st-btn ghost dashed" onclick={() => addSfx()}>+ add emoji sound</button>
            {/each}
            <p class="micro wrap">a sound is a jam patch on an emoji, played by your own synth · playback arrives with the emoji sounds slice</p>
            <div class="st-sec nxt">all sfx · {allSfx.length}</div>
            {#each allSfx as [sid, s] (sid)}
              <button type="button" class="st-sfx" class:cur={s.fr === frameId} onclick={() => openFrame(s.fr)}>
                <span class="fr">{frames.indexOf(s.fr) + 1}</span>
                <i style="background: {tint(root.frame[s.fr]?.author ?? me)}"></i>
                <span class="mono">{s.p} · {noteName(s.n)}</span>
                <span class="grow"></span>
                <span class="micro">{who(root.frame[s.fr]?.author ?? me)}</span>
              </button>
            {/each}
          {:else}
            <div class="st-sec">soundtrack</div>
            {#if scoreTitle}
              <div class="st-card accent row">
                <span class="mono">{scoreTitle}</span>
                <span class="micro">score · linked</span>
                <span class="grow"></span>
                <button type="button" class="st-link" onclick={unlinkScore}>unlink</button>
              </div>
            {:else}
              <p class="muted small">No score linked. The score editor is a later slice.</p>
            {/if}
            <p class="micro wrap">patches {patchCount} of {FLIPNOTE_MAX_PATCHES} · score patches first, then sfx</p>
            <div class="st-sec nxt">size</div>
            <div class="st-bar"><i style="width: {Math.min(100, (totalBytes / FLIPNOTE_FRAME_BYTES_PROMISE) * 100)}%"></i></div>
            <p class="micro wrap">{fmtKib(totalBytes)} of 8 mib · {frames.length} of {FLIPNOTE_MAX_FRAMES} frames</p>
            <div class="st-sec nxt">history</div>
            {#if settlement}
              <p class="st-hist"><i class={settlement.gate === "settled" ? "ok" : settlement.gate === "fault" ? "danger" : "warn"}></i>
                {#if settlement.receiptBy}settled by {who(settlement.receiptBy)} · {rel(settlement.receiptTs)}{:else}no owner receipt yet{/if}
              </p>
              <p class="micro wrap">{settlement.label}</p>
            {/if}
            {#if recovery?.retained.length}
              <p class="st-hist"><i class="info"></i>previous version available · epoch {recovery.retained[0].epoch}</p>
              <span class="st-card-acts">
                <button type="button" class="st-btn" onclick={() => onnotice(store.recoveryAction(objectId, "restore"), "info")}>restore</button>
                <button type="button" class="st-btn" onclick={() => onnotice(store.recoveryAction(objectId, "copy"), "info")}>copy</button>
                <button type="button" class="st-btn ghost" onclick={() => onnotice(store.recoveryAction(objectId, "export"), "info")}>export</button>
              </span>
            {/if}
            {#if recovery?.evictionDeadline}
              <p class="st-hist"><i class="warn"></i>a previous version will be removed in {Math.max(0, Math.ceil((recovery.evictionDeadline - Date.now()) / 86_400_000))}d unless exported</p>
            {/if}
          {/if}
        </div>
      </div>
    </div>

    <!-- Timeline -->
    <div class="st-timeline">
      <div class="st-transport">
        <button type="button" class="st-tool sm" title="first frame" onclick={() => frames[0] && openFrame(frames[0], true)}><svg viewBox="0 0 16 16" class="solid"><path d="M8 3 2 8l6 5zM14 3 8 8l6 5z"></path></svg></button>
        <button type="button" class="st-tool sm" title="previous frame [,]" onclick={() => step(-1)}><svg viewBox="0 0 16 16" class="solid"><path d="M3 3h1.5v10H3zM12.5 3 6 8l6.5 5z"></path></svg></button>
        <button type="button" class="st-tool play" class:on={playing} title={playing ? "pause [space]" : "play [space]"} onclick={() => (playing = !playing)}>
          {#if playing}<svg viewBox="0 0 16 16" class="solid"><path d="M3.5 2.5h3.5v11H3.5zM9 2.5h3.5v11H9z"></path></svg>{:else}<svg viewBox="0 0 16 16" class="solid"><path d="M4 2.5 13 8 4 13.5z"></path></svg>{/if}
        </button>
        <button type="button" class="st-tool sm" title="next frame [.]" onclick={() => step(1)}><svg viewBox="0 0 16 16" class="solid"><path d="M3.5 3 10 8l-6.5 5zM11.5 3H13v10h-1.5z"></path></svg></button>
        <button type="button" class="st-tool sm" title="last frame" onclick={() => frames.length && openFrame(frames[frames.length - 1], true)}><svg viewBox="0 0 16 16" class="solid"><path d="M2 3l6 5-6 5zM8 3l6 5-6 5z"></path></svg></button>
        <button type="button" class="st-tool sm" class:on={loop} title="loop" onclick={() => (loop = !loop)}><svg viewBox="0 0 16 16"><path d="M3 8a5 5 0 015-5h4M13 8a5 5 0 01-5 5H4"></path><path d="M10.5 1.5 12 3l-1.5 1.5M5.5 11.5 4 13l1.5 1.5"></path></svg></button>
        <span class="mono strong">{frameIndex + 1} / {frames.length}</span>
        <span class="micro">{((frameIndex + 1) / Math.max(1, root.fps)).toFixed(2)} s of {(frames.length / Math.max(1, root.fps)).toFixed(2)} s</span>
        <span class="grow"></span>
        {#each frames.filter((f) => { const c = store.claimOn(f); return c && c.by !== me; }).slice(0, 2) as f (f)}
          {@const c = store.claimOn(f)}
          {#if c}<span class="st-chip" style="--c: {tint(c.by)}; border-color: color-mix(in oklab, var(--c) 40%, var(--panel)); background: color-mix(in oklab, var(--c) 16%, var(--panel))"><i style="background: var(--c)"></i><span class="micro" style="color: var(--c)">{who(c.by)} is drawing {frames.indexOf(f) + 1}</span></span>{/if}
        {/each}
        <button type="button" class="st-btn ghost" disabled={!canEdit && frames.length > 0} onclick={() => addFrameAfter(false)}>+ frame after</button>
        <button type="button" class="st-btn ghost" disabled={!canEdit} onclick={() => addFrameAfter(true)}>duplicate</button>
        <button type="button" class="st-btn ghost" disabled={!frameId} onclick={deleteFrame}>delete</button>
      </div>
      <div class="st-strip">
        {#each frames as f, i (f)}
          {@const rec = root.frame[f]}
          {@const c = store.claimOn(f)}
          {@const st = store.frameState.get(f) ?? "held"}
          <button type="button" class="st-thumb" class:cur={f === frameId} class:over={overCap.has(f)} class:conflict={store.conflicts.has(f)} style="--c: {c ? tint(c.by) : 'transparent'}" onclick={() => openFrame(f)}>
            <span class="ix" style={c && c.by !== me ? `color: ${tint(c.by)}` : ""}>{i + 1}{#if c && c.by !== me} · {who(c.by)} {store.claimSecondsLeft(f)}s{/if}</span>
            <span class="fr" class:claimed={!!c && c.by !== me} class:fetching={st === "fetching"} class:replaying={st === "replaying"}>
              {#if st === "fetching"}
                <span class="micro">fetching</span>
              {:else}
                <canvas width="64" height="48" use:thumb={rec?.cid ?? ""}></canvas>
              {/if}
              <i class="dot" style="background: {tint(rec?.author ?? me)}"></i>
              {#if store.conflicts.has(f)}<i class="corner"></i>{/if}
            </span>
          </button>
        {/each}
      </div>
      <div class="st-sfxlane">
        <span class="micro lb">sfx</span>
        {#each frames as f (f)}
          <span class="cell">{#each allSfx.filter(([, s]) => s.fr === f) as [sid, s] (sid)}<i style="background: {tint(root.frame[s.fr]?.author ?? me)}"></i>{/each}</span>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .studio { display: flex; flex-direction: column; flex: 1; min-height: 0; outline: none; gap: 0; }
  .studio-empty { padding: 2rem; }
  .micro { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .micro.wrap { white-space: normal; line-height: 1.5; margin: 4px 0 0; }
  .micro.it { font-style: italic; text-transform: none; letter-spacing: 0.04em; }
  .mono { font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); white-space: nowrap; }
  .mono.strong { color: var(--text); }
  .grow { flex: 1; }
  svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; flex: none; }
  svg.solid { fill: currentColor; stroke: none; }

  /* Header: the channel head's bar, with the document's facts and view toggles. */
  .st-head { display: flex; align-items: center; gap: 0.6rem; padding: 0.4rem 0.6rem; background: var(--panel); border-bottom: 1px solid var(--border); margin: -0.3rem -0.5rem 0.2rem; flex-wrap: wrap; }
  .st-title { font-size: 0.95rem; font-weight: 600; color: var(--text); background: transparent; border: 1px solid transparent; padding: 2px 4px; width: 9rem; }
  .st-title:hover, .st-title:focus { border-color: var(--border); background: var(--bg-elev); }
  .st-chip { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 2px 9px; white-space: nowrap; }
  .st-chip i { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); }
  .st-chip.ok { border-color: var(--ok-brd); background: var(--ok-dim); } .st-chip.ok .micro, .st-chip.ok i { color: var(--ok); background: var(--ok); }
  .st-chip.ok .micro { background: none; }
  .st-chip.warn { border-color: var(--warn-brd); background: var(--warn-dim); } .st-chip.warn .micro { color: var(--warn); } .st-chip.warn i { background: var(--warn); }
  .st-chip.danger { border-color: var(--danger-brd); background: var(--danger-dim); } .st-chip.danger .micro { color: var(--danger); } .st-chip.danger i { background: var(--danger); }
  .st-chip input { width: 2.2rem; background: transparent; border: none; color: var(--text-2); font-family: var(--mono); font-size: 0.68rem; padding: 0; text-align: right; }
  .st-tg { display: inline-flex; align-items: center; gap: 6px; background: transparent; border: none; padding: 0; color: inherit; }
  .st-tg i { width: 22px; height: 12px; border-radius: 999px; background: var(--bg-elev); border: 1px solid var(--border); position: relative; display: inline-block; }
  .st-tg i::after { content: ""; position: absolute; top: 1px; left: 1px; width: 8px; height: 8px; border-radius: 50%; background: var(--faint); }
  .st-tg.on i { background: var(--accent-dim); border-color: var(--accent); }
  .st-tg.on i::after { left: 11px; background: var(--accent); }
  .st-tg.on .micro { color: var(--text-2); }
  .st-btn { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.06em; text-transform: uppercase; white-space: nowrap; }
  .st-btn.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
  .st-btn.ghost { background: transparent; border-color: var(--border-soft); color: var(--muted); }
  .st-btn.dashed { border-style: dashed; }
  .st-btn:disabled { opacity: 0.45; }
  .st-banner { margin: 0.2rem 0; padding: 6px 10px; border-radius: var(--r); font-size: 0.78rem; }
  .st-banner.warn { background: var(--warn-dim); border: 1px solid var(--warn-brd); color: var(--text-2); }
  .st-banner.danger { background: var(--danger-dim); border: 1px solid var(--danger-brd); color: var(--text-2); }

  /* Editor row */
  .st-row { flex: 1; min-height: 0; display: flex; gap: 10px; padding: 6px 0; }
  .st-tools { display: flex; flex-direction: column; gap: 4px; flex: none; }
  .st-tool { width: 32px; height: 32px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); display: flex; align-items: center; justify-content: center; position: relative; padding: 0; }
  .st-tool.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-tool:disabled { opacity: 0.4; }
  .st-tool .k { position: absolute; right: 2px; bottom: 0; font-family: var(--mono); font-size: 0.5rem; color: var(--faint); }
  .st-tool.on .k { color: var(--accent-hi); }
  .st-tool.sm { width: 26px; height: 26px; } .st-tool.sm svg { width: 11px; height: 11px; }
  /* Play is the one transport control that changes state; it reads as the primary button. */
  .st-tool.play { width: 34px; height: 30px; border-radius: 999px; background: var(--accent); border-color: var(--accent); color: var(--on-accent); margin: 0 4px; }
  .st-tool.play svg { width: 14px; height: 14px; }
  .st-tool.play.on { background: var(--accent-dim); color: var(--accent-hi); }
  .st-sep { height: 1px; background: var(--border-soft); margin: 4px 2px; }
  .st-canvas-card { flex: 0 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 6px; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); padding: 10px; max-width: 100%; }
  .st-canvas-scroll { overflow: auto; max-height: 100%; scrollbar-gutter: stable; }
  .st-canvas-wrap { position: relative; max-width: none; border: 1px solid var(--border); line-height: 0; cursor: crosshair; }
  .st-zoom { display: inline-flex; align-items: center; gap: 6px; }
  .st-zoom .st-tile.txt { width: 26px; padding: 0; font-size: 0.8rem; }
  .st-canvas-wrap.locked { cursor: default; }
  .st-canvas, .st-overlay { position: absolute; inset: 0; width: 100%; height: 100%; image-rendering: pixelated; }
  .st-canvas { position: relative; display: block; }
  .st-overlay { touch-action: none; }
  .st-veil { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; background: color-mix(in oklab, var(--bg-0) 72%, transparent); color: var(--text); font-size: 0.84rem; line-height: 1.4; text-align: center; }
  .st-veil.soft { justify-content: flex-end; padding-bottom: 28px; background: transparent; }
  .st-veil .who { width: 8px; height: 8px; border-radius: 2px; background: var(--c); }
  .st-veil-acts, .st-card-acts { display: flex; gap: 6px; margin-top: 4px; }
  .st-readout { position: absolute; bottom: 6px; font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); background: color-mix(in oklab, var(--bg-0) 70%, transparent); padding: 2px 5px; border-radius: 3px; line-height: 1.3; }
  .st-readout.left { left: 8px; } .st-readout.right { right: 8px; color: var(--accent-hi); }
  .st-canvas-foot { display: flex; align-items: center; gap: 10px; }

  /* Inspector */
  .st-inspector { flex: 1; min-width: 240px; display: flex; flex-direction: column; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); overflow: hidden; }
  .st-itabs { display: flex; border-bottom: 1px solid var(--border); }
  .st-itab { flex: 1; padding: 7px 0; border: none; border-bottom: 2px solid transparent; border-radius: 0; background: transparent; font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); }
  .st-itab.on { color: var(--text); border-bottom-color: var(--accent); }
  .st-ibody { display: flex; flex-direction: column; gap: 4px; padding: 4px 12px 10px; overflow-y: auto; }
  .st-sec { padding: 6px 0 4px; font-family: var(--mono); font-size: 0.62rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-2); }
  .st-sec.nxt { border-top: 1px solid var(--border-soft); margin-top: 6px; }
  .st-opt { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; row-gap: 4px; }
  .st-opt .lb { font-family: var(--mono); font-size: 0.6rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); width: 44px; flex: none; }
  .st-tile { display: inline-flex; align-items: center; justify-content: center; width: 30px; height: 26px; padding: 0; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); font-family: var(--mono); font-size: 0.6rem; text-transform: uppercase; }
  .st-tile.txt { width: auto; padding: 0 8px; }
  .st-tile.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-tile svg { width: 14px; height: 14px; }
  .st-input { flex: 1; min-width: 0; background: var(--bg-elev); border: 1px solid var(--border); border-radius: var(--r); color: var(--text); font-family: var(--mono); font-size: 0.72rem; padding: 3px 6px; }
  .st-size { display: inline-flex; gap: 2px; align-items: flex-end; height: 16px; }
  .st-size button { width: 6px; padding: 0; border: none; border-radius: 1px; background: var(--faint); }
  .st-size button.on { background: var(--accent); }
  .st-wells { display: grid; grid-template-columns: repeat(8, minmax(0, 1fr)); gap: 4px; }
  .st-well { height: 22px; border-radius: 3px; border: 1px solid color-mix(in oklab, var(--bg-0) 60%, transparent); padding: 0; position: relative; }
  .st-well .micro { position: absolute; left: 0; right: 0; top: 100%; font-size: 0.5rem; letter-spacing: 0.04em; text-align: center; }
  .st-wells { margin-bottom: 14px; }
  .st-well.sel { outline: 2px solid var(--accent); outline-offset: 2px; }
  .st-layer { display: flex; align-items: center; gap: 8px; padding: 3px 6px; border-radius: var(--r); border: 1px solid transparent; }
  .st-layer.on { background: var(--accent-dim); border-color: var(--accent); }
  .st-layer .eye { width: 20px; height: 20px; padding: 0; border: none; background: transparent; color: var(--muted); display: inline-flex; align-items: center; justify-content: center; }
  .st-layer .eye svg { width: 14px; height: 14px; } .st-layer .eye.off { color: var(--faint); }
  .st-layer .pick { flex: 1; display: flex; align-items: center; gap: 8px; background: transparent; border: none; padding: 2px 0; color: inherit; text-align: left; }
  .st-layer .pick .stack { width: 16px; height: 16px; color: var(--faint); }
  .st-layer .pick .stack .lit { stroke: var(--accent); stroke-width: 2; }
  .st-layer .nm { font-family: var(--mono); font-size: 0.66rem; color: var(--text-2); }
  .st-layer.on .nm { color: var(--text); }
  .st-layer .what { margin-left: auto; font-size: 0.66rem; color: var(--muted); }
  .st-card { display: flex; flex-direction: column; gap: 3px; background: var(--bg-elev); border: 1px solid var(--border-soft); border-radius: var(--r); padding: 8px 10px; font-size: 0.8rem; color: var(--text); }
  .st-card.row { flex-direction: row; align-items: center; gap: 8px; }
  .st-card.accent { background: var(--accent-dim); border-color: var(--accent); }
  .st-card.warn { background: var(--warn-dim); border-color: var(--warn-brd); }
  .st-card .sub { font-size: 0.72rem; color: var(--text-2); }
  .st-link { background: transparent; border: none; padding: 0; font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); text-decoration: underline; }
  .st-link.danger { color: var(--danger); }
  .st-sfx { display: flex; align-items: center; gap: 8px; padding: 4px 6px; border-radius: var(--r); background: transparent; border: none; color: inherit; width: 100%; text-align: left; }
  .st-sfx.cur { background: var(--accent-dim); }
  .st-sfx .fr { font-family: var(--mono); font-size: 0.62rem; color: var(--faint); width: 22px; text-align: right; }
  .st-sfx i { width: 7px; height: 7px; border-radius: 2px; flex: none; }
  .st-bar { height: 4px; border-radius: 999px; background: var(--bg-elev); overflow: hidden; }
  .st-bar i { display: block; height: 100%; background: var(--accent); }
  .st-hist { display: flex; align-items: center; gap: 6px; margin: 2px 0; font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); }
  .st-hist i { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .st-hist i.ok { background: var(--ok); } .st-hist i.warn { background: var(--warn); } .st-hist i.danger { background: var(--danger); } .st-hist i.info { background: var(--info); }

  /* Timeline */
  .st-timeline { flex: none; background: var(--panel); border-top: 1px solid var(--border); margin: 0 -0.5rem; padding: 8px 10px 10px; display: flex; flex-direction: column; gap: 8px; }
  .st-transport { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .st-strip { display: flex; gap: 6px; align-items: flex-start; overflow-x: auto; padding-bottom: 4px; scrollbar-gutter: stable; }
  .st-thumb { display: flex; flex-direction: column; gap: 3px; align-items: center; background: transparent; border: none; padding: 0; color: inherit; flex: none; }
  .st-thumb .ix { font-family: var(--mono); font-size: 0.58rem; color: var(--faint); white-space: nowrap; }
  .st-thumb.cur .ix { color: var(--accent-hi); }
  .st-thumb .fr { position: relative; display: block; width: 64px; height: 48px; border: 1px solid var(--border); border-radius: 3px; overflow: hidden; line-height: 0; background: var(--bg-0); }
  .st-thumb .fr canvas { display: block; width: 64px; height: 48px; image-rendering: pixelated; }
  .st-thumb.cur .fr { outline: 2px solid var(--accent); outline-offset: 1px; }
  .st-thumb .fr.claimed { outline: 2px solid var(--c); outline-offset: 1px; border-color: var(--c); }
  .st-thumb .fr.claimed::after { content: ""; position: absolute; inset: 0; background: color-mix(in oklab, var(--c) 12%, transparent); }
  .st-thumb .fr.fetching, .st-thumb .fr.replaying { border-style: dashed; border-color: var(--info); display: flex; align-items: center; justify-content: center; }
  .st-thumb .fr.fetching .micro { color: var(--info); font-size: 0.5rem; }
  .st-thumb .fr.replaying canvas { opacity: 0.7; }
  .st-thumb.over .fr { opacity: 0.45; background: repeating-linear-gradient(135deg, var(--bg-elev) 0 6px, var(--panel) 6px 12px); }
  .st-thumb.conflict .fr { border-color: var(--warn); }
  .st-thumb .dot { position: absolute; left: 3px; bottom: 3px; width: 7px; height: 7px; border-radius: 2px; }
  .st-thumb .corner { position: absolute; right: 0; top: 0; width: 0; height: 0; border-top: 9px solid var(--warn); border-left: 9px solid transparent; }
  .st-sfxlane { display: flex; align-items: center; gap: 6px; }
  .st-sfxlane .lb { width: 24px; font-size: 0.5rem; }
  .st-sfxlane .cell { width: 64px; height: 8px; border-bottom: 1px solid var(--border-soft); position: relative; flex: none; }
  .st-sfxlane .cell i { position: absolute; left: 4px; top: 0; width: 6px; height: 6px; border-radius: 50%; }
</style>
