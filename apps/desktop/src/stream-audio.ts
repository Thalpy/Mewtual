type StoppableTrack = { stop(): void };
type MixerDestination = { stream: { getTracks(): StoppableTrack[] } };
type MixerMaster = { disconnect(): void };
type MixerContext = { close(): Promise<unknown> };

/** Release every browser-owned object in an idle shared-audio graph. */
export function disposeStreamAudioGraph(
  destination: MixerDestination | null,
  master: MixerMaster | null,
  context: MixerContext | null,
): void {
  for (const track of destination?.stream.getTracks() ?? []) track.stop();
  try { master?.disconnect(); } catch { /* already disconnected */ }
  if (context) void context.close().catch(() => {});
}
