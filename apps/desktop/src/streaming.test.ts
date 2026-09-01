import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  DEFAULT_STREAM_SETTINGS,
  MAX_STREAM_AUDIO_SOURCES,
  captureResolutionKnownAfterConstraint,
  PeerVideoBudgetController,
  estimatedMeshMbps,
  nearestStreamHeight,
  normalizeStreamAudioLevel,
  parseStreamSettings,
  peerStreamPlan,
  preferEfficientVideoCodecs,
  receivingHeightForViewport,
  recommendedStreamMbps,
  screenAudioSourceSlotAvailable,
  shouldClearScreenAudioOnModeChange,
  streamAudioGain,
} from "./streaming.ts";

test("stream presets have visible safe defaults and sanitize persisted values", () => {
  assert.deepEqual(parseStreamSettings(null), DEFAULT_STREAM_SETTINGS);
  assert.deepEqual(parseStreamSettings({
    resolution: 2160,
    frameRate: 60,
    quality: "motion",
    mbpsPerPeer: 500,
    audioMode: "separate",
    audioLevel: 175,
  }), {
    resolution: 2160,
    frameRate: 60,
    quality: "motion",
    mbpsPerPeer: 50,
    audioMode: "separate",
    audioLevel: 175,
  });
  assert.deepEqual(parseStreamSettings({
    resolution: 123,
    frameRate: 99,
    quality: "maximum",
    mbpsPerPeer: Number.NaN,
    audioMode: "system",
  }), DEFAULT_STREAM_SETTINGS);
  for (const mbpsPerPeer of [null, "", false, true]) {
    assert.equal(
      parseStreamSettings({ mbpsPerPeer }).mbpsPerPeer,
      DEFAULT_STREAM_SETTINGS.mbpsPerPeer,
      `non-number ${JSON.stringify(mbpsPerPeer)} must not become the minimum cap`,
    );
  }
});

test("streamer mixer levels are finite, bounded and map to linear Web Audio gain", () => {
  assert.equal(normalizeStreamAudioLevel(-1), 0);
  assert.equal(normalizeStreamAudioLevel(99.6), 100);
  assert.equal(normalizeStreamAudioLevel(201), 200);
  assert.equal(normalizeStreamAudioLevel(Number.NaN), 100);
  assert.equal(streamAudioGain(0), 0);
  assert.equal(streamAudioGain(100), 1);
  assert.equal(streamAudioGain(200), 2);
});

test("streamer application-audio fan-in has a hard source bound", () => {
  assert.equal(MAX_STREAM_AUDIO_SOURCES, 8);
  assert.equal(screenAudioSourceSlotAvailable(0), true);
  assert.equal(screenAudioSourceSlotAvailable(7), true);
  assert.equal(screenAudioSourceSlotAvailable(8), false);
  assert.equal(screenAudioSourceSlotAvailable(9), false);
  assert.equal(screenAudioSourceSlotAvailable(-1), false);
  assert.equal(screenAudioSourceSlotAvailable(1.5), false);
});

