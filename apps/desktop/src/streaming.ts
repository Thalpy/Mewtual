/** User-facing, wire-visible screen-stream resolution buckets. */
export const STREAM_HEIGHTS = [720, 1080, 1440, 2160] as const;
export type StreamHeight = (typeof STREAM_HEIGHTS)[number];
export type StreamFrameRate = 15 | 24 | 30 | 60;
export type StreamQuality = "motion" | "balanced" | "detail";

export type StreamSettings = {
  resolution: StreamHeight;
  frameRate: StreamFrameRate;
  quality: StreamQuality;
  /** Maximum upload budget for a full-resolution viewer, in megabits per second. */
  mbpsPerPeer: number;
};

export const DEFAULT_STREAM_SETTINGS: StreamSettings = {
  resolution: 1080,
  frameRate: 30,
  quality: "balanced",
  mbpsPerPeer: 6,
};

/** Clamp an untrusted signaling value to one of the four values the UI and encoder understand. */
export function isStreamHeight(value: unknown): value is StreamHeight {
  return typeof value === "number" && STREAM_HEIGHTS.includes(value as StreamHeight);
}

/**
 * Round a physical-pixel receiving surface to the nearest supported transport resolution.
 *
 * A modest dead band around the midpoints stops a resizable window from renegotiating sender
 * parameters on every one-pixel wobble. This is deliberately based only on a coarse height: exact
 * monitor and window dimensions never leave the receiving device.
 */
export function nearestStreamHeight(
  physicalHeight: number,
  previous?: StreamHeight,
  hysteresisPixels = 72,
): StreamHeight {
  const height = Number.isFinite(physicalHeight) ? Math.max(0, physicalHeight) : 0;
  if (previous !== undefined) {
    const index = STREAM_HEIGHTS.indexOf(previous);
    if (index >= 0) {
      const lower = index === 0 ? -Infinity : (STREAM_HEIGHTS[index - 1] + previous) / 2;
      const upper =
        index === STREAM_HEIGHTS.length - 1
          ? Infinity
          : (previous + STREAM_HEIGHTS[index + 1]) / 2;
      if (height >= lower - hysteresisPixels && height <= upper + hysteresisPixels) return previous;
    }
  }
  return STREAM_HEIGHTS.reduce((best, candidate) =>
    Math.abs(candidate - height) < Math.abs(best - height) ? candidate : best,
  );
}

/** Height of the largest 16:9 picture that fits in the current Mewtual window. */
export function receivingHeightForViewport(
  cssWidth: number,
  cssHeight: number,
  devicePixelRatio = 1,
): number {
  const width = Math.max(0, Number.isFinite(cssWidth) ? cssWidth : 0);
  const height = Math.max(0, Number.isFinite(cssHeight) ? cssHeight : 0);
  const ratio = Math.max(0.5, Math.min(4, Number.isFinite(devicePixelRatio) ? devicePixelRatio : 1));
  return Math.min(height, (width * 9) / 16) * ratio;
}

/** A practical warning threshold, not a promise about WebRTC's eventual variable bitrate. */
export function recommendedStreamMbps(settings: Omit<StreamSettings, "mbpsPerPeer">): number {
  const base30: Record<StreamHeight, number> = { 720: 2.5, 1080: 5, 1440: 8, 2160: 15 };
  const fpsFactor = settings.frameRate <= 24 ? 0.82 : settings.frameRate >= 60 ? 1.65 : 1;
  const qualityFactor = settings.quality === "detail" ? 1.2 : settings.quality === "motion" ? 1.1 : 1;
  return Math.round(base30[settings.resolution] * fpsFactor * qualityFactor * 10) / 10;
}

export type PeerStreamPlan = {
  receiveHeight: StreamHeight;
  transportHeight: StreamHeight;
  scaleResolutionDownBy: number;
  maxBitrate: number;
  estimatedMbps: number;
};

/**
 * Convert a receiver's authenticated coarse preference into one sender encoding. WebRTC performs
 * the actual compressed encode; the scale and cap make it a distinct, smaller mesh encode rather
 * than sending 4K bits to a 720p window.
 */
