// Name effects travel through the existing opaque profile `effect` string. Keeping the stack in
// that field avoids a wire-format migration; every value is validated again before it reaches CSS.

export type NameEffectId =
  | "gradient"
  | "rainbow"
  | "neon"
  | "wave"
  | "mexican"
  | "pulse"
  | "outline"
  | "shadow"
  | "retro"
  | "glitch"
  | "shimmer"
  | "sparkle"
  | "wobble"
  | "candy"
  | "ghost"
  | "fire"
  | "extrude"
  | "typography"
  | "master";

export type NameEffectOptions = {
  speed?: number;
  direction?: number;
  glow?: number;
  intensity?: number;
  height?: number;
  spread?: number;
  depth?: number;
  width?: number;
  color?: string;
  x?: number;
  y?: number;
  blur?: number;
  opacity?: number;
  offset?: number;
  stops?: string[];
  angle?: number;
  amount?: number;
  secondary?: string;
  weight?: number;
  italic?: boolean;
  uppercase?: boolean;
  tracking?: number;
  bubble?: number;
};

export type NameEffect = { id: NameEffectId; enabled: boolean; options: NameEffectOptions };

export const STACK_PREFIX = "fxs1:";
export const GRADIENT_RE = /^grad2-((?:[0-9a-f]{6})(?:-[0-9a-f]{6}){1,7})-(\d{1,3})(?:-a(\d{1,2})(r?))?$/;

const IDS = new Set<NameEffectId>([
  "gradient", "rainbow", "neon", "wave", "mexican", "pulse", "outline", "shadow", "retro", "glitch",
  "shimmer", "sparkle", "wobble", "candy", "ghost", "fire", "extrude", "typography", "master",
]);

const LEGACY_IDS = new Set<NameEffectId>([
  "gradient", "rainbow", "neon", "wave", "pulse", "outline", "retro", "glitch",
]);

const DEFAULTS: Record<NameEffectId, NameEffectOptions> = {
  gradient: { stops: ["#e879c0", "#977df2"], angle: 90, speed: 0, direction: 1 },
  rainbow: { speed: 4, direction: 1 },
  neon: { glow: 8, intensity: 70 },
  wave: { height: 3, speed: 5 },
  mexican: { height: 5, speed: 5, spread: 4, direction: 1 },
  pulse: { speed: 5, depth: 55 },
  outline: { width: 1, color: "#e879c0" },
  shadow: { x: 2, y: 2, blur: 3, opacity: 70, color: "#000000" },
  retro: { offset: 2, opacity: 80 },
  glitch: { spread: 1, opacity: 70 },
  shimmer: { speed: 5, intensity: 65, direction: 1 },
  sparkle: { speed: 4, intensity: 65 },
  wobble: { speed: 5, amount: 3 },
  candy: { speed: 0, angle: 45, color: "#ff8fc7", secondary: "#fff4fb", direction: 1 },
  ghost: { opacity: 62, blur: 1, glow: 6 },
  fire: { height: 5, intensity: 75, speed: 6 },
  extrude: { depth: 3, direction: 1, color: "#4b244a", opacity: 90 },
  typography: { weight: 600, italic: false, uppercase: false, tracking: 0, bubble: 0 },
  master: { intensity: 100, speed: 100 },
};

const n = (value: unknown, fallback: number, min: number, max: number) => {
  const parsed = typeof value === "number" ? value : Number(value);
  return Number.isFinite(parsed) ? Math.max(min, Math.min(max, parsed)) : fallback;
};

const hex = (value: unknown, fallback: string) =>
  typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value) ? value.toLowerCase() : fallback;

const bool = (value: unknown, fallback: boolean) => typeof value === "boolean" ? value : fallback;

export function defaultNameEffect(id: NameEffectId): NameEffect {
  return { id, enabled: true, options: { ...DEFAULTS[id], stops: DEFAULTS[id].stops?.slice() } };
}

