/**
 * Behavioural tests for App.svelte's `embedMedia`, the composer path that sends videos through the
 * pre-upload editor. App cannot be mounted under node:test, so this runs App's own function text
 * (types stripped) inside a scripted scope, the pattern `studio-exit-routes.test.ts` uses.
 *
 * Collaborator contracts modelled here, from App:
 * - `addSharedFile(file, folder, name)` uploads to the server active when it is CALLED and
 *   resolves with the new CID; it may take any amount of time, during which the person can move.
 * - `reviewVideo(file, position, scope)` resolves once with the editor's answer; a scope change
 *   while it is open settles it as `skip` (see `video-review.ts`).
 * - `videoEditorScope` is App's derived key of lock state, server and channel.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { editableContainer, editedFileName, type VideoEditorResult } from "./video-export.ts";
import { pushToTalkEvent, type PushToTalkSettings } from "./push-to-talk.ts";

const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");

/** App's 2-space function `name`, parameters and body matched by bracket depth. */
function functionText(name: string): string {
  const start = source.search(new RegExp(String.raw`\n {2}(?:async )?function ${name}\(`)) + 1;
  assert.ok(start > 0, `${name} not found in App.svelte`);
  const match = (open: string, close: string, from: number) => {
    let depth = 0;
    for (let i = from; i < source.length; i++) { if (source[i] === open) depth++; else if (source[i] === close && --depth === 0) return i; }
    assert.fail(`${name}: unbalanced ${open}${close}`);
  };
  const paramsEnd = match("(", ")", source.indexOf("(", start));
  return source.slice(start, match("{", "}", source.indexOf("{", paramsEnd)) + 1);
}

const video = (name: string) => new File([new Uint8Array(4)], name, { type: "video/mp4" });
const image = (name: string) => new File([new Uint8Array(4)], name, { type: "image/png" });

type Scripted = {
  review?: (file: File, state: State) => Promise<VideoEditorResult>;
  upload?: (file: File, name: string, state: State) => Promise<void>;
};
type State = Record<string, unknown> & {
  scope: string; epoch: number; draft: string; uploads: Array<[string, string]>; reviews: string[];
  toasts: string[]; locked: boolean; uploading: boolean;
};

/** What App's rescope effect does on every scope change: a new key and a new epoch. */
function move(s: State, scope: string) { s.scope = scope; s.epoch++; }

function harness(script: Scripted = {}) {
  const state: State = {
    scope: "open|1|1:general", epoch: 1, draft: "", uploads: [], reviews: [], toasts: [], locked: false, uploading: false,
  };
  let cid = 0;
  const scope = {
    get videoEditorScope() { return state.scope; },
    get videoScopeEpoch() { return state.epoch; },
    get activeServerId() { return 1; },
    get locked() { return state.locked; },
    get uploading() { return state.uploading; }, set uploading(v: boolean) { state.uploading = v; },
    get draft() { return state.draft; }, set draft(v: string) { state.draft = v; },
    statusDraft: "",
    myEmbedFolder: "embeds/me",
    webCodecsAvailable: () => true,
    editableContainer, editedFileName,
    chanKey: () => state.scope,
    saveDraftFor: () => {},
    refreshFiles: async () => {},
    toast: (text: string) => { state.toasts.push(text); return state.toasts.length; },
    updateToast: (_id: number, text: string) => { state.toasts.push(text); },
    reviewVideo: async (file: File) => {
      state.reviews.push(file.name);
      return script.review ? script.review(file, state) : { kind: "original" as const };
    },
    addSharedFile: async (file: File, _folder: string, name: string) => {
      state.uploads.push([file.name, name]);
      if (script.upload) await script.upload(file, name, state);
      return `cid${++cid}`;
    },
  };
  const js = stripTypeScriptTypes(functionText("embedMedia"), { mode: "strip" }).trim();
  const { embedMedia } = new Function("state", `with (state) { const embedMedia = (${js}); return { embedMedia }; }`)(scope) as {
    embedMedia: (target: "chat" | "status", items: { file: File; name: string }[]) => Promise<void>;
  };
  return { state, embedMedia };
}

const items = (...files: File[]) => files.map((file) => ({ file, name: file.name }));

test("an untouched video and an image both upload and attach in order", async () => {
  const { state, embedMedia } = harness();
  await embedMedia("chat", items(video("a.mp4"), image("b.png")));
  assert.deepEqual(state.reviews, ["a.mp4"], "only the video is reviewed");
  assert.deepEqual(state.uploads, [["a.mp4", "a.mp4"], ["b.png", "b.png"]]);
  assert.equal(state.draft, "![a.mp4](cid:cid1) ![b.png](cid:cid2)");
  assert.equal(state.uploading, false);
});

test("an edited video uploads the re-encode under its container's name", async () => {
  const edited = new File([new Uint8Array(2)], "whatever", { type: "video/mp4" });
  const { state, embedMedia } = harness({ review: async () => ({ kind: "edited", file: edited }) });
  await embedMedia("chat", [{ file: new File([new Uint8Array(4)], "trip.mov", { type: "video/quicktime" }), name: "trip.mov" }]);
  assert.deepEqual(state.uploads, [["whatever", "trip.mp4"]]);
  assert.equal(state.draft, "![trip.mp4](cid:cid1)");
});