export function peerStreamPlan(
  settings: StreamSettings,
  receiveHeight: StreamHeight,
  sourceHeight: number = settings.resolution,
): PeerStreamPlan {
  const transportHeight = Math.min(settings.resolution, receiveHeight) as StreamHeight;
  // Capture constraints are advisory in some WebViews. Scale from the browser's *actual* track
  // height so a rejected 720p request cannot accidentally leave a 4K source at scale 1.
  const actualSourceHeight = Number.isFinite(sourceHeight) && sourceHeight > 0
    ? sourceHeight
    : settings.resolution;
  const scaleResolutionDownBy = Math.max(1, actualSourceHeight / transportHeight);
  // Pixel work falls with the square of the linear scale. Keep a small floor so low-resolution
  // text does not become unreadable when a user chooses a high full-resolution cap.
  const scaledMbps = settings.mbpsPerPeer / scaleResolutionDownBy ** 2;
  const estimatedMbps = Math.min(settings.mbpsPerPeer, Math.max(0.5, scaledMbps));
  return {
    receiveHeight,
    transportHeight,
    scaleResolutionDownBy,
    maxBitrate: Math.round(estimatedMbps * 1_000_000),
    estimatedMbps: Math.round(estimatedMbps * 10) / 10,
  };
}

/** Estimated screen-share mesh upload. Camera/audio and protocol overhead are intentionally extra. */
export function estimatedMeshMbps(
  settings: StreamSettings,
  receiverHeights: readonly StreamHeight[],
  sourceHeight: number = settings.resolution,
): number {
  return Math.round(
    receiverHeights.reduce(
      (sum, height) => sum + peerStreamPlan(settings, height, sourceHeight).estimatedMbps,
      0,
    ) *
      10,
  ) / 10;
}

export type PeerBudgetResult = {
  state: "applied" | "failed" | "paused" | "stop-required" | "stale";
  plan: PeerStreamPlan;
  error?: string;
};

type VideoSenderLike = {
  getParameters(): RTCRtpSendParameters;
  setParameters(parameters: RTCRtpSendParameters): Promise<void>;
  replaceTrack?(track: MediaStreamTrack | null): Promise<void>;
};

type PeerBudgetRequest = {
  sender: VideoSenderLike;
  settings: StreamSettings;
  receiveHeight: StreamHeight;
  sourceHeight: number;
  pauseOnFailure: boolean;
  attachAfterApply?: MediaStreamTrack;
  generation: number;
  plan: PeerStreamPlan;
  resolve: (result: PeerBudgetResult) => void;
};

type PeerBudgetState = {
  generation: number;
  running: boolean;
  pending?: PeerBudgetRequest;
};

/**
 * Serialize `setParameters` per sender and suppress stale completions.
 *
 * Chromium rejects overlapping parameter mutations on some versions. A generation alone prevents
 * stale UI, but serialization also guarantees the newest request is eventually attempted after
 * the old Promise settles. Callers retain/display the last applied result when a later cap fails.
 */
export class PeerVideoBudgetController {
  #states = new WeakMap<object, PeerBudgetState>();

  /** Invalidate screen work before a camera/stop mutation takes ownership of this sender. */
  invalidate(sender: VideoSenderLike): void {
    const key = sender as object;
    const state = this.#states.get(key);
    if (!state) return;
    state.generation += 1;
    state.pending?.resolve({ state: "stale", plan: state.pending.plan });
    state.pending = undefined;
  }

  apply(
    sender: VideoSenderLike,
    settings: StreamSettings,
    receiveHeight: StreamHeight,
    sourceHeight: number,
    pauseOnFailure = false,
    attachAfterApply?: MediaStreamTrack,
  ): Promise<PeerBudgetResult> {
    const key = sender as object;
    const state = this.#states.get(key) ?? { generation: 0, running: false };
    state.generation += 1;
    this.#states.set(key, state);
    const generation = state.generation;
    const plan = peerStreamPlan(settings, receiveHeight, sourceHeight);
    const result = new Promise<PeerBudgetResult>((resolve) => {
      const request: PeerBudgetRequest = {
        sender,
        settings,
        receiveHeight,
        sourceHeight,
        pauseOnFailure,
        attachAfterApply,
        generation,
        plan,
        resolve,
      };
      if (state.running) {
        // Keep exactly one overwriteable pending request. ResizeObserver and settings changes can
        // otherwise enqueue one closure per frame while Chromium has a slow encoder mutation in
        // flight. Superseded callers settle immediately as stale.
        state.pending?.resolve({ state: "stale", plan: state.pending.plan });
        state.pending = request;
      } else {
        state.running = true;
        void this.#drain(key, state, request);
      }
    });
    return result;
  }

  async #drain(key: object, state: PeerBudgetState, first: PeerBudgetRequest): Promise<void> {
    let request: PeerBudgetRequest | undefined = first;
    while (request) {
      request.resolve(await this.#perform(key, state, request));
      request = state.pending;
      state.pending = undefined;
    }
    state.running = false;
  }