function sanitizeEffect(value: unknown): NameEffect | null {
  if (!value || typeof value !== "object") return null;
  const record = value as { id?: unknown; enabled?: unknown; options?: unknown };
  if (typeof record.id !== "string" || !IDS.has(record.id as NameEffectId)) return null;
  const id = record.id as NameEffectId;
  const raw = record.options && typeof record.options === "object"
    ? record.options as Record<string, unknown>
    : {};
  const d = DEFAULTS[id];
  let options: NameEffectOptions;
  switch (id) {
    case "gradient": {
      const rawStops = Array.isArray(raw.stops) ? raw.stops : d.stops!;
      const stops = rawStops.slice(0, 8).map((v, i) => hex(v, d.stops![i % d.stops!.length]));
      while (stops.length < 2) stops.push(d.stops![stops.length]);
      options = {
        stops,
        angle: n(raw.angle, d.angle!, 0, 360),
        speed: n(raw.speed, d.speed!, 0, 10),
        direction: n(raw.direction, d.direction!, -1, 1) < 0 ? -1 : 1,
      };
      break;
    }
    case "rainbow":
      options = { speed: n(raw.speed, d.speed!, 1, 10), direction: n(raw.direction, 1, -1, 1) < 0 ? -1 : 1 };
      break;
    case "neon":
      options = { glow: n(raw.glow, d.glow!, 2, 18), intensity: n(raw.intensity, d.intensity!, 20, 100) };
      break;
    case "wave":
      options = { height: n(raw.height, d.height!, 1, 8), speed: n(raw.speed, d.speed!, 1, 10) };
      break;
    case "mexican":
      options = {
        height: n(raw.height, d.height!, 1, 10),
        speed: n(raw.speed, d.speed!, 1, 10),
        spread: n(raw.spread, d.spread!, 1, 10),
        direction: n(raw.direction, 1, -1, 1) < 0 ? -1 : 1,
      };
      break;
    case "pulse":
      options = { speed: n(raw.speed, d.speed!, 1, 10), depth: n(raw.depth, d.depth!, 15, 85) };
      break;
    case "outline":
      options = { width: n(raw.width, d.width!, 0.5, 3), color: hex(raw.color, d.color!) };
      break;
    case "shadow":
      options = {
        x: n(raw.x, d.x!, -8, 8), y: n(raw.y, d.y!, -8, 8), blur: n(raw.blur, d.blur!, 0, 16),
        opacity: n(raw.opacity, d.opacity!, 10, 100), color: hex(raw.color, d.color!),
      };
      break;
    case "retro":
      options = { offset: n(raw.offset, d.offset!, 1, 6), opacity: n(raw.opacity, d.opacity!, 20, 100) };
      break;
    case "glitch":
      options = { spread: n(raw.spread, d.spread!, 1, 5), opacity: n(raw.opacity, d.opacity!, 20, 100) };
      break;
    case "shimmer":
      options = {
        speed: n(raw.speed, d.speed!, 1, 10), intensity: n(raw.intensity, d.intensity!, 20, 100),
        direction: n(raw.direction, 1, -1, 1) < 0 ? -1 : 1,
      };
      break;
    case "sparkle":
      options = { speed: n(raw.speed, d.speed!, 1, 10), intensity: n(raw.intensity, d.intensity!, 20, 100) };
      break;
    case "wobble":
      options = { speed: n(raw.speed, d.speed!, 1, 10), amount: n(raw.amount, d.amount!, 1, 8) };
      break;
    case "candy":
      options = {
        speed: n(raw.speed, d.speed!, 0, 10), angle: n(raw.angle, d.angle!, 0, 360),
        color: hex(raw.color, d.color!), secondary: hex(raw.secondary, d.secondary!),
        direction: n(raw.direction, 1, -1, 1) < 0 ? -1 : 1,
      };
      break;
    case "ghost":
      options = {
        opacity: n(raw.opacity, d.opacity!, 20, 95), blur: n(raw.blur, d.blur!, 0, 3),
        glow: n(raw.glow, d.glow!, 2, 14),
      };
      break;
    case "fire":
      options = {
        height: n(raw.height, d.height!, 1, 10), intensity: n(raw.intensity, d.intensity!, 20, 100),
        speed: n(raw.speed, d.speed!, 1, 10),
      };
      break;
    case "extrude":
      options = {
        depth: n(raw.depth, d.depth!, 1, 7), direction: n(raw.direction, 1, -1, 1) < 0 ? -1 : 1,
        color: hex(raw.color, d.color!), opacity: n(raw.opacity, d.opacity!, 20, 100),
      };
      break;
    case "typography":
      options = {
        weight: n(raw.weight, d.weight!, 400, 900), italic: bool(raw.italic, d.italic!),
        uppercase: bool(raw.uppercase, d.uppercase!), tracking: n(raw.tracking, d.tracking!, -1, 6),
        bubble: n(raw.bubble, d.bubble!, 0, 3),
      };
      break;
    case "master":
      options = { intensity: n(raw.intensity, d.intensity!, 25, 175), speed: n(raw.speed, d.speed!, 25, 200) };
      break;
  }
  return { id, enabled: record.enabled !== false, options };
}

