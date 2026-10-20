/**
 * "Music we've made" for the clip maker: the Studio songs (scores) of the channel the clip is being
 * posted from, read through the same native commands and the same resolution the Studio itself
 * uses, so a song plays in a clip exactly as it plays in the Studio.
 *
 * The editor never talks to the native side itself. App builds one of these for the open channel
 * and hands it to the editor as a prop; a context without a Studio (or a test) passes none, and the
 * Music tab then offers only files from disk.
 *
 * Only resolved songs play. A song with unresolved concurrent edits, a missing recipe or an
 * overflow is refused by `scorePlaybackFromNative` with the Studio's own sentence, and the library
 * shows that sentence instead of quietly playing some other version of it. Songs whose expiry has
 * passed are not offered: a clip would keep a copy of something its author set to go away.
 */
import { indexModel, parseIndexView, studioRead, type IndexEntryView, type StudioIpc, type StudioTargetArgs } from "./studio-native.ts";
import { scorePlaybackFromNative } from "./studio-resolved-export.ts";

export type ClipSong = { readonly id: string; readonly title: string };
export type ClipMusicSource = {
  /** The channel's songs, oldest first, as the Studio lists them. A channel whose Studio holds
   * nothing yet has no songs; that is not an error. */
  list(): Promise<ClipSong[]>;
  /** One song's playback: the jam take and its authored grid end. Rejects with a sentence. */
  load(id: string): Promise<{ take: unknown; gridMs: number }>;
};

/** Songs from an index listing: visible, unexpired scores only (not overflowed or deleted), titled. */
export function songsFrom(model: { readonly entries: readonly IndexEntryView[] }, nowMs: number): ClipSong[] {
  return model.entries
    .filter((e) => e.kind === "score" && !(e.expiry.kind === "at" && e.expiry.ms <= nowMs))
    .map((e) => ({ id: e.id, title: e.title.trim() || "Untitled song" }));
}

/** The Studio of `target`, as the clip maker's music source. `me` is this device's identity, the
 * exporter the Studio stamps on every take it renders. */
export function studioMusic(ipc: StudioIpc, target: StudioTargetArgs, me: string, nowMs: () => number = () => Date.now()): ClipMusicSource {
  return {
    async list() {
      // Read directly rather than through `studioList`, which treats "no index" as a failure: a
      // channel nobody has opened the Studio in has no index, and simply no songs.
      const raw = await ipc.invoke<unknown>("studio_list", { server: target.server, channel: target.channel });
      if (raw === null || raw === undefined) return [];
      return songsFrom(indexModel(parseIndexView(raw).content), nowMs());
    },
    async load(id) {
      const view = await studioRead(ipc, target, id, "score");
      if (!view) throw new Error("This song is not on this device yet.");
      const playback = await scorePlaybackFromNative(id, view, me);
      return { take: playback.take, gridMs: playback.gridMs };
    },
  };
}