test("a skipped video uploads nothing and the batch carries on", async () => {
  const { state, embedMedia } = harness({ review: async (file) => ({ kind: file.name === "a.mp4" ? "skip" : "original" }) });
  await embedMedia("chat", items(video("a.mp4"), video("b.mp4")));
  assert.deepEqual(state.uploads.map(([n]) => n), ["b.mp4"]);
});

test("moving channel during an upload attaches nothing to the new channel and stops the batch", async () => {
  const { state, embedMedia } = harness({
    upload: async (file, _name, s) => { if (file.name === "a.mp4") move(s, "open|1|1:random"); },
  });
  await embedMedia("chat", items(video("a.mp4"), video("b.mp4"), image("c.png")));
  assert.deepEqual(state.uploads.map(([n]) => n), ["a.mp4"], "nothing after the move is uploaded");
  assert.deepEqual(state.reviews, ["a.mp4"], "nothing after the move is reviewed");
  assert.equal(state.draft, "", "the shared file is not attached to the channel now on screen");
  assert.ok(state.toasts.some((t) => /Shared a\.mp4, not attached/.test(t)));
  assert.ok(state.toasts.some((t) => /2 files were not attached/.test(t)));
  assert.equal(state.uploading, false);
});

test("moving while the editor is open stops the batch without a second message for that video", async () => {
  const { state, embedMedia } = harness({
    review: async (file, s) => { if (file.name === "a.mp4") move(s, "open|2|2:general"); return { kind: "skip" }; },
  });
  await embedMedia("chat", items(video("a.mp4"), image("b.png")));
  assert.deepEqual(state.uploads, []);
  assert.deepEqual(state.toasts.filter((t) => /not attached/.test(t)), ["1 file was not attached: you moved to another channel"]);
});

test("a lock during the batch stops it quietly", async () => {
  const { state, embedMedia } = harness({
    upload: async (_file, _name, s) => { s.locked = true; move(s, "locked|1|1:general"); },
  });
  await embedMedia("chat", items(image("a.png"), image("b.png")));
  assert.deepEqual(state.uploads.map(([n]) => n), ["a.png"]);
  assert.ok(!state.toasts.some((t) => /files? (was|were) not attached/.test(t)), "the lock screen explains itself");
});

test("a round trip away and back during an upload still stops the batch", async () => {
  // Channel A, then B, then A again while a.png uploads: the key is back where it started, but
  // the batch was interrupted, and its draft may have been replaced in between.
  const { state, embedMedia } = harness({
    upload: async (file, _name, s) => { if (file.name === "a.png") { move(s, "open|1|1:random"); move(s, "open|1|1:general"); } },
  });
  await embedMedia("chat", items(image("a.png"), image("b.png")));
  assert.equal(state.scope, "open|1|1:general");
  assert.deepEqual(state.uploads.map(([n]) => n), ["a.png"]);
  assert.equal(state.draft, "");
});

/** App's real `videoEditorPassKey` with push-to-talk bound to `key` and a call in progress. */
function passKeyWith(key: string, inCall = true) {
  const settings: PushToTalkSettings = { mode: "ptt", key } as PushToTalkSettings;
  const js = stripTypeScriptTypes(functionText("videoEditorPassKey"), { mode: "strip" }).trim();
  const scope = { pushToTalk: settings, inCall, pushToTalkEvent };
  return new Function("state", `with (state) { return (${js}); }`)(scope) as (e: Partial<KeyboardEvent>) => boolean;
}
const keyEvent = (code: string, key: string, mods: Partial<KeyboardEvent> = {}) =>
  ({ code, key, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods });

test("the editor lets Lock and a plain push-to-talk press through, and nothing else", () => {
  const pass = passKeyWith("KeyZ");
  assert.equal(pass(keyEvent("KeyL", "l", { ctrlKey: true })), true, "Lock works from anywhere");
  assert.equal(pass(keyEvent("KeyL", "l", { metaKey: true })), true);
  assert.equal(pass(keyEvent("KeyZ", "z")), true, "push-to-talk");
  assert.equal(pass(keyEvent("KeyZ", "z", { ctrlKey: true })), false, "Ctrl+Z is the editor's undo, not push-to-talk");
  assert.equal(pass(keyEvent("KeyS", "s")), false);
  assert.equal(passKeyWith("KeyZ", false)(keyEvent("KeyZ", "z")), false, "no call, no push-to-talk");
  const arrows = passKeyWith("ArrowLeft");
  assert.equal(arrows(keyEvent("ArrowLeft", "ArrowLeft", { altKey: true })), false, "Alt+Left must not navigate behind the dialog");
  const ctrl = passKeyWith("ControlLeft");
  assert.equal(ctrl(keyEvent("ControlLeft", "Control", { ctrlKey: true })), true, "a modifier bound as push-to-talk still works");
});