function dedupe(effects: NameEffect[]): NameEffect[] {
  const seen = new Set<NameEffectId>();
  return effects.filter((effect) => !seen.has(effect.id) && !!seen.add(effect.id)).slice(0, IDS.size);
}

/** Decode both current effect stacks and every previously shipped single-effect value. */
export function decodeNameEffects(value: string): NameEffect[] {
  if (!value || value === "none") return [];
  const gradient = GRADIENT_RE.exec(value);
  if (gradient) {
    return [sanitizeEffect({
      id: "gradient",
      options: {
        stops: gradient[1].split("-").map((stop) => `#${stop}`),
        angle: Number(gradient[2]),
        speed: gradient[3] ? Number(gradient[3]) : 0,
        direction: gradient[4] === "r" ? -1 : 1,
      },
    })!];
  }
  if (LEGACY_IDS.has(value as NameEffectId)) return [defaultNameEffect(value as NameEffectId)];
  if (!value.startsWith(STACK_PREFIX) || value.length > 4096) return [];
  try {
    const decoded = JSON.parse(decodeURIComponent(value.slice(STACK_PREFIX.length)));
    if (!Array.isArray(decoded)) return [];
    return dedupe(decoded.map(sanitizeEffect).filter((effect): effect is NameEffect => effect !== null));
  } catch {
    return [];
  }
}

/** Compact, URI-safe storage inside the existing opaque profile field. */
export function encodeNameEffects(effects: NameEffect[]): string {
  const clean = dedupe(effects.map(sanitizeEffect).filter((effect): effect is NameEffect => effect !== null));
  if (!clean.length) return "none";
  return STACK_PREFIX + encodeURIComponent(JSON.stringify(clean));
}

export const effectConfigured = (effects: NameEffect[], id: NameEffectId) => effects.some((effect) => effect.id === id);

export const effectEnabled = (effects: NameEffect[], id: NameEffectId) =>
  effects.some((effect) => effect.id === id && effect.enabled);

export function effectOptions(effects: NameEffect[], id: NameEffectId): NameEffectOptions {
  return effects.find((effect) => effect.id === id)?.options ?? DEFAULTS[id];
}

export const animatedEffect = (id: NameEffectId) =>
  id === "gradient" || id === "rainbow" || id === "wave" || id === "mexican" || id === "pulse" ||
  id === "shimmer" || id === "sparkle" || id === "wobble" || id === "candy" || id === "fire";

const duration = (speed: number, slow: number, fast: number) =>
  slow - ((Math.max(1, Math.min(10, speed)) - 1) / 9) * (slow - fast);

const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));

const rgba = (color: string, opacity: number) => {
  const match = /^#([0-9a-f]{6})$/i.exec(color);
  if (!match) return "rgba(0,0,0,0.7)";
  const value = Number.parseInt(match[1], 16);
  return `rgba(${value >> 16},${(value >> 8) & 255},${value & 255},${opacity / 100})`;
};

export function nameEffectClasses(effects: NameEffect[]): string {
  const enabled = effects.filter((effect) => effect.enabled);
  if (!enabled.length) return "";
  const classes = ["fx-stack"];
  for (const effect of enabled) {
    if (effect.id === "gradient") classes.push("fx-grad2");
    else if (effect.id === "mexican") classes.push("fx-mexican");
    else if (effect.id !== "shadow" && effect.id !== "typography" && effect.id !== "master") classes.push(`fx-${effect.id}`);
  }
  return classes.join(" ");
}

