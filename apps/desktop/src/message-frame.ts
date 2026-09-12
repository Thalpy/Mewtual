export const MESSAGE_FRAME_MOTIONS = [
  "none", "glide", "fly", "pop", "drift",
  "wipe", "split", "blinds", "checker", "bars", "wheel", "dissolve",
  "blackout", "newsflash", "swivel", "flip", "spiral", "crawl",
  "bounce", "boomerang", "slam", "quake",
] as const;
export type MessageFrameMotion = (typeof MESSAGE_FRAME_MOTIONS)[number];

export type MessageFrameMotionFamily = "quiet" | "reveal" | "ceremony" | "physical";

/**
 * What the shared arrival knobs mean for one motion. `distance` names the job the 4..80 slider
 * does ("none" greys it out), `fade` and `direction` say whether starting visibility and the
 * entry vector apply at all, and `vector` is the pair of labels the direction toggle shows.
 */
export type MessageFrameMotionTraits = {
  family: MessageFrameMotionFamily;
  distance: "travel" | "depth" | "grain" | "amplitude" | "spin" | "none";
  fade: boolean;
  direction: boolean;
  vector: [string, string];
};

const TRAITS: Record<MessageFrameMotion, MessageFrameMotionTraits> = {
  none: { family: "quiet", distance: "none", fade: false, direction: false, vector: ["", ""] },
  glide: { family: "quiet", distance: "travel", fade: true, direction: true, vector: ["↑ from above", "from below ↓"] },
  fly: { family: "quiet", distance: "travel", fade: true, direction: true, vector: ["← from left", "from right →"] },
  pop: { family: "quiet", distance: "depth", fade: true, direction: false, vector: ["centred", "centred"] },
  drift: { family: "quiet", distance: "travel", fade: true, direction: true, vector: ["↖ drift left", "drift right ↗"] },
  wipe: { family: "reveal", distance: "none", fade: false, direction: true, vector: ["← from the right", "from the left →"] },
  split: { family: "reveal", distance: "none", fade: false, direction: true, vector: ["↔ open sideways", "open up and down ↕"] },
  blinds: { family: "reveal", distance: "grain", fade: false, direction: true, vector: ["∥ vertical slats", "horizontal slats ≡"] },
  checker: { family: "reveal", distance: "grain", fade: false, direction: false, vector: ["", ""] },
  bars: { family: "reveal", distance: "grain", fade: false, direction: false, vector: ["", ""] },
  wheel: { family: "reveal", distance: "none", fade: false, direction: true, vector: ["↺ anticlockwise", "clockwise ↻"] },
  dissolve: { family: "reveal", distance: "grain", fade: false, direction: false, vector: ["", ""] },
  blackout: { family: "ceremony", distance: "none", fade: false, direction: false, vector: ["", ""] },
  newsflash: { family: "ceremony", distance: "spin", fade: true, direction: true, vector: ["↺ spin left", "spin right ↻"] },
  swivel: { family: "ceremony", distance: "none", fade: true, direction: true, vector: ["hinge on the right", "hinge on the left"] },
  flip: { family: "ceremony", distance: "none", fade: true, direction: true, vector: ["hinge at the bottom", "hinge at the top"] },
  spiral: { family: "ceremony", distance: "travel", fade: true, direction: true, vector: ["↖ from top left", "from top right ↗"] },
  crawl: { family: "ceremony", distance: "travel", fade: true, direction: false, vector: ["", ""] },
  bounce: { family: "physical", distance: "travel", fade: true, direction: true, vector: ["↑ dropped from above", "tossed from below ↓"] },
  boomerang: { family: "physical", distance: "travel", fade: true, direction: true, vector: ["← from left", "from right →"] },
  slam: { family: "physical", distance: "depth", fade: true, direction: false, vector: ["", ""] },
  quake: { family: "physical", distance: "amplitude", fade: true, direction: false, vector: ["", ""] },
};

export function messageFrameMotionTraits(id: MessageFrameMotion): MessageFrameMotionTraits {
  return TRAITS[id] ?? TRAITS.none;
}