  async #perform(
    key: object,
    state: PeerBudgetState,
    request: PeerBudgetRequest,
  ): Promise<PeerBudgetResult> {
    const { sender, settings, generation, plan, pauseOnFailure, attachAfterApply } = request;
    if (state.generation !== generation || this.#states.get(key) !== state) {
      return { state: "stale", plan };
    }
    try {
      if (attachAfterApply) {
        if (!sender.replaceTrack) {
          return {
            state: "stop-required",
            plan,
            error: "the screen sender cannot be parked before applying its cap",
          };
        }
        // A new screen edge starts trackless, and every reconfiguration returns to that state.
        // The selected track is attached only after the browser accepts the exact peer budget.
        await sender.replaceTrack(null);
      }
      const parameters = sender.getParameters();
      parameters.encodings = parameters.encodings?.length
        ? parameters.encodings.map((encoding) => ({
            ...encoding,
            maxBitrate: plan.maxBitrate,
            maxFramerate: settings.frameRate,
            scaleResolutionDownBy: plan.scaleResolutionDownBy,
          }))
        : [{
          maxBitrate: plan.maxBitrate,
          maxFramerate: settings.frameRate,
          scaleResolutionDownBy: plan.scaleResolutionDownBy,
          }];
      parameters.degradationPreference = degradationPreference(settings.quality);
      await sender.setParameters(parameters);
      if (state.generation !== generation) return { state: "stale", plan };
      if (attachAfterApply && sender.replaceTrack) {
        try {
          await sender.replaceTrack(attachAfterApply);
        } catch (attachError) {
          return {
            state: "stop-required",
            plan,
            error: `the capped screen track could not be attached: ${String(attachError)}`,
          };
        }
        if (state.generation !== generation) {
          // A queued screen request parks as its first serialized step. A camera caller instead
          // invalidates before replacing the track, so this stale completion must not park and
          // overwrite that newer, separately capture-bounded camera.
          return { state: "stale", plan };
        }
      }
      return { state: "applied", plan };
    } catch (error) {
      if (pauseOnFailure) {
        // Safety outranks freshness: until the queued newer cap succeeds, this sender is still
        // attached with the rejected (possibly uncapped) parameters. Park it even when resize
        // churn has already superseded this request. Returning `paused` lets that newest caller
        // explicitly reattach only after its own bounded mutation succeeds.
        if (!sender.replaceTrack) {
          return { state: "stop-required", plan, error: String(error) };
        }
        if (attachAfterApply) {
          // This request parked the sender before touching encoder parameters. On supersession the
          // edge is therefore either still null (a newer screen cap) or owned by an explicitly
          // invalidating camera mutation; do not overwrite that newer track from this catch path.
          return {
            state: state.generation === generation ? "paused" : "stale",
            plan,
            error: String(error),
          };
        }
        try {
          await sender.replaceTrack(null);
          return {
            state: "paused",
            plan,
            error: String(error),
          };
        } catch (pauseError) {
          return {
            state: "stop-required",
            plan,
            error: `${String(error)}; could not pause edge: ${String(pauseError)}`,
          };
        }
      }
      return {
        state: state.generation === generation ? "failed" : "stale",
        plan,
        error: String(error),
      };
    }
  }
}

/** Map the UI quality choice onto the standard WebRTC encoder degradation policy. */
export function degradationPreference(
  quality: StreamQuality,
): "maintain-framerate" | "balanced" | "maintain-resolution" {
  if (quality === "motion") return "maintain-framerate";
  if (quality === "detail") return "maintain-resolution";
  return "balanced";
}

const CODEC_RANK: Readonly<Record<string, number>> = {
  "video/h265": 0,
  "video/hevc": 0,
  "video/av1": 1,
  "video/vp9": 2,
  "video/h264": 3,
  "video/vp8": 4,
};

/**
 * Prefer efficient primary codecs without claiming the runtime supports them. Unknown repair and
 * redundancy codecs retain their original order after the primaries; the negotiated stats remain
 * the only source of truth shown to the user.
 */
export function preferEfficientVideoCodecs<T extends { mimeType: string }>(codecs: readonly T[]): T[] {
  return codecs
    .map((codec, index) => ({ codec, index, rank: CODEC_RANK[codec.mimeType.toLowerCase()] ?? 100 }))
    .sort((left, right) => left.rank - right.rank || left.index - right.index)
    .map(({ codec }) => codec);
}

export function streamResolutionLabel(height: StreamHeight): string {
  return height === 2160 ? "4K" : `${height}p`;
}