/** Build only validated CSS declarations. Multiple shadows and animations share one property. */
export function nameEffectStyle(effects: NameEffect[]): string {
  const styles: string[] = [];
  const shadows: string[] = [];
  const animations: string[] = [];
  const master = effects.find((effect) => effect.id === "master" && effect.enabled)?.options;
  const intensity = (master?.intensity ?? 100) / 100;
  const speed = (master?.speed ?? 100) / 100;
  const amp = (value: number, min = 0, max = 999) => clamp(value * intensity, min, max);
  const seconds = (value: number) => (value / speed).toFixed(2);
  for (const effect of effects) {
    if (!effect.enabled) continue;
    const o = effect.options;
    switch (effect.id) {
      case "gradient": {
        const stops = o.stops!;
        const animated = o.speed! > 0;
        styles.push(`background-image:linear-gradient(${o.angle}deg, ${[...stops, ...(animated ? [stops[0]] : [])].join(", ")})`);
        if (animated) {
          styles.push("background-size:200% 200%");
          animations.push(`fx-grad2-scroll ${seconds(duration(o.speed!, 12, 1.2))}s linear infinite${o.direction! < 0 ? " reverse" : ""}`);
        }
        break;
      }
      case "rainbow":
        styles.push("background-image:linear-gradient(90deg,#ff5f6d,#ffc371,#3ad1ff,#b984ff,#ff5f6d)", "background-size:200% auto");
        animations.push(`fx-rainbow ${seconds(duration(o.speed!, 6, 1))}s linear infinite${o.direction! < 0 ? " reverse" : ""}`);
        break;
      case "neon": {
        const glow = amp(o.glow!, 1, 24);
        const alpha = Math.round(amp(o.intensity!, 10, 100) * 0.8);
        shadows.push(`0 0 ${Math.max(1, glow / 2)}px currentcolor`, `0 0 ${glow}px color-mix(in oklab,currentcolor ${alpha}%,transparent)`);
        break;
      }
      case "wave":
        styles.push(`--fx-wave-y:${-amp(o.height!, 0.5, 12)}px`);
        animations.push(`fx-stack-wave ${seconds(duration(o.speed!, 2.4, 0.55))}s ease-in-out infinite`);
        break;
      case "mexican":
        styles.push(
          `--fx-letter-y:${-amp(o.height!, 0.5, 14)}px`,
          `--fx-letter-dur:${seconds(duration(o.speed!, 2.4, 0.65))}s`,
          `--fx-letter-step:${(0.025 + (o.spread! / 10) * 0.095).toFixed(3)}s`,
          `--fx-letter-dir:${o.direction! < 0 ? -1 : 1}`,
        );
        break;
      case "pulse":
        styles.push(`--fx-pulse-min:${(1 - amp(o.depth!, 5, 92) / 100).toFixed(2)}`);
        animations.push(`fx-stack-pulse ${seconds(duration(o.speed!, 2.8, 0.7))}s ease-in-out infinite`);
        break;
      case "outline":
        styles.push(`-webkit-text-stroke:${amp(o.width!, 0.25, 4)}px ${o.color}`);
        break;
      case "shadow":
        shadows.push(`${amp(o.x!, -10, 10)}px ${amp(o.y!, -10, 10)}px ${amp(o.blur!, 0, 20)}px ${rgba(o.color!, amp(o.opacity!, 5, 100))}`);
        break;
      case "retro":
        shadows.push(`${amp(o.offset!, 0.5, 8)}px ${amp(o.offset!, 0.5, 8)}px 0 color-mix(in oklab,currentcolor ${amp(o.opacity!, 10, 100)}%,var(--bg-0))`);
        break;
      case "glitch":
        shadows.push(
          `${amp(o.spread!, 0.5, 7)}px 0 0 color-mix(in oklab,var(--danger) ${amp(o.opacity!, 10, 100)}%,transparent)`,
          `${-amp(o.spread!, 0.5, 7)}px 0 0 color-mix(in oklab,var(--info) ${amp(o.opacity!, 10, 100)}%,transparent)`,
        );
        break;
      case "shimmer":
        styles.push(
          `background-image:linear-gradient(105deg,currentcolor 20%,color-mix(in oklab,currentcolor ${Math.round(100 - amp(o.intensity!, 10, 95) * 0.65)}%,white) 45%,currentcolor 70%)`,
          "background-size:250% auto",
        );
        animations.push(`fx-shimmer ${seconds(duration(o.speed!, 5, 0.8))}s linear infinite${o.direction! < 0 ? " reverse" : ""}`);
        break;
      case "sparkle":
        styles.push(
          `--fx-sparkle-dur:${seconds(duration(o.speed!, 3.8, 0.7))}s`,
          `--fx-sparkle-alpha:${(amp(o.intensity!, 10, 100) / 100).toFixed(2)}`,
        );
        break;
      case "wobble":
        styles.push(`--fx-wobble:${amp(o.amount!, 0.5, 10)}deg`, `--fx-wobble-neg:${-amp(o.amount!, 0.5, 10)}deg`);
        animations.push(`fx-stack-wobble ${seconds(duration(o.speed!, 2.5, 0.55))}s ease-in-out infinite`);
        break;
      case "candy": {
        const animated = o.speed! > 0;
        styles.push(
          `background-image:repeating-linear-gradient(${o.angle}deg,${o.color} 0 7px,${o.secondary} 7px 14px)`,
          "background-size:200% 200%",
        );
        if (animated) animations.push(`fx-candy-scroll ${seconds(duration(o.speed!, 7, 1))}s linear infinite${o.direction! < 0 ? " reverse" : ""}`);
        break;
      }
      case "ghost": {
        const ghostOpacity = clamp((o.opacity! / 100) * Math.min(1, intensity), 0.15, 0.95);
        styles.push(`opacity:${ghostOpacity.toFixed(2)}`, `filter:blur(${amp(o.blur!, 0, 4).toFixed(2)}px)`);
        shadows.push(`0 0 ${amp(o.glow!, 1, 18)}px color-mix(in oklab,currentcolor ${amp(60, 15, 90)}%,transparent)`);
        break;
      }
      case "fire":
        {
        const fireY = -amp(o.height!, 1, 14);
        styles.push(
          `--fx-fire-dur:${seconds(duration(o.speed!, 2.2, 0.45))}s`,
          `--fx-fire-y:${fireY}px`,
          `--fx-fire-y2:${fireY / 2}px`,
          `--fx-fire-alpha:${(amp(o.intensity!, 10, 100) / 100).toFixed(2)}`,
        );
        shadows.push(
          `0 ${amp(2, 0.5, 4)}px ${amp(3, 1, 7)}px rgba(255,82,24,${(amp(o.intensity!, 10, 100) / 100).toFixed(2)})`,
          `0 0 ${amp(8, 2, 16)}px rgba(255,184,48,${(amp(o.intensity!, 10, 100) / 130).toFixed(2)})`,
        );
        break;
        }
      case "extrude": {
        const depth = Math.round(amp(o.depth!, 1, 9));
        for (let step = 1; step <= depth; step++) {
          const offset = step * (o.direction! < 0 ? -1 : 1);
          shadows.push(`${offset}px ${step}px 0 ${rgba(o.color!, amp(o.opacity!, 10, 100))}`);
        }
        break;
      }
      case "typography":
        styles.push(
          `font-weight:${o.weight}`,
          `font-style:${o.italic ? "italic" : "normal"}`,
          `letter-spacing:${o.tracking}px`,
          `text-transform:${o.uppercase ? "uppercase" : "none"}`,
        );
        if (o.bubble! > 0 && !effects.some((candidate) => candidate.id === "outline" && candidate.enabled)) {
          styles.push(`-webkit-text-stroke:${amp(o.bubble!, 0.2, 3.5)}px currentcolor`, "paint-order:stroke fill");
        }
        break;
      case "master":
        break;
    }
  }
  if (shadows.length) styles.push(`text-shadow:${shadows.join(",")}`);
  if (animations.length) styles.push(`animation:${animations.join(",")}`);
  return styles.length ? `;${styles.join(";")}` : "";
}