export const MESSAGE_FRAME_SHAPES = ["terminal", "bracket", "packet", "holo", "signal"] as const;
export type MessageFrameShape = (typeof MESSAGE_FRAME_SHAPES)[number];

export const MESSAGE_FRAME_EFFECTS = ["none", "scan", "pulse", "trace", "flicker"] as const;
export type MessageFrameEffect = (typeof MESSAGE_FRAME_EFFECTS)[number];
export const MESSAGE_FRAME_EFFECT_IDS = ["scan", "pulse", "trace", "flicker"] as const;
export type MessageFrameEffectId = (typeof MESSAGE_FRAME_EFFECT_IDS)[number];

export const MESSAGE_FRAME_EASINGS = ["soft", "snappy", "spring"] as const;
export type MessageFrameEasing = (typeof MESSAGE_FRAME_EASINGS)[number];

export type MessageFrameEffectOptions = {
  speed: number;
  intensity: number;
  amount: number;
  direction: -1 | 1;
};

export type MessageFrameLayer = {
  id: MessageFrameEffectId;
  enabled: boolean;
  options: MessageFrameEffectOptions;
};

export type MessageFrameArrival = {
  duration: number;
  distance: number;
  fade: number;
  direction: -1 | 1;
  easing: MessageFrameEasing;
};

export type MessageFrame = {
  surface: string;
  opacity: number;
  edge: number;
  motion: MessageFrameMotion;
  shape: MessageFrameShape;
  effects: MessageFrameLayer[];
  arrival: MessageFrameArrival;
};

export type FrameMessage = {
  author: string;
  ts: number;
  reply_to?: string;
};

export type MessageFramePosition = "single" | "start" | "middle" | "end";

export type MessageFrameScanGeometry = { offset: number; height: number };

/**
 * Temporary rollout gate for custom message backgrounds in the live chat log.
 *
 * Keep the frame codec, profile data, editor, and preview intact while this is false: existing
 * choices continue to round-trip and the feature can be restored without a migration. The chat
 * renderer also uses this gate to avoid installing the frame scan observers while frames are off.
 */
export const CHAT_MESSAGE_FRAMES_ENABLED = false;

export const DEFAULT_MESSAGE_FRAME: MessageFrame = {
  surface: "",
  opacity: 56,
  edge: 68,
  motion: "none",
  shape: "terminal",
  effects: [],
  arrival: { duration: 480, distance: 30, fade: 12, direction: 1, easing: "soft" },
};

const FRAME_PREFIX_V1 = "mf1";
const FRAME_PREFIX_V2 = "mf2";
const FRAME_PREFIX_V3 = "mf3";
const MAX_FRAME_VALUE_LENGTH = 512;
const MAX_FRAME_LAYERS = 4;
const MOTIONS = new Set<string>(MESSAGE_FRAME_MOTIONS);
const SHAPES = new Set<string>(MESSAGE_FRAME_SHAPES);
const EFFECTS = new Set<string>(MESSAGE_FRAME_EFFECTS);
const EFFECT_IDS = new Set<string>(MESSAGE_FRAME_EFFECT_IDS);
const EASINGS = new Set<string>(MESSAGE_FRAME_EASINGS);

const EFFECT_DEFAULTS: Record<MessageFrameEffectId, MessageFrameEffectOptions> = {
  scan: { speed: 5, intensity: 68, amount: 2, direction: 1 },
  pulse: { speed: 5, intensity: 64, amount: 38, direction: 1 },
  trace: { speed: 5, intensity: 78, amount: 34, direction: 1 },
  flicker: { speed: 5, intensity: 58, amount: 32, direction: 1 },
};

function defaultFrame(): MessageFrame {
  return {
    ...DEFAULT_MESSAGE_FRAME,
    effects: [],
    arrival: { ...DEFAULT_MESSAGE_FRAME.arrival },
  };
}

export function defaultMessageFrameLayer(id: MessageFrameEffectId): MessageFrameLayer {
  return { id, enabled: true, options: { ...EFFECT_DEFAULTS[id] } };
}

