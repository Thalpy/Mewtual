<script lang="ts">
  // The flipnote editor surface (design-creative-suite.md 2.2, 2.10; mockups "Flipnote Editor",
  // "Timeline States", "Art / Sound / Music"), connected to the native Studio through the shared
  // session: reads are typed views, edits are publish-then-apply saves that keep their identity
  // across retries, and what the backend does not offer yet (claims, sound, linked Music, .pixa
  // export, durable edits while rotating, signed repair) is shown as unavailable rather than
  // pretended.
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
    FLIPNOTE_FPS_MAX,
    FLIPNOTE_FPS_MIN,
    FLIPNOTE_FRAME_BYTES_PROMISE,
    FLIPNOTE_H,
    FLIPNOTE_MAX_FRAMES,
    FLIPNOTE_MAX_PATCHES,
    FLIPNOTE_W,
    LAYER_NAMES,
    PIX_ROLE_NAMES,
    type PixPaletteEntry,
  } from "./studio-contract.ts";
  import { DEFAULT_PALETTE, PALETTE_LABELS } from "./studio-store.ts";
  import { base64ToBytes, type FrameConflictValue, type FrameView, type NativeExpiry, type RecoveryMode, type RecoveryVersion } from "./studio-native.ts";
  import { reason, type RecoveryItem, type SaveRecord } from "./studio-session.ts";
  import { ensureStudio, setStudioScope, studio } from "./studio-state.svelte.ts";

  let { me, server, channel, nameOf, colorOf, adapt = true, onnotice } = $props<{
    me: string;
    server: number | null;
    channel: string;
    nameOf: (identity: string) => string;
    colorOf: (identity: string) => string;
    adapt?: boolean;
    onnotice: (text: string, kind: "info" | "warn" | "error") => void;
  }>();

  // Built once per mount (state may be written during init, never inside a derived); the
  // surface remounts with the tab, and the session outlives it.
  // svelte-ignore state_referenced_locally
  const session = ensureStudio(me);
  $effect(() => { setStudioScope(server, channel); });
  // The sidebar selects; the surface follows the selection (and re-opens after a remount).
  $effect(() => {
    const id = studio.selected;
    if (id && session.doc?.object !== id) session.open(id);
    else if (!id && session.doc) session.open(null);
  });

  const objectId = $derived(studio.selected);
  const doc = $derived.by(() => { void studio.rev; return session.doc?.object === objectId ? session.doc : null; });
  const view = $derived(doc?.view ?? null);
  const model = $derived(doc?.model ?? null);
  const entry = $derived.by(() => {
    void studio.rev;
    const m = session.indexModel;
    return m ? [...m.entries, ...m.overflow].find((e) => e.id === objectId) ?? null : null;
  });
  const isScore = $derived(entry?.kind === "score");
  const readOnlyWhy = $derived.by(() => { void studio.rev; if (!objectId) return ""; const ep = session.editableEpoch(objectId); return "refused" in ep ? ep.refused : ""; });
  const pending = $derived.by(() => { void studio.rev; return objectId ? session.pendingFor(objectId) : []; });
  const uncertain = $derived(pending.filter((s) => s.status === "uncertain"));
  const inflight = $derived(pending.some((s) => s.status !== "uncertain"));
  const pendingInserts = $derived(pending.filter((s): s is Extract<SaveRecord, { kind: "frame" }> => s.kind === "frame" && s.op === "insert"));
  const receivePaused = $derived.by(() => { void studio.rev; return session.receivePaused; });

  function who(id: string): string { return id === me ? "you" : nameOf(id); }
  function tint(id: string): string { return id === me ? "var(--accent)" : colorOf(id); }

  // --- Editor state ------------------------------------------------------------------------------
  let frameId = $state("");
  let raster = $state.raw<PixRaster | null>(null);
  let rasterCid = $state(""); // which selected value the raster was loaded from (or "unsaved")
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

  const frames = $derived<FrameView[]>(model?.frames ?? []);
  const frameIds = $derived(frames.map((f) => f.id));
  const frameIndex = $derived(frameIds.indexOf(frameId));
  const frameRec = $derived(frames.find((f) => f.id === frameId) ?? null);
  const pendingHere = $derived(pendingInserts.find((s) => s.frame === frameId) ?? null);
  const frameKnown = $derived(frameIndex >= 0 || pendingHere !== null);
  const totalBytes = $derived(model?.declaredFrameBytes ?? 0);
  const overCapCount = $derived(model?.overCapCount ?? 0);
  const canEdit = $derived(!!raster && !!model && !isScore && !readOnlyWhy && overCapCount === 0);
  const hasScope = $derived.by(() => { void studio.rev; return session.scope !== null; });
  // Blob state is session data, not part of the model: read it through the revision so the
  // strip and the veil follow a fetch as it lands.
  function blobStateOf(cid: string) { void studio.rev; return session.blobState(cid); }
  function blobProblemOf(cid: string) { void studio.rev; return session.blobProblem(cid); }
  const blobStateHere = $derived(frameRec ? blobStateOf(frameRec.cid) : "held");
  function sameBytes(a: Uint8Array | undefined, b: Uint8Array): boolean {
    if (!a || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }

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
  function openFrame(id: string) {
    if (!model) return;
    commit();
    frameId = id;
    loadRaster();
  }

  /// Load the raster from the newest unsaved pixels for this frame, else from the held blob for
  /// its selected value; ask for the blob (top priority) when neither is here yet.
  function loadRaster() {
    const rec = frames.find((f) => f.id === frameId) ?? null;
    const unsaved = objectId && frameId ? session.unsavedPix(objectId, frameId) : null;
    const bytes = unsaved ?? (rec ? session.blob(rec.cid) : undefined);
    if (!bytes) {
      raster = null;
      rasterCid = "";
      if (rec) session.want(rec.cid, rec.bytes, 0);
      return;
    }
    let img;
    try { img = decodePix(bytes); } catch (e) { raster = null; rasterCid = ""; onnotice(`frame pixels rejected: ${reason(e)}`, "warn"); return; }
    const r = new PixRaster(img.w, img.h, img.palette);
    r.loadFlat(img.pixels);
    raster = r;
    rasterCid = unsaved ? "unsaved" : rec?.cid ?? "";
    undo.clear();
    dirty = false;
    paintRev++;
  }

  /// Write the raster back as a replace_frame save if anything changed (2.9: an op only when a
  /// value changed; bursts are coalesced by committing on stroke end, not per cell). The save
  /// keeps the pixels until it lands; a failure shows up in the save card, never as lost work.
  function commit() {
    if (!raster || !frameId || !dirty || !model || !objectId) return;
    try {
      session.saveFrame(objectId, frameId, raster.encode());
      rasterCid = "unsaved";
    } catch (e) {
      onnotice(reason(e), "warn");
    }
    dirty = false;
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
    if (!canvasEl || !raster) return null;
    const rect = canvasEl.getBoundingClientRect();
    const x = Math.floor(((e.clientX - rect.left) / rect.width) * raster.w);
    const y = Math.floor(((e.clientY - rect.top) / rect.height) * raster.h);
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
    if (!canvasEl || !model) return;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;
    const w = raster?.w ?? FLIPNOTE_W, h = raster?.h ?? FLIPNOTE_H;
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
      const prevRec = frames[frameIndex - 1];
      const prev = session.blob(prevRec.cid);
      if (!prev) session.want(prevRec.cid, prevRec.bytes, 1);
      else {
        try {
          const img = decodePix(prev);
          const rgba = pixToRgba(img, resolve);
          const paper = 0;
          for (let i = 0; i < img.pixels.length; i++) if (img.pixels[i] === paper) rgba[i * 4 + 3] = 0;
          const s = scratchCanvas(img.w, img.h);
          s.putImageData(new ImageData(rgba, img.w, img.h), 0, 0);
          ctx.globalAlpha = 0.3;
          ctx.drawImage(scratch!, 0, 0, canvasEl.width, canvasEl.height);
          ctx.globalAlpha = 1;
        } catch { /* an invalid neighbour is reported on its own thumbnail */ }
      }
    }
  }

  function drawOverlay() {
    if (!overlayEl || !model) return;
    const ctx = overlayEl.getContext("2d");
    if (!ctx) return;
    const w = raster?.w ?? FLIPNOTE_W, h = raster?.h ?? FLIPNOTE_H;
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
    if (model && (!frameId || !frameKnown)) {
      const first = frameIds[0] ?? pendingInserts[0]?.frame;
      if (first) openFrame(first);
      else { frameId = ""; raster = null; rasterCid = ""; }
    } else if (!model && frameId) { frameId = ""; raster = null; rasterCid = ""; }
  });
  $effect(() => {
    // Pixels arrived for the open frame, or its selected value changed under us while we were
    // not drawing: load them. A dirty raster keeps the member's strokes; the save decides.
    void studio.rev;
    if (!frameId || dirty || !objectId) return;
    const rec = frames.find((f) => f.id === frameId);
    const unsaved = session.unsavedPix(objectId, frameId);
    const wanted = unsaved ? "unsaved" : rec?.cid ?? "";
    if (!wanted || wanted === rasterCid) return;
    // Our own save just landed: the selected value now names the bytes this raster already
    // holds, so adopt the cid and keep the undo history instead of reloading.
    if (rasterCid === "unsaved" && raster && rec && sameBytes(session.blob(rec.cid), raster.encode())) { rasterCid = rec.cid; return; }
    if (unsaved || (rec && session.blob(rec.cid))) loadRaster();
    else if (rec) session.want(rec.cid, rec.bytes, 0);
  });
  $effect(() => {
    // The recovery rail follows the open document.
    if (objectId && !isScore) session.watchRecovery(objectId);
  });

  /// Thumbnail action: decode the frame's held bytes into a small canvas, or ask for them.
  function thumb(node: HTMLCanvasElement, arg: { cid: string; bytes: number; rev: number }) {
    const draw = (a: { cid: string; bytes: number; rev: number }) => {
      const ctx = node.getContext("2d");
      if (!ctx) return;
      ctx.imageSmoothingEnabled = false;
      ctx.clearRect(0, 0, node.width, node.height);
      const bytes = session.blob(a.cid);
      if (!bytes) { session.want(a.cid, a.bytes, 2); return; }
      try {
        const img = decodePix(bytes);
        const s = scratchCanvas(img.w, img.h);
        s.putImageData(new ImageData(pixToRgba(img, resolve), img.w, img.h), 0, 0);
        ctx.drawImage(scratch!, 0, 0, node.width, node.height);
      } catch { /* shown as invalid by the frame's state */ }
    };
    draw(arg);
    return { update: draw };
  }

  /// A pending insert's thumbnail comes from its own unsaved pixels.
  function thumbBytes(node: HTMLCanvasElement, arg: { bytes: Uint8Array; rev: number }) {
    const draw = (a: { bytes: Uint8Array; rev: number }) => {
      const ctx = node.getContext("2d");
      if (!ctx) return;
      ctx.imageSmoothingEnabled = false;
      try {
        const img = decodePix(a.bytes);
        const s = scratchCanvas(img.w, img.h);
        s.putImageData(new ImageData(pixToRgba(img, resolve), img.w, img.h), 0, 0);
        ctx.drawImage(scratch!, 0, 0, node.width, node.height);
      } catch { /* nothing to show */ }
    };
    draw(arg);
    return { update: draw };
  }

  // --- Timeline actions ------------------------------------------------------------------------
  function guard(fn: () => void) {
    try { fn(); } catch (e) { onnotice(reason(e), "warn"); }
  }

  function blankPix(): Uint8Array {
    return new PixRaster(FLIPNOTE_W, FLIPNOTE_H, raster?.palette.map((e) => ({ ...e })) ?? DEFAULT_PALETTE.map((e) => ({ ...e }))).encode();
  }

  function addFrameAfter(copy: boolean) {
    if (!model || !objectId) return;
    commit();
    guard(() => {
      const bytes = copy && raster ? raster.encode() : blankPix();
      const id = session.insertFrame(objectId, frameIndex >= 0 ? frameId : null, bytes);
      frameId = id;
      loadRaster();
    });
  }

  function deleteFrame() {
    if (!model || !frameId || !objectId) return;
    if (pendingHere) { onnotice("this frame is still being saved; wait for it or discard the save below", "info"); return; }
    const idx = frameIndex;
    const gone = frameId;
    guard(() => session.removeFrame(objectId, gone));
    const next = frameIds.filter((f) => f !== gone)[Math.max(0, Math.min(idx, frameIds.length - 2))];
    frameId = "";
    raster = null;
    rasterCid = "";
    if (next) openFrame(next);
  }

  function step(delta: number) {
    if (!frameIds.length) return;
    const n = frameIds.length;
    let i = (Math.max(0, frameIndex) + delta + n) % n;
    // Playback skips frames that are over the cap or not held; stepping does not.
    if (playing) {
      let guardN = n;
      while (guardN-- && (frames[i].overCap || !session.blob(frames[i].cid))) i = (i + delta + n) % n;
    }
    if (!loop && playing && i === 0 && delta > 0) { playing = false; return; }
    openFrame(frameIds[i]);
  }

  let playTimer: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (playTimer) { clearInterval(playTimer); playTimer = null; }
    const fps = model?.fps ?? 12;
    if (playing && model) playTimer = setInterval(() => step(1), 1000 / Math.max(FLIPNOTE_FPS_MIN, Math.min(FLIPNOTE_FPS_MAX, fps)));
    return () => { if (playTimer) clearInterval(playTimer); };
  });

  function setFps(v: number) {
    if (!objectId || !Number.isInteger(v) || v < FLIPNOTE_FPS_MIN || v > FLIPNOTE_FPS_MAX) { onnotice("fps is 1 to 24", "warn"); return; }
    guard(() => session.setFps(objectId, v));
  }
  function setTitle(v: string) {
    if (!objectId || !model) return;
    const t = v.trim();
    if (t && t !== model.title) guard(() => session.setTitle(objectId, t));
  }

  // --- Conflicts: every live value of a frame's pixels, each usable on purpose -----------------
  async function useVersion(alt: FrameConflictValue, how: "replace" | "insertAfter") {
    if (!objectId || !frameId) return;
    commit();
    try { await session.useVersion(objectId, frameId, alt, how); } catch (e) { onnotice(reason(e), "warn"); }
  }

  // --- Saves: retry the same request, re-author on purpose, or discard on purpose ---------------
  function retrySave(s: SaveRecord) { guard(() => session.retry(s.id)); }
  function reauthorSave(s: SaveRecord) { guard(() => session.reauthor(s.id)); }
  let discardArmed = $state(0);
  function discardSave(s: SaveRecord) {
    if (discardArmed !== s.id) { discardArmed = s.id; return; }
    discardArmed = 0;
    session.discard(s.id);
    if (s.kind === "frame" && s.frame === frameId) { rasterCid = ""; loadRaster(); }
  }

  // --- Index entry: expiry and deletion are Index operations ------------------------------------
  function setExpiry(e: NativeExpiry) { if (objectId) guard(() => session.setEntryExpiry(objectId, e)); }
  let deleteArmed = $state(false);
  function deleteFlipnote() {
    if (!objectId) return;
    commit();
    guard(() => { session.deleteEntry(objectId); deleteArmed = false; studio.selected = ""; });
  }
  function expiryDate(e: NativeExpiry): string {
    if (e.kind !== "at") return "";
    const d = new Date(e.ms);
    return Number.isFinite(d.getTime()) ? d.toISOString().slice(0, 10) : "";
  }
  function expiryText(e: NativeExpiry): string {
    if (e.kind === "never") return "keeps forever";
    if (e.kind === "unrecorded") return "no expiry recorded";
    const d = Math.ceil((e.ms - Date.now()) / 86_400_000);
    return d <= 0 ? `expired ${new Date(e.ms).toLocaleDateString()}` : `expires in ${d}d`;
  }

  // --- Recovery rail -------------------------------------------------------------------------------
  const listing = $derived.by(() => { void studio.rev; return session.recoveryTarget === objectId ? session.recoveryListing : null; });
  const recoveryError = $derived.by(() => { void studio.rev; return session.recoveryError; });
  const run = $derived.by(() => { void studio.rev; return session.recoveryRun; });
  const pointer = $derived.by(() => { void studio.rev; return session.pointer; });
  let inspected = $state("");
  let copyConfirm = $state("");
  const inspectedVersion = $derived.by(() => { void studio.rev; return inspected ? session.versions.get(inspected) ?? null : null; });
  function exportFor(snapshot: string) { void studio.rev; return session.exports.get(snapshot) ?? null; }
  async function inspect(v: RecoveryVersion) {
    inspected = inspected === v.snapshot ? "" : v.snapshot;
    if (!inspected) return;
    try { await session.readVersion(v.snapshot); } catch (e) { onnotice(reason(e), "warn"); }
  }
  async function startRecovery(v: RecoveryVersion, mode: RecoveryMode) {
    if (mode === "copy" && copyConfirm !== v.snapshot) { copyConfirm = v.snapshot; return; }
    copyConfirm = "";
    commit();
    try { await session.runRecovery(v.snapshot, mode); } catch (e) { onnotice(reason(e), "warn"); }
  }
  async function exportVersion(v: RecoveryVersion) {
    try { await session.exportVersion(v.snapshot); onnotice("backup ready below · p1-recovery-v1, not a .pixa", "info"); } catch (e) { onnotice(reason(e), "warn"); }
  }
  function saveExport(snapshot: string) {
    const x = session.exports.get(snapshot);
    if (!x) return;
    try {
      const url = URL.createObjectURL(new Blob([base64ToBytes(x.bytesB64)], { type: "application/octet-stream" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = `${(model?.title || "index").replace(/[^\w.-]+/g, "_")}-epoch-${listing?.versions.find((v) => v.snapshot === snapshot)?.epoch ?? "x"}.p1-recovery-v1.bin`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
    } catch (e) { onnotice(`could not hand the file to the browser: ${reason(e)}; copy it instead`, "warn"); }
  }
  async function copyExport(snapshot: string) {
    const x = session.exports.get(snapshot);
    if (!x) return;
    try { await navigator.clipboard.writeText(x.bytesB64); onnotice("backup copied as base64", "info"); } catch (e) { onnotice(reason(e), "warn"); }
  }
  async function acknowledge() {
    try { await session.acknowledgeEviction(); onnotice("eviction acknowledged: the named oldest version is removed at the next settlement pass", "info"); } catch (e) { onnotice(reason(e), "warn"); }
  }
  function deadlineText(deadlineMs: string): string {
    // Display only: the lossless decimal stays in the listing; the request never carries it.
    const ms = Number(deadlineMs);
    if (!Number.isFinite(ms)) return `at receiver clock ${deadlineMs}`;
    const left = ms - Date.now();
    if (left <= 0) return "at the next settlement pass";
    const d = Math.floor(left / 86_400_000), h = Math.floor((left % 86_400_000) / 3_600_000);
    return d ? `in ${d}d ${h}h` : `in ${h}h`;
  }
  function reasonText(r: RecoveryVersion["reason"]): string {
    return r === "excluded" ? "left out by a checkpoint" : r === "rewound" ? "rewound" : r === "conflictOverflow" ? "conflict overflow" : "repair";
  }
  function itemTone(s: RecoveryItem["state"]): string {
    if (s === "applied" || s === "alreadySaved" || s === "unchanged") return "ok";
    if (s === "error" || s === "deleted" || s === "missingTarget") return "danger";
    if (s === "conflict" || s === "full") return "warn";
    return "info";
  }

  onMount(() => {
    const t = setInterval(() => { tick++; }, 1000);
    return () => clearInterval(t);
  });
  onDestroy(() => { commit(); });

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
    void tick;
    const d = Date.now() - ts;
    if (d < 60_000) return "just now";
    if (d < 3_600_000) return `${Math.round(d / 60_000)} min ago`;
    if (d < 86_400_000) return `${Math.round(d / 3_600_000)}h ago`;
    return `${Math.round(d / 86_400_000)}d ago`;
  }
  function settlementChip(): { text: string; tone: string; title: string } {
    if (!doc) return { text: "", tone: "", title: "" };
    if (!view) {
      if (doc.error) return { text: "read failed", tone: "danger", title: doc.error };
      if (doc.absent && !pending.some((s) => s.kind === "create")) return { text: "not on this device", tone: "warn", title: "studio_read found no local copy; this is not a deletion and not proof nobody has it" };
      return { text: "reading…", tone: "", title: "" };
    }
    if (view.awaitingTenureReceipt) return { text: "read-only preview", tone: "warn", title: "current owner has not yet confirmed this document's history; no phase, receipt or publication is claimed" };
    if (view.phase === "fault") return { text: "history fault", tone: "danger", title: "history fault: conflicting owner receipts" };
    if (view.phase === "closing") return { text: "rotating", tone: "warn", title: "rotating: durable edits resume when the owner settles this rotation" };
    if (view.phase === "settled") return { text: `settled · epoch ${view.epoch}`, tone: "ok", title: "stored phase settled; a settled phase is not a receipt for every displayed edit" };
    return { text: `open · epoch ${view.epoch}`, tone: "", title: "open: edits save locally and provisionally; an owner receipt settles them later" };
  }
</script>

<!-- Hotkeys apply while focus is inside the editor (the tool rail, the canvas, the timeline);
     typing in one of its inputs is exempt inside onKey. -->
<svelte:window onkeydown={(e) => { if (rootEl && rootEl.contains(document.activeElement)) onKey(e); }} />
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="studio" bind:this={rootEl} role="application" aria-label="Flipnote editor" tabindex="0">
  {#if !objectId || (!model && !doc)}
    <div class="studio-empty">
      {#if !hasScope}
        <p class="muted">Open a channel to see its studio.</p>
      {:else}
        <p class="muted">Pick a flipnote on the left, or start a new one.</p>
      {/if}
    </div>
  {:else if isScore}
    <div class="studio-empty">
      <p class="muted">The score editor is not built yet. This entry lives in the studio index as a score; open a flipnote on the left to draw.</p>
    </div>
  {:else if !model}
    <div class="studio-empty">
      {#if doc?.error}
        <div class="st-banner danger">could not read this flipnote: {doc.error} <button type="button" class="st-btn" onclick={() => session.open(objectId)}>retry</button></div>
      {:else if doc?.absent && !pending.length}
        <p class="muted">This flipnote is not on this device yet. Absence is not a deletion: it may still arrive from another member.</p>
        <button type="button" class="st-btn" onclick={() => session.open(objectId)}>read again</button>
      {:else}
        <p class="muted">{pending.some((s) => s.kind === "create") ? "Creating…" : "Reading…"}</p>
        {#each uncertain as s (s.id)}
          <div class="st-card warn">
            <span>{s.label} · uncertain</span>
            <span class="micro wrap">{s.error}</span>
            <span class="st-card-acts">
              <button type="button" class="st-btn primary" onclick={() => retrySave(s)}>retry same request</button>
              <button type="button" class="st-btn ghost" onclick={() => discardSave(s)}>{discardArmed === s.id ? "discard for real" : "discard"}</button>
            </span>
          </div>
        {/each}
      {/if}
    </div>
  {:else}
    <!-- Header -->
    <div class="st-head">
      <input class="st-title" value={model.title} placeholder="untitled" disabled={!!readOnlyWhy} onchange={(e) => setTitle(e.currentTarget.value)} aria-label="Flipnote title" />
      {#if model.titleConflicts.length}<span class="st-chip warn" title="another title was set at the same time by {model.titleConflicts.map((c) => who(c.source.author)).join(', ')}: {model.titleConflicts.map((c) => c.value).join(' / ')}"><i></i><span class="micro">title conflict</span></span>{/if}
      <span class="micro">flipnote · {raster?.w ?? FLIPNOTE_W}×{raster?.h ?? FLIPNOTE_H}</span>
      <label class="st-chip" title="Frames per second, 1 to 24{model.fpsConflicts.length ? ' · another value was set concurrently' : ''}">
        <input type="number" min={FLIPNOTE_FPS_MIN} max={FLIPNOTE_FPS_MAX} value={model.fps} disabled={!!readOnlyWhy} onchange={(e) => setFps(Number(e.currentTarget.value))} />
        <span class="micro">fps</span>
      </label>
      <span class="st-chip {settlementChip().tone}" title={settlementChip().title}><i></i><span class="micro">{settlementChip().text}</span></span>
      {#if inflight}<span class="st-chip" title="a save is in flight; unsaved pixels stay here until it lands"><i class="pulse"></i><span class="micro">saving…</span></span>{/if}
      <span class="grow"></span>
      <button type="button" class="st-tg" class:on={adaptOn} onclick={() => (adaptOn = !adaptOn)} title="Role colours follow your theme; literals never move"><i></i><span class="micro">adapt to my theme</span></button>
      <button type="button" class="st-btn" onclick={() => (playing = !playing)}>{playing ? "pause" : "play"}</button>
      <button type="button" class="st-btn" title="Posting a frame into the channel as a doodle is not connected yet" disabled>
        <svg viewBox="0 0 16 16" style="width: 11px; height: 11px"><path d="M2 8l12-6-4 12-2.5-4.5z"></path></svg>
        post to chat
      </button>
      <button type="button" class="st-btn primary" title=".pixa export arrives with the backend's Gate 6; recovery backups are exported from the music tab" disabled>export .pixa</button>
    </div>

    {#if uncertain.length}
      <div class="st-banner warn st-save">
        <span><b>save uncertain</b> · {uncertain[0].label}{#if uncertain.length > 1} · and {uncertain.length - 1} more{/if}</span>
        <span class="micro wrap">{uncertain[0].error}{#if uncertain[0].epochChanged} · the document moved to a newer epoch since; a retry resends the original request and the backend decides{/if} · attempt {uncertain[0].attempts} · your work is kept</span>
        <span class="st-card-acts">
          <button type="button" class="st-btn primary" onclick={() => retrySave(uncertain[0])}>retry same request</button>
          {#if uncertain[0].kind === "frame" || uncertain[0].kind === "apply" || uncertain[0].kind === "applyIndex"}
            <button type="button" class="st-btn" title="a new operation with a fresh nonce against the current epoch; only if you are sure the first never committed" disabled={!!readOnlyWhy} onclick={() => reauthorSave(uncertain[0])}>save again as a new edit</button>
          {/if}
          <button type="button" class="st-btn ghost" onclick={() => discardSave(uncertain[0])}>{discardArmed === uncertain[0].id ? "discard for real" : "discard"}</button>
        </span>
      </div>
    {/if}
    {#if overCapCount}
      <div class="st-banner warn">document full: {overCapCount} frame{overCapCount === 1 ? "" : "s"} past the {FLIPNOTE_MAX_FRAMES}-frame list or the 8 MiB promise. Editing is refused until they are trimmed; playback skips them.</div>
    {:else if readOnlyWhy}
      <div class="st-banner {view?.phase === 'fault' ? 'danger' : 'warn'}">{readOnlyWhy}</div>
    {/if}
    {#if receivePaused}
      <div class="st-banner info">receiving is paused for this server after a storage or scan problem. Nothing local is lost and nothing is settled or unsettled by this; a successful read or save resumes it. <button type="button" class="st-btn" onclick={() => session.open(objectId)}>read again</button></div>
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
        <div class="st-canvas-wrap" class:locked={!canEdit} style="width: {(raster?.w ?? FLIPNOTE_W) * zoom}px; aspect-ratio: {raster?.w ?? FLIPNOTE_W} / {raster?.h ?? FLIPNOTE_H}" onwheel={(e) => { if (e.ctrlKey) { e.preventDefault(); setZoom(zoom + (e.deltaY < 0 ? 1 : -1)); } }}>
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
          {#if !frameId}
            <div class="st-veil"><span>no frames yet</span><span class="micro">add one below to start drawing</span></div>
          {:else if !raster && frameRec}
            {#if blobStateHere === "invalid" || blobStateHere === "unavailable"}
              <div class="st-veil"><span>{blobStateHere === "invalid" ? "these pixels were rejected" : "pixels not available yet"}</span><span class="micro">{blobProblemOf(frameRec.cid) || "bounded by the declared size"} · {fmtKib(frameRec.bytes)}</span><span class="st-veil-acts"><button type="button" class="st-btn" onclick={() => { session.invalidate({ objects: [objectId] }); }}>ask again</button></span></div>
            {:else}
              <div class="st-veil"><span>fetching {fmtKib(frameRec.bytes)}</span><span class="micro">bounded by the declared size · validated before it is shown</span></div>
            {/if}
          {:else if pendingHere}
            <div class="st-veil soft"><span class="micro">{pendingHere.status === "uncertain" ? "this frame's save is uncertain · see the card above" : "saving this frame…"}</span></div>
          {/if}
          <span class="st-readout left">{zoom}× · {hover ? `${hover[0]},${hover[1]}` : "…"}{#if tool === "shape"} · {shape}{/if}</span>
          <span class="st-readout right">layer · {LAYER_NAMES[layer]}{#if dirty} · unsaved{:else if rasterCid === "unsaved"} · saving{/if}</span>
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

            <div class="st-sec nxt">frame {frameIndex >= 0 ? frameIndex + 1 : "·"}</div>
            {#if frameRec?.conflicts.length}
              <div class="st-card warn">
                <span>another version by {#each frameRec.conflicts as c, i}{i ? ", " : ""}<b style="color: {tint(c.author)}">{who(c.author)}</b>{/each}</span>
                <span class="micro">replaced this frame at the same time · nothing resolved silently · pick one on purpose</span>
                <span class="st-alt"><i style="background: {tint(frameRec.author)}"></i><span>showing {who(frameRec.author)} · {rel(frameRec.ts)} · {fmtKib(frameRec.bytes)}</span><span class="grow"></span><button type="button" class="st-link" disabled={!!readOnlyWhy} onclick={() => useVersion({ cid: frameRec!.cid, bytes: frameRec!.bytes, author: frameRec!.author, ts: frameRec!.ts, opId: frameRec!.opId }, "replace")}>keep this</button></span>
                {#each frameRec.conflicts as c (c.opId)}
                  <span class="st-alt"><i style="background: {tint(c.author)}"></i><span>{who(c.author)} · {rel(c.ts)} · {fmtKib(c.bytes)}</span><span class="grow"></span><button type="button" class="st-link" disabled={!!readOnlyWhy} onclick={() => useVersion(c, "replace")}>use this</button><button type="button" class="st-link" disabled={!!readOnlyWhy} onclick={() => useVersion(c, "insertAfter")}>keep both</button></span>
                {/each}
              </div>
            {/if}
            {#if pendingHere}
              <div class="st-card accent">
                <span>New frame{#if pendingHere.status === "uncertain"} · save uncertain{:else} · saving{/if}</span>
                <span class="micro it">it joins the timeline when the save lands</span>
              </div>
            {:else if frameRec}
              <p class="micro">by {who(frameRec.author)} · {rel(frameRec.ts)} · {fmtKib(frameRec.bytes)}{#if frameRec.insertions > 1} · inserted {frameRec.insertions}× concurrently{/if}{#if frameRec.overCap} · over cap{/if}</p>
            {/if}
            <p class="micro wrap it">claims (who is drawing what) arrive with the call draw channel; nothing here is a lock</p>
          {:else if inspectorTab === "sound"}
            <div class="st-sec">frame {frameIndex >= 0 ? frameIndex + 1 : "·"}</div>
            <p class="muted small">No sound on this frame.</p>
            <button type="button" class="st-btn ghost dashed" disabled title="Emoji sounds on frames arrive with the backend's Gate 6">+ add emoji sound</button>
            <p class="micro wrap">a sound is a jam patch on an emoji, played by your own synth · not accepted by the studio yet</p>
          {:else}
            <div class="st-sec">soundtrack</div>
            <p class="muted small">No score linked. Linked Music arrives with the backend's Gate 6.</p>
            <p class="micro wrap">patches 0 of {FLIPNOTE_MAX_PATCHES} · score patches first, then sfx</p>
            <div class="st-sec nxt">size</div>
            <div class="st-bar"><i style="width: {Math.min(100, (totalBytes / FLIPNOTE_FRAME_BYTES_PROMISE) * 100)}%"></i></div>
            <p class="micro wrap">{fmtKib(totalBytes)} of 8 mib declared · {frames.length} of {FLIPNOTE_MAX_FRAMES} frames{#if model.deletedFrames.length} · {model.deletedFrames.length} deleted kept in history{/if}</p>

            <div class="st-sec nxt">entry</div>
            {#if entry}
              <div class="st-opt"><span class="lb">expiry</span>
                <button type="button" class="st-tile txt" class:on={entry.expiry.kind === "never"} disabled={!!readOnlyWhy} onclick={() => setExpiry({ kind: "never" })}>keeps</button>
                <button type="button" class="st-tile txt" class:on={entry.expiry.kind === "unrecorded"} disabled={!!readOnlyWhy} onclick={() => setExpiry({ kind: "unrecorded" })}>unset</button>
                <input type="date" class="st-input date" class:on={entry.expiry.kind === "at"} value={expiryDate(entry.expiry)} disabled={!!readOnlyWhy} onchange={(e) => { const ms = Date.parse(e.currentTarget.value); if (Number.isFinite(ms)) setExpiry({ kind: "at", ms }); }} aria-label="expiry date" />
              </div>
              <p class="micro wrap">{expiryText(entry.expiry)}{#if entry.expiryConflicts} · another expiry was set at the same time{/if} · recorded now, enforced later</p>
              <div class="st-opt"><span class="lb">delete</span>
                {#if deleteArmed}
                  <button type="button" class="st-btn danger" onclick={deleteFlipnote}>delete this flipnote</button>
                  <button type="button" class="st-btn ghost" onclick={() => (deleteArmed = false)}>cancel</button>
                {:else}
                  <button type="button" class="st-btn ghost" disabled={!!readOnlyWhy} onclick={() => (deleteArmed = true)}>delete…</button>
                {/if}
              </div>
              <p class="micro wrap">a deletion is a tombstone in the channel index: the entry leaves the list, its history stays in recovery</p>
            {:else}
              <p class="micro wrap">this flipnote has no entry in the channel index on this device{#if pending.some((s) => s.kind === "create")} · it appears when the create lands{/if}</p>
            {/if}

            <div class="st-sec nxt">history</div>
            {#if view}
              <p class="st-hist"><i class={view.awaitingTenureReceipt ? "warn" : view.phase === "settled" ? "ok" : view.phase === "fault" ? "danger" : view.phase === "closing" ? "warn" : "info"}></i>
                {#if view.awaitingTenureReceipt}awaiting the owner's tenure receipt · epoch {view.epoch}{:else}{view.phase} · epoch {view.epoch} · saved locally, provisional{/if}
              </p>
              <p class="micro wrap">{view.awaitingTenureReceipt ? "current owner has not yet confirmed this document's history" : view.phase === "closing" ? "rotating: local edits only until the owner settles this rotation" : view.phase === "fault" ? "history fault: conflicting owner receipts" : view.phase === "settled" ? "the owner settled this epoch; later edits are provisional until the next receipt" : "an owner receipt settles these edits later"}</p>
            {/if}
            {#if listing}
              {#if listing.pendingIntents}<p class="micro wrap">{listing.pendingIntents} of your saves await an owner receipt · not lost, not excluded</p>{/if}
              {#if listing.evictionPending}
                <div class="st-card warn">
                  <span>a previous version will be removed {deadlineText(listing.evictionPending.deadlineMs)} unless exported</span>
                  <span class="micro wrap">oldest {listing.evictionPending.oldestSnapshot.slice(0, 8)} makes room for staged {listing.evictionPending.stagedSnapshot.slice(0, 8)} · acknowledging removes it now instead of at the deadline</span>
                  <span class="st-card-acts"><button type="button" class="st-btn" onclick={acknowledge}>acknowledge · remove now</button></span>
                </div>
              {/if}
              {#each listing.versions as v (v.snapshot)}
                <div class="st-ver" class:staged={v.staged}>
                  <p class="st-hist"><i class="info"></i>{v.staged ? "staged" : "previous"} version · epoch {v.epoch} · {reasonText(v.reason)} · {fmtKib(v.bytes)}</p>
                  <span class="st-card-acts">
                    <button type="button" class="st-btn ghost" onclick={() => inspect(v)}>{inspected === v.snapshot ? "hide" : "inspect"}</button>
                    <button type="button" class="st-btn" disabled={!!readOnlyWhy || run?.status === "running"} title="add what is missing; never overwrites" onclick={() => startRecovery(v, "restore")}>restore</button>
                    <button type="button" class="st-btn" disabled={!!readOnlyWhy || run?.status === "running"} title="replace current title/fps and apply the fork's deletions; asks first" onclick={() => startRecovery(v, "copy")}>{copyConfirm === v.snapshot ? "confirm copy" : "copy"}</button>
                    <button type="button" class="st-btn ghost" onclick={() => exportVersion(v)}>export</button>
                  </span>
                  {#if copyConfirm === v.snapshot}<p class="micro wrap">copy replaces the current title and fps with this version's and applies deletions recorded on that fork · nothing deleted here is resurrected · press confirm copy</p>{/if}
                  {#if inspected === v.snapshot}
                    {#if inspectedVersion}
                      {@const c = inspectedVersion.content}
                      <p class="micro wrap">{#if c.kind === "flipnote"}title "{c.title?.selected.value ?? ""}" · {c.fps?.selected.value ?? 12} fps · {c.timeline.length} frames · {Object.keys(c.tombstones).length} deleted · {fmtKib(c.declaredFrameBytes)} declared{:else}index · {Object.keys(c.objects).length} entries · {Object.keys(c.overflow).length} overflow · {Object.keys(c.deletedObjects).length} deleted{/if} · historical, not the current view</p>
                    {:else}
                      <p class="micro wrap">reading…</p>
                    {/if}
                  {/if}
                  {#if exportFor(v.snapshot)}
                    {@const x = exportFor(v.snapshot)!}
                    <div class="st-card">
                      <span>backup ready · {fmtKib(x.bytes)}</span>
                      <span class="micro wrap">{x.format} · private history and operation evidence for recovery, not playable media or a .pixa</span>
                      <span class="st-card-acts"><button type="button" class="st-btn" onclick={() => saveExport(v.snapshot)}>save file</button><button type="button" class="st-btn ghost" onclick={() => copyExport(v.snapshot)}>copy base64</button></span>
                    </div>
                  {/if}
                </div>
              {:else}
                <p class="micro wrap">no previous versions retained for this document{#if listing.source === null} · no local source{/if}</p>
              {/each}
              {#if run && run.object === objectId}
                <div class="st-card">
                  <span>{run.mode} from epoch {listing.versions.find((v) => v.snapshot === run.snapshot)?.epoch ?? "?"} · {run.status === "running" ? "in progress" : run.status === "stopped" ? "stopped" : "walked"}</span>
                  <span class="micro wrap">each choice is previewed, applied only when ready, then the document is re-read · what was applied is saved content, not an all-or-nothing restore</span>
                  <ul class="st-run">
                    {#each run.items as it, i (i)}
                      <li><i class={itemTone(it.state)}></i><span>{it.label}</span><span class="grow"></span><span class="micro">{it.state}{#if it.error} · {it.error}{/if}</span></li>
                    {/each}
                  </ul>
                  {#if run.status === "running"}<span class="st-card-acts"><button type="button" class="st-btn ghost" onclick={() => session.stopRecovery()}>stop after this item</button></span>{/if}
                </div>
                {#if run.status !== "running"}
                  <div class="st-card">
                    <span>registry pointer · {pointer.status === "done" ? "restored" : pointer.status === "blocked" ? "blocked" : pointer.status === "running" ? "restoring…" : "pending"}</span>
                    <span class="micro wrap">{#if pointer.status === "done" && pointer.result}checkpoint epoch {pointer.result.checkpointEpoch} · registry epoch {pointer.result.registryEpochId.slice(0, 8)} · provisional{:else if pointer.status === "blocked"}{pointer.error} · content already saved stays saved; retry separately{:else}a separate step that makes the saved document discoverable again; retryable on its own{/if}</span>
                    <span class="st-card-acts"><button type="button" class="st-btn" disabled={pointer.status === "running"} onclick={() => session.restorePointer(objectId)}>{pointer.status === "blocked" ? "retry pointer" : "restore pointer"}</button></span>
                  </div>
                {/if}
              {/if}
            {:else if recoveryError}
              <p class="micro wrap">recovery listing failed: {recoveryError}</p>
            {/if}
          {/if}
        </div>
      </div>
    </div>

    <!-- Timeline -->
    <div class="st-timeline">
      <div class="st-transport">
        <button type="button" class="st-tool sm" title="first frame" onclick={() => frameIds[0] && openFrame(frameIds[0])}><svg viewBox="0 0 16 16" class="solid"><path d="M8 3 2 8l6 5zM14 3 8 8l6 5z"></path></svg></button>
        <button type="button" class="st-tool sm" title="previous frame [,]" onclick={() => step(-1)}><svg viewBox="0 0 16 16" class="solid"><path d="M3 3h1.5v10H3zM12.5 3 6 8l6.5 5z"></path></svg></button>
        <button type="button" class="st-tool play" class:on={playing} title={playing ? "pause [space]" : "play [space]"} onclick={() => (playing = !playing)}>
          {#if playing}<svg viewBox="0 0 16 16" class="solid"><path d="M3.5 2.5h3.5v11H3.5zM9 2.5h3.5v11H9z"></path></svg>{:else}<svg viewBox="0 0 16 16" class="solid"><path d="M4 2.5 13 8 4 13.5z"></path></svg>{/if}
        </button>
        <button type="button" class="st-tool sm" title="next frame [.]" onclick={() => step(1)}><svg viewBox="0 0 16 16" class="solid"><path d="M3.5 3 10 8l-6.5 5zM11.5 3H13v10h-1.5z"></path></svg></button>
        <button type="button" class="st-tool sm" title="last frame" onclick={() => frameIds.length && openFrame(frameIds[frameIds.length - 1])}><svg viewBox="0 0 16 16" class="solid"><path d="M2 3l6 5-6 5zM8 3l6 5-6 5z"></path></svg></button>
        <button type="button" class="st-tool sm" class:on={loop} title="loop" onclick={() => (loop = !loop)}><svg viewBox="0 0 16 16"><path d="M3 8a5 5 0 015-5h4M13 8a5 5 0 01-5 5H4"></path><path d="M10.5 1.5 12 3l-1.5 1.5M5.5 11.5 4 13l1.5 1.5"></path></svg></button>
        <span class="mono strong">{frameIndex >= 0 ? frameIndex + 1 : "·"} / {frames.length}</span>
        <span class="micro">{((Math.max(0, frameIndex) + 1) / Math.max(1, model.fps)).toFixed(2)} s of {(frames.length / Math.max(1, model.fps)).toFixed(2)} s</span>
        <span class="grow"></span>
        <button type="button" class="st-btn ghost" disabled={!!readOnlyWhy || overCapCount > 0} onclick={() => addFrameAfter(false)}>+ frame after</button>
        <button type="button" class="st-btn ghost" disabled={!canEdit} onclick={() => addFrameAfter(true)}>duplicate</button>
        <button type="button" class="st-btn ghost" disabled={!frameId || !!readOnlyWhy || !!pendingHere} onclick={deleteFrame}>delete</button>
      </div>
      <div class="st-strip">
        {#each frames as f, i (f.id)}
          {@const st = blobStateOf(f.cid)}
          <button type="button" class="st-thumb" class:cur={f.id === frameId} class:over={!!f.overCap} class:conflict={f.conflicts.length > 0} onclick={() => openFrame(f.id)} title={f.overCap ? "over the cap: skipped in playback, greyed until trimmed" : st === "invalid" ? `pixels rejected: ${blobProblemOf(f.cid)}` : st === "unavailable" ? "pixels not available yet" : ""}>
            <span class="ix">{i + 1}</span>
            <span class="fr" class:fetching={st === "fetching" || st === "queued"} class:missing={st === "unavailable" || st === "invalid"}>
              {#if st === "held"}
                <canvas width="64" height="48" use:thumb={{ cid: f.cid, bytes: f.bytes, rev: studio.rev }}></canvas>
              {:else if st === "invalid"}
                <span class="micro">rejected</span>
              {:else if st === "unavailable"}
                <span class="micro">not here</span>
              {:else}
                <canvas width="64" height="48" use:thumb={{ cid: f.cid, bytes: f.bytes, rev: studio.rev }}></canvas>
                <span class="micro tag">fetching</span>
              {/if}
              <i class="dot" style="background: {tint(f.author)}"></i>
              {#if f.conflicts.length}<i class="corner"></i>{/if}
            </span>
          </button>
        {/each}
        {#each pendingInserts as s (s.id)}
          <button type="button" class="st-thumb" class:cur={s.frame === frameId} onclick={() => openFrame(s.frame)} title={s.status === "uncertain" ? `save uncertain: ${s.error}` : "saving this new frame"}>
            <span class="ix">{s.status === "uncertain" ? "?" : "…"}</span>
            <span class="fr pending" class:uncertain={s.status === "uncertain"}>
              <canvas width="64" height="48" use:thumbBytes={{ bytes: s.pix, rev: studio.rev }}></canvas>
              <i class="dot" style="background: var(--accent)"></i>
            </span>
          </button>
        {/each}
      </div>
      <div class="st-sfxlane">
        <span class="micro lb">sfx</span>
        {#each frames as f (f.id)}
          <span class="cell"></span>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .studio { display: flex; flex-direction: column; flex: 1; min-height: 0; outline: none; gap: 0; }
  .studio-empty { padding: 2rem; display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
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
  .st-title:disabled { color: var(--text-2); }
  .st-chip { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 2px 9px; white-space: nowrap; }
  .st-chip i { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); }
  .st-chip i.pulse { background: var(--accent); animation: st-pulse 1s ease-in-out infinite alternate; }
  @keyframes st-pulse { from { opacity: 0.3; } to { opacity: 1; } }
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
  .st-btn.danger { background: var(--danger-dim); border-color: var(--danger-brd); color: var(--danger); }
  .st-btn.dashed { border-style: dashed; }
  .st-btn:disabled { opacity: 0.45; }
  .st-banner { margin: 0.2rem 0; padding: 6px 10px; border-radius: var(--r); font-size: 0.78rem; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .st-banner.warn { background: var(--warn-dim); border: 1px solid var(--warn-brd); color: var(--text-2); }
  .st-banner.danger { background: var(--danger-dim); border: 1px solid var(--danger-brd); color: var(--text-2); }
  .st-banner.info { background: var(--bg-elev); border: 1px solid var(--border); color: var(--text-2); }
  .st-banner.st-save { flex-direction: column; align-items: flex-start; gap: 3px; }
  .st-banner.st-save .micro.wrap { margin: 0; }

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
  .st-veil-acts, .st-card-acts { display: flex; gap: 6px; margin-top: 4px; flex-wrap: wrap; }
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
  .st-tile:disabled { opacity: 0.45; }
  .st-tile svg { width: 14px; height: 14px; }
  .st-input { flex: 1; min-width: 0; background: var(--bg-elev); border: 1px solid var(--border); border-radius: var(--r); color: var(--text); font-family: var(--mono); font-size: 0.72rem; padding: 3px 6px; }
  .st-input.date { flex: 0 1 9rem; height: 26px; box-sizing: border-box; color-scheme: dark; }
  .st-input.date.on { border-color: var(--accent); background: var(--accent-dim); }
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
  .st-alt { display: flex; align-items: center; gap: 6px; font-size: 0.74rem; color: var(--text-2); }
  .st-alt i { width: 7px; height: 7px; border-radius: 2px; flex: none; }
  .st-link { background: transparent; border: none; padding: 0; font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); text-decoration: underline; }
  .st-link.danger { color: var(--danger); }
  .st-link:disabled { opacity: 0.45; text-decoration: none; }
  .st-bar { height: 4px; border-radius: 999px; background: var(--bg-elev); overflow: hidden; }
  .st-bar i { display: block; height: 100%; background: var(--accent); }
  .st-hist { display: flex; align-items: center; gap: 6px; margin: 2px 0; font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); }
  .st-hist i { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .st-hist i.ok { background: var(--ok); } .st-hist i.warn { background: var(--warn); } .st-hist i.danger { background: var(--danger); } .st-hist i.info { background: var(--info); }
  .st-ver { display: flex; flex-direction: column; gap: 3px; padding: 6px 0; border-top: 1px dashed var(--border-soft); }
  .st-ver.staged .st-hist { color: var(--warn); }
  .st-run { list-style: none; margin: 4px 0 0; padding: 0; display: flex; flex-direction: column; gap: 2px; max-height: 160px; overflow-y: auto; }
  .st-run li { display: flex; align-items: center; gap: 6px; font-size: 0.72rem; color: var(--text-2); }
  .st-run li i { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .st-run li i.ok { background: var(--ok); } .st-run li i.warn { background: var(--warn); } .st-run li i.danger { background: var(--danger); } .st-run li i.info { background: var(--info); }

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
  .st-thumb .fr.fetching, .st-thumb .fr.pending { border-style: dashed; border-color: var(--info); display: flex; align-items: center; justify-content: center; }
  .st-thumb .fr.missing { border-style: dashed; border-color: var(--warn); display: flex; align-items: center; justify-content: center; }
  .st-thumb .fr.fetching .micro, .st-thumb .fr.missing .micro { color: var(--info); font-size: 0.5rem; }
  .st-thumb .fr.missing .micro { color: var(--warn); }
  .st-thumb .fr .tag { position: absolute; left: 0; right: 0; bottom: 0; text-align: center; background: color-mix(in oklab, var(--bg-0) 70%, transparent); line-height: 1.6; }
  .st-thumb .fr.pending canvas { opacity: 0.7; }
  .st-thumb .fr.pending.uncertain { border-color: var(--warn); }
  .st-thumb.over .fr { opacity: 0.45; background: repeating-linear-gradient(135deg, var(--bg-elev) 0 6px, var(--panel) 6px 12px); }
  .st-thumb.conflict .fr { border-color: var(--warn); }
  .st-thumb .dot { position: absolute; left: 3px; bottom: 3px; width: 7px; height: 7px; border-radius: 2px; }
  .st-thumb .corner { position: absolute; right: 0; top: 0; width: 0; height: 0; border-top: 9px solid var(--warn); border-left: 9px solid transparent; }
  .st-sfxlane { display: flex; align-items: center; gap: 6px; }
  .st-sfxlane .lb { width: 24px; font-size: 0.5rem; }
  .st-sfxlane .cell { width: 64px; height: 8px; border-bottom: 1px solid var(--border-soft); position: relative; flex: none; }
</style>