test("stream selects bind typed defaults to visible option labels", () => {
  const app = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const panel = app.match(/\{#snippet streamSettingsPanel[\s\S]*?\{\/snippet\}/)?.[0] ?? "";

  // A plain `value={...}` attribute does not select an option in HTML. These bindings are the
  // rendered contract that makes the defaults and later persisted choices visible in WebView2.
  assert.match(panel, /<select bind:value=\{streamSettings\.resolution\}/);
  assert.match(panel, /<select bind:value=\{streamSettings\.frameRate\}/);
  assert.match(panel, /<select bind:value=\{receiveResolutionMode\}/);
  assert.match(panel, /<option value=\{1080\}>1080p<\/option>/);
  assert.match(panel, /<option value=\{30\}>30 fps<\/option>/);
  assert.match(panel, /<option value="auto">Auto from this window<\/option>/);
  assert.doesNotMatch(panel, /<select value=\{/);
});

test("the live stream panel exposes a bounded master and per-source audio mixer", () => {
  const app = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const panel = app.match(/\{#snippet streamSettingsPanel[\s\S]*?\{\/snippet\}/)?.[0] ?? "";
  assert.match(panel, /aria-label="Shared audio master level"[^>]*min="0" max="200"/);
  assert.match(panel, /aria-label="Streamer audio mixer"/);
  assert.match(panel, /setScreenAudioSourceLevel\(source\.id/);
  assert.match(panel, /0% mutes[^<]*100% unity[^<]*boosts can clip/);
  assert.match(app, /node\.connect\(gain\);[\s\S]*gain\.connect\(master\)/);
});

test("changing audio capture interpretation revokes old source grants", () => {
  assert.equal(shouldClearScreenAudioOnModeChange("surface", "separate"), true);
  assert.equal(shouldClearScreenAudioOnModeChange("separate", "surface"), true);
  assert.equal(shouldClearScreenAudioOnModeChange("separate", "none"), true);
  assert.equal(shouldClearScreenAudioOnModeChange("separate", "separate"), false);
});

test("a rejected capture constraint fails closed when source dimensions are unknown", () => {
  assert.equal(captureResolutionKnownAfterConstraint(true, undefined), false);
  assert.equal(captureResolutionKnownAfterConstraint(true, 2160), true);
  assert.equal(captureResolutionKnownAfterConstraint(false, undefined), true);
});

test("receiver surfaces round to the nearest supported resolution", () => {
  assert.equal(nearestStreamHeight(719), 720);
  assert.equal(nearestStreamHeight(900), 720); // exact midpoint keeps the lower bucket
  assert.equal(nearestStreamHeight(901), 1080);
  assert.equal(nearestStreamHeight(1260), 1080);
  assert.equal(nearestStreamHeight(1261), 1440);
  assert.equal(nearestStreamHeight(1900), 2160);
  assert.equal(nearestStreamHeight(Number.NaN), 720);
});

test("auto resolution has hysteresis and uses only the fitting 16:9 window surface", () => {
  assert.equal(receivingHeightForViewport(1600, 1200, 1), 900);
  assert.equal(receivingHeightForViewport(1920, 800, 2), 1600);
  assert.equal(nearestStreamHeight(940, 720), 720, "a small resize must not flap the wire value");
  assert.equal(nearestStreamHeight(973, 720), 1080, "a decisive resize crosses the dead band");
});

test("a 4K sender gives each smaller viewer a separately scaled bounded encode", () => {
  const settings = { ...DEFAULT_STREAM_SETTINGS, resolution: 2160 as const, mbpsPerPeer: 16 };
  const p720 = peerStreamPlan(settings, 720);
  const p1080 = peerStreamPlan(settings, 1080);
  const p4k = peerStreamPlan(settings, 2160);
  assert.deepEqual(
    [p720.transportHeight, p1080.transportHeight, p4k.transportHeight],
    [720, 1080, 2160],
  );
  assert.equal(p720.scaleResolutionDownBy, 3);
  assert.equal(p1080.scaleResolutionDownBy, 2);
  assert.equal(p4k.scaleResolutionDownBy, 1);
  assert.ok(p720.maxBitrate < p1080.maxBitrate && p1080.maxBitrate < p4k.maxBitrate);
  assert.equal(estimatedMeshMbps(settings, [720, 1080, 2160]), 21.8);
});

test("a receiver cannot ask for more pixels or bits than the stream setting", () => {
  const plan = peerStreamPlan(DEFAULT_STREAM_SETTINGS, 2160);
  assert.equal(plan.transportHeight, 1080);
  assert.equal(plan.scaleResolutionDownBy, 1);
  assert.equal(plan.estimatedMbps, DEFAULT_STREAM_SETTINGS.mbpsPerPeer);
});

test("a rejected capture downscale still plans from the actual 4K source", () => {
  const settings = { ...DEFAULT_STREAM_SETTINGS, resolution: 720 as const, mbpsPerPeer: 4 };
  const plan = peerStreamPlan(settings, 720, 2160);
  assert.equal(plan.transportHeight, 720);
  assert.equal(plan.scaleResolutionDownBy, 3);
});

test("sender budgets coalesce a resize burst to one pending newest request", async () => {
  const controller = new PeerVideoBudgetController();
  const calls: RTCRtpSendParameters[] = [];
  let finishFirst!: () => void;
  const sender = {
    getParameters: () => ({ encodings: [{}] }) as RTCRtpSendParameters,
    setParameters: (parameters: RTCRtpSendParameters) => {
      calls.push(parameters);
      if (calls.length === 1) return new Promise<void>((resolve) => { finishFirst = resolve; });
      return Promise.resolve();
    },
  };

  const first = controller.apply(sender, { ...DEFAULT_STREAM_SETTINGS, resolution: 2160 }, 2160, 2160);
  await new Promise((resolve) => setTimeout(resolve, 0));
  const burst = Array.from({ length: 250 }, (_, index) => controller.apply(
    sender,
    { ...DEFAULT_STREAM_SETTINGS, resolution: index === 249 ? 720 : 2160 },
    index === 249 ? 720 : 2160,
    2160,
  ));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(calls.length, 1, "only the active encoder mutation runs during the burst");
  const superseded = await Promise.all(burst.slice(0, -1));
  assert.ok(superseded.every((result) => result.state === "stale"));
  finishFirst();
  assert.equal((await first).state, "stale");
  const latest = await burst.at(-1)!;
  assert.equal(latest.state, "applied");
  assert.equal(calls.length, 2, "the burst retains exactly one overwriteable pending mutation");
  assert.equal(latest.plan.scaleResolutionDownBy, 3);
  assert.equal(calls[1].encodings?.[0]?.maxFramerate, DEFAULT_STREAM_SETTINGS.frameRate);
});

test("a rejected sender cap parks the edge instead of leaking an uncapped stream", async () => {
  const controller = new PeerVideoBudgetController();
  const replacements: (MediaStreamTrack | null)[] = [];
  const sender = {
    getParameters: () => ({ encodings: [{}] }) as RTCRtpSendParameters,
    setParameters: async () => { throw new Error("encoder refused the cap"); },
    replaceTrack: async (track: MediaStreamTrack | null) => { replacements.push(track); },
  };
  const result = await controller.apply(sender, DEFAULT_STREAM_SETTINGS, 720, 1080, true);
  assert.equal(result.state, "paused");
  assert.deepEqual(replacements, [null]);
  assert.match(result.error ?? "", /encoder refused/);
});

test("a superseded rejected cap parks before the newest queued mutation runs", async () => {
  const controller = new PeerVideoBudgetController();
  const order: string[] = [];
  let rejectFirst!: (error: Error) => void;
  let mutations = 0;
  const sender = {
    getParameters: () => ({ encodings: [{}] }) as RTCRtpSendParameters,
    setParameters: async () => {
      mutations += 1;
      order.push(`set-${mutations}`);
      if (mutations === 1) {
        await new Promise<void>((_resolve, reject) => { rejectFirst = reject; });
      }
    },
    replaceTrack: async (_track: MediaStreamTrack | null) => { order.push("pause"); },
  };

  const first = controller.apply(sender, DEFAULT_STREAM_SETTINGS, 2160, 2160, true);
  await new Promise((resolve) => setTimeout(resolve, 0));
  const newest = controller.apply(sender, DEFAULT_STREAM_SETTINGS, 720, 2160, true);
  rejectFirst(new Error("old cap rejected"));

  assert.equal((await first).state, "paused");
  assert.equal((await newest).state, "applied");
  assert.deepEqual(order, ["set-1", "pause", "set-2"]);
});

test("a new screen track stays detached while its first encoder cap is pending", async () => {
  const controller = new PeerVideoBudgetController();
  const replacements: (MediaStreamTrack | null)[] = [];
  const track = { kind: "video" } as MediaStreamTrack;
  const sender = {
    getParameters: () => ({ encodings: [{}] }) as RTCRtpSendParameters,
    setParameters: () => new Promise<void>(() => { /* deliberately never settles */ }),
    replaceTrack: async (next: MediaStreamTrack | null) => { replacements.push(next); },
  };

  void controller.apply(sender, DEFAULT_STREAM_SETTINGS, 1080, 1080, true, track);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(replacements, [null]);
});

test("switching to camera invalidates a slow screen completion", async () => {
  const controller = new PeerVideoBudgetController();
  const replacements: (MediaStreamTrack | null)[] = [];
  const screen = { kind: "screen" } as unknown as MediaStreamTrack;
  const camera = { kind: "camera" } as unknown as MediaStreamTrack;
  let finishCap!: () => void;
  const sender = {
    getParameters: () => ({ encodings: [{}] }) as RTCRtpSendParameters,
    setParameters: () => new Promise<void>((resolve) => { finishCap = resolve; }),
    replaceTrack: async (next: MediaStreamTrack | null) => { replacements.push(next); },
  };

  const oldScreen = controller.apply(
    sender,
    DEFAULT_STREAM_SETTINGS,
    1080,
    1080,
    true,
    screen,
  );
  await new Promise((resolve) => setTimeout(resolve, 0));
  controller.invalidate(sender);
  await sender.replaceTrack(camera);
  finishCap();

  assert.equal((await oldScreen).state, "stale");
  assert.deepEqual(replacements, [null, camera]);
});

test("4K and 60 fps have an honest higher recommended bitrate", () => {
  const hd = recommendedStreamMbps({ resolution: 1080, frameRate: 30, quality: "balanced" });
  const ultra = recommendedStreamMbps({ resolution: 2160, frameRate: 60, quality: "detail" });
  assert.ok(ultra > hd * 4);
});

test("codec preferences try H.265 then modern fallbacks while retaining auxiliary codecs", () => {
  const codecs = [
    { mimeType: "video/VP8" },
    { mimeType: "video/rtx" },
    { mimeType: "video/H264" },
    { mimeType: "video/VP9" },
    { mimeType: "video/AV1" },
    { mimeType: "video/H265" },
    { mimeType: "video/red" },
  ];
  assert.deepEqual(
    preferEfficientVideoCodecs(codecs).map((codec) => codec.mimeType),
    ["video/H265", "video/AV1", "video/VP9", "video/H264", "video/VP8", "video/rtx", "video/red"],
  );
});

test("departed peers lose their stream budget diagnostics with the rest of the edge", () => {
  const app = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  assert.match(
    app,
    /function removePeer\(fp: string\)[\s\S]*const \{ \[fp\]: _codec,[\s\S]*peerVideoCodec = codecs;[\s\S]*const \{ \[fp\]: _budget,[\s\S]*peerVideoBudget = budgets;/,
  );
});