function safeSurface(raw: unknown): string | null {
  if (typeof raw !== "string" || raw.length > MAX_FRAME_VALUE_LENGTH) return null;
  const surface = raw.trim();
  if (!surface) return "";
  if (!/^[#a-z0-9 ,.%()-]+$/i.test(surface)) return null;
  if (/url|expression|image|var\(/i.test(surface)) return null;
  return surface;
}

/**
 * A motion id this build does not know arrives as Still, never as a rejected frame: the motion
 * catalogue grows over time, and a peer on a newer build must not lose their whole surface and
 * chassis on older screens just because they picked an arrival that was added after this build.
 */
function knownMotion(raw: string): MessageFrameMotion {
  return MOTIONS.has(raw) ? (raw as MessageFrameMotion) : "none";
}

function boundedInteger(raw: unknown, min: number, max: number, fallback: number): number {
  const value = typeof raw === "number" ? raw : Number(raw);
  return Number.isFinite(value) ? Math.min(max, Math.max(min, Math.round(value))) : fallback;
}

function sanitizeArrival(raw: Partial<MessageFrameArrival> | undefined): MessageFrameArrival {
  const d = DEFAULT_MESSAGE_FRAME.arrival;
  return {
    duration: boundedInteger(raw?.duration, 240, 1200, d.duration),
    distance: boundedInteger(raw?.distance, 4, 80, d.distance),
    fade: boundedInteger(raw?.fade, 0, 80, d.fade),
    direction: Number(raw?.direction) < 0 ? -1 : 1,
    easing: EASINGS.has(raw?.easing ?? "") ? raw!.easing! : d.easing,
  };
}

function effectAmount(id: MessageFrameEffectId, raw: unknown): number {
  const d = EFFECT_DEFAULTS[id].amount;
  if (id === "scan") return boundedInteger(raw, 1, 8, d);
  if (id === "trace") return boundedInteger(raw, 10, 70, d);
  if (id === "pulse") return boundedInteger(raw, 10, 80, d);
  return boundedInteger(raw, 5, 80, d);
}

function sanitizeLayer(raw: Partial<MessageFrameLayer> | undefined): MessageFrameLayer | null {
  if (!raw || typeof raw.id !== "string" || !EFFECT_IDS.has(raw.id)) return null;
  const id = raw.id as MessageFrameEffectId;
  const d = EFFECT_DEFAULTS[id];
  return {
    id,
    enabled: raw.enabled !== false,
    options: {
      speed: boundedInteger(raw.options?.speed, 1, 10, d.speed),
      intensity: boundedInteger(raw.options?.intensity, 20, 100, d.intensity),
      amount: effectAmount(id, raw.options?.amount),
      direction: Number(raw.options?.direction) < 0 ? -1 : 1,
    },
  };
}

function sanitizeLayers(raw: readonly Partial<MessageFrameLayer>[] | undefined): MessageFrameLayer[] {
  const seen = new Set<MessageFrameEffectId>();
  const clean: MessageFrameLayer[] = [];
  for (const candidate of raw ?? []) {
    const layer = sanitizeLayer(candidate);
    if (!layer || seen.has(layer.id)) continue;
    seen.add(layer.id);
    clean.push(layer);
    if (clean.length === MAX_FRAME_LAYERS) break;
  }
  return clean;
}

function parseCompactLayers(raw: string): MessageFrameLayer[] {
  if (!raw) return [];
  return sanitizeLayers(raw.split(";").map((entry) => {
    const [id, enabled, speed, intensity, amount, direction] = entry.split(".");
    return {
      id: id as MessageFrameEffectId,
      enabled: enabled !== "0",
      options: { speed: Number(speed), intensity: Number(intensity), amount: Number(amount), direction: Number(direction) < 0 ? -1 : 1 },
    };
  }));
}

function encodeCompactLayers(layers: readonly MessageFrameLayer[]): string {
  return sanitizeLayers(layers).map((layer) => {
    const o = layer.options;
    return `${layer.id}.${layer.enabled ? 1 : 0}.${o.speed}.${o.intensity}.${o.amount}.${o.direction}`;
  }).join(";");
}

/**
 * Decode the opaque profile field. Plain historical colours/gradients and mf1 values remain
 * valid; mf2 added catalogued chassis/effect ids and mf3 adds ordered effect layers plus an
 * arrival recipe without changing the shared profile schema.
 */
export function parseMessageFrame(raw: unknown): MessageFrame {
  if (typeof raw !== "string" || raw.length > MAX_FRAME_VALUE_LENGTH) return defaultFrame();
  const value = raw.trim();
  if (!value) return defaultFrame();
  if (
    !value.startsWith(`${FRAME_PREFIX_V1}|`) &&
    !value.startsWith(`${FRAME_PREFIX_V2}|`) &&
    !value.startsWith(`${FRAME_PREFIX_V3}|`)
  ) {
    const surface = safeSurface(value);
    return surface === null ? defaultFrame() : { ...defaultFrame(), surface };
  }

  const parts = value.split("|");
  if (parts[0] === FRAME_PREFIX_V1) {
    if (parts.length !== 5) return defaultFrame();
    const surface = safeSurface(parts[1]);
    if (surface === null) return defaultFrame();
    return {
      ...defaultFrame(),
      surface,
      opacity: boundedInteger(parts[2], 20, 90, DEFAULT_MESSAGE_FRAME.opacity),
      edge: boundedInteger(parts[3], 0, 100, DEFAULT_MESSAGE_FRAME.edge),
      motion: knownMotion(parts[4]),
    };
  }

  if (parts[0] === FRAME_PREFIX_V2) {
    if (parts.length !== 7) return defaultFrame();
    const surface = safeSurface(parts[1]);
    if (
      surface === null ||
      !SHAPES.has(parts[5]) ||
      !EFFECTS.has(parts[6])
    ) return defaultFrame();
    return {
      ...defaultFrame(),
      surface,
      opacity: boundedInteger(parts[2], 20, 90, DEFAULT_MESSAGE_FRAME.opacity),
      edge: boundedInteger(parts[3], 0, 100, DEFAULT_MESSAGE_FRAME.edge),
      motion: knownMotion(parts[4]),
      shape: parts[5] as MessageFrameShape,
      effects: parts[6] === "none" ? [] : [defaultMessageFrameLayer(parts[6] as MessageFrameEffectId)],
    };
  }

  if (parts.length !== 8) return defaultFrame();
  const surface = safeSurface(parts[1]);
  if (
    surface === null ||
    !SHAPES.has(parts[5])
  ) return defaultFrame();
  const [duration, distance, fade, direction, easing] = parts[6].split(".");
  return {
    surface,
    opacity: boundedInteger(parts[2], 20, 90, DEFAULT_MESSAGE_FRAME.opacity),
    edge: boundedInteger(parts[3], 0, 100, DEFAULT_MESSAGE_FRAME.edge),
    motion: knownMotion(parts[4]),
    shape: parts[5] as MessageFrameShape,
    effects: parseCompactLayers(parts[7]),
    arrival: sanitizeArrival({
      duration: Number(duration), distance: Number(distance), fade: Number(fade),
      direction: Number(direction) < 0 ? -1 : 1,
      easing: easing as MessageFrameEasing,
    }),
  };
}

/** Pack studio settings back into the existing profile field. */
export function encodeMessageFrame(frame: MessageFrame): string {
  const surface = safeSurface(frame.surface);
  if (surface === null) return "";
  const opacity = boundedInteger(frame.opacity, 20, 90, DEFAULT_MESSAGE_FRAME.opacity);
  const edge = boundedInteger(frame.edge, 0, 100, DEFAULT_MESSAGE_FRAME.edge);
  const motion = MOTIONS.has(frame.motion) ? frame.motion : DEFAULT_MESSAGE_FRAME.motion;
  const shape = SHAPES.has(frame.shape) ? frame.shape : DEFAULT_MESSAGE_FRAME.shape;
  const effects = sanitizeLayers(frame.effects);
  const arrival = sanitizeArrival(frame.arrival);
  const defaultArrival = DEFAULT_MESSAGE_FRAME.arrival;
  const arrivalIsDefault = arrival.duration === defaultArrival.duration &&
    arrival.distance === defaultArrival.distance && arrival.fade === defaultArrival.fade &&
    arrival.direction === defaultArrival.direction && arrival.easing === defaultArrival.easing;
  if (!surface && motion === "none" && shape === DEFAULT_MESSAGE_FRAME.shape && !effects.length && arrivalIsDefault) return "";
  const packedArrival = `${arrival.duration}.${arrival.distance}.${arrival.fade}.${arrival.direction}.${arrival.easing}`;
  return `${FRAME_PREFIX_V3}|${surface}|${opacity}|${edge}|${motion}|${shape}|${packedArrival}|${encodeCompactLayers(effects)}`;
}

/**
 * Turn a validated frame into the inline custom properties consumed by the chat renderer.
 * Text is never made translucent: opacity belongs only to the frame's generated background.
 */
export function messageFrameStyle(raw: unknown): string {
  const frame = parseMessageFrame(raw);
  if (!frame.surface) return "";
  return `--message-surface:${frame.surface};--message-opacity:${frame.opacity / 100};--message-edge:${frame.edge}%`;
}

/** A rollout gate or local viewer preference suppresses peer frames without changing stored data. */
export function visibleMessageFrameStyle(
  raw: unknown,
  framesDisabled: boolean,
  isOwn = false,
  featureEnabled = true,
): string {
  if (!featureEnabled) return "";
  return framesDisabled && !isOwn ? "" : messageFrameStyle(raw);
}

export function messageFrameShape(raw: unknown): MessageFrameShape {
  return parseMessageFrame(raw).shape;
}

export function messageFrameEffect(raw: unknown): MessageFrameEffect {
  return parseMessageFrame(raw).effects.find((layer) => layer.enabled)?.id ?? "none";
}

export function messageFrameLayerStyle(layer: MessageFrameLayer): string {
  const clean = sanitizeLayer(layer);
  if (!clean) return "";
  const { speed, intensity, amount } = clean.options;
  const duration = 4.2 - ((speed - 1) / 9) * 3.45;
  const alpha = intensity / 100;
  const low = Math.max(0.03, alpha * (1 - amount / 100));
  return [
    `--frame-fx-duration:${duration.toFixed(2)}s`,
    `--frame-fx-alpha:${alpha.toFixed(2)}`,
    `--frame-fx-low:${low.toFixed(2)}`,
    `--frame-fx-amount:${amount}`,
    `--frame-fx-size:${amount}px`,
    `--frame-fx-half:${amount / 2}px`,
    `--frame-fx-glow:${amount * 2}px`,
  ].join(";");
}

export function messageFrameArrivalStyle(raw: unknown): string {
  const frame = parseMessageFrame(raw);
  const { duration, distance, fade, direction, easing } = frame.arrival;
  let x = 0;
  let y = 0;
  if (frame.motion === "glide") y = direction * distance;
  if (frame.motion === "fly") x = direction * distance;
  if (frame.motion === "drift") {
    x = direction * distance;
    y = -Math.round(distance * 0.35);
  }
  if (frame.motion === "spiral") {
    x = direction * Math.round(distance * 2.3);
    y = -distance;
  }
  if (frame.motion === "crawl") y = Math.round(distance * 1.7);
  if (frame.motion === "bounce") y = -direction * Math.round(distance * 1.2);
  if (frame.motion === "boomerang") x = direction * distance * 3;
  const scale = Math.max(0.84, 1 - distance / 600);
  const traits = messageFrameMotionTraits(frame.motion);
  // Reveals do not fade: they are fully painted behind their mask. Forcing the start opacity
  // to 1 keeps the shared keyframes honest when a viewer switches motion mid-preview.
  const opacity = traits.fade ? fade / 100 : 1;
  const curve = easing === "snappy"
    ? "cubic-bezier(0.2,0.95,0.25,1)"
    : easing === "spring" ? "cubic-bezier(0.18,0.9,0.2,1.12)" : "cubic-bezier(0.16,0.84,0.24,1)";
  // The one slider reads differently per family: slat/tile/speckle size for reveals, shake
  // amplitude for the quake, zoom depth for the slam and the newsflash spin.
  const grain = Math.max(6, Math.round(distance * 0.6));
  const shake = Math.max(1, Math.round(distance / 6));
  const zoom = (1 + distance / 25).toFixed(2);
  const spin = Math.round(180 + distance * 12);
  const clipFrom = frame.motion === "wipe"
    ? (direction < 0 ? "inset(0 0 0 100%)" : "inset(0 100% 0 0)")
    : frame.motion === "split"
      ? (direction < 0 ? "inset(0 50%)" : "inset(50% 0)")
      : "inset(0)";
  const blindDir = direction < 0 ? "to right" : "to bottom";
  const origin = frame.motion === "swivel"
    ? (direction < 0 ? "right center" : "left center")
    : frame.motion === "flip"
      ? (direction < 0 ? "bottom center" : "top center")
      : frame.motion === "crawl" ? "bottom center" : "center center";
  return [
    `--message-arrival-duration:${duration}ms`,
    `--message-arrival-cycle:${duration * 4}ms`,
    `--message-arrival-opacity:${opacity}`,
    `--message-arrival-x:${x}px`,
    `--message-arrival-y:${y}px`,
    `--message-arrival-scale:${scale.toFixed(3)}`,
    `--message-arrival-ease:${curve}`,
    `--message-arrival-dir:${direction}`,
    `--message-arrival-grain:${grain}px`,
    `--message-arrival-shake:${shake}px`,
    `--message-arrival-zoom:${zoom}`,
    `--message-arrival-spin:${spin}deg`,
    `--message-arrival-clip-from:${clipFrom}`,
    `--message-arrival-blind-dir:${blindDir}`,
    `--message-arrival-origin:${origin}`,
  ].join(";");
}

/** Position a clipped frame beam in the one coordinate system shared by the chat viewport. */
export function messageFrameScanGeometry(
  viewportTop: number,
  viewportHeight: number,
  bodyTop: number,
): MessageFrameScanGeometry {
  return {
    offset: Number.isFinite(bodyTop - viewportTop) ? bodyTop - viewportTop : 0,
    height: Number.isFinite(viewportHeight) ? Math.max(1, viewportHeight) : 1,
  };
}

/** The sender's validated arrival motion, independent of whether they use a coloured frame. */
export function messageFrameMotion(raw: unknown): MessageFrameMotion {
  return parseMessageFrame(raw).motion;
}

/**
 * Resolve one newly inserted row's motion from that row author's profile. Callers pass the
 * author's own opaque frame value; there is deliberately no room for a global/default motion.
 */
export function visibleMessageFrameMotion(
  authorFrame: unknown,
  isNewArrival: boolean,
  motionsDisabled: boolean,
): MessageFrameMotion {
  return !isNewArrival || motionsDisabled ? "none" : messageFrameMotion(authorFrame);
}

function joinsPrevious(messages: readonly FrameMessage[], index: number, breaks: ReadonlySet<number>): boolean {
  if (index <= 0 || index >= messages.length || breaks.has(index)) return false;
  const current = messages[index];
  const previous = messages[index - 1];
  const elapsed = current.ts - previous.ts;
  return !current.reply_to && current.author === previous.author && elapsed >= 0 && elapsed < 300_000;
}

/**
 * Locate a row inside its same-author run. `breaks` contains row indexes preceded by a day or
 * unread divider, matching the visual grouping rules in the message log.
 */
export function messageFramePosition(
  messages: readonly FrameMessage[],
  index: number,
  breaks: ReadonlySet<number> = new Set<number>(),
): MessageFramePosition {
  if (index < 0 || index >= messages.length) return "single";
  const before = joinsPrevious(messages, index, breaks);
  const after = joinsPrevious(messages, index + 1, breaks);
  if (before && after) return "middle";
  if (before) return "end";
  if (after) return "start";
  return "single";
}
