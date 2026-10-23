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
import { carriesFiles } from "./drop-guard.ts";
import type { SendOutcome } from "./message-send.ts";

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
  /** The rest of a started send or post, after it marked itself in flight (App's own waits and
   * its clear-if-unchanged); resolves when it settles, to its outcome ("sent" when it gives none). */
  send?: (state: State) => Promise<SendOutcome | void>;
  post?: (state: State) => Promise<SendOutcome | void>;
};
type State = Record<string, unknown> & {
  scope: string; epoch: number; draft: string; statusDraft: string; uploads: Array<[string, string]>;
  folders: string[]; reviews: Array<[string, string]>; toasts: string[]; locked: boolean; uploading: boolean;
  sends: number; sentTexts: string[]; statusPosts: number; folder: string; wikiPage: string; wikiBody: string[];
  replyingTo: string; sending: boolean; statusPosting: boolean;
};

/** What App's rescope effect does on every scope change: a new key and a new epoch. */
function move(s: State, scope: string) { s.scope = scope; s.epoch++; }

/** The real App functions under test, each with its real review and posting helpers. */
const UNDER_TEST = ["embedMedia", "wikiEmbed", "uploadFile", "videoReviewPlan", "reviewForUpload", "composerState", "postFromComposer"];

function harness(script: Scripted = {}) {
  const state: State = {
    scope: "open|1|1:general", epoch: 1, draft: "", statusDraft: "", uploads: [], folders: [], reviews: [], toasts: [],
    locked: false, uploading: false, sends: 0, sentTexts: [], statusPosts: 0, folder: "photos", wikiPage: "Home", wikiBody: [],
    replyingTo: "", sending: false, statusPosting: false,
  };
  let cid = 0;
  const scope = {
    get videoEditorScope() { return state.scope; },
    get videoScopeEpoch() { return state.epoch; },
    get activeServerId() { return 1; },
    get locked() { return state.locked; },
    get uploading() { return state.uploading; }, set uploading(v: boolean) { state.uploading = v; },
    get draft() { return state.draft; }, set draft(v: string) { state.draft = v; },
    get statusDraft() { return state.statusDraft; }, set statusDraft(v: string) { state.statusDraft = v; },
    get folder() { return state.folder; },
    get activeWikiPage() { return state.wikiPage; },
    insertIntoWikiBody: (text: string) => { state.wikiBody.push(text); },
    myEmbedFolder: "embeds/me",
    webCodecsAvailable: () => true,
    editableContainer, editedFileName,
    chanKey: () => state.scope,
    saveDraftFor: () => {},
    refreshFiles: async () => {},
    get replyingTo() { return state.replyingTo; },
    get sending() { return state.sending; },
    get statusPosting() { return state.statusPosting; },
    // App's contract for both: refuse while one is out; otherwise mark it out before the first
    // await, send what is in the composer, and clear the mark when it settles.
    send: (): Promise<SendOutcome> => {
      if (state.sending) return Promise.resolve("refused");
      state.sending = true; state.sends++; state.sentTexts.push(state.draft);
      return (script.send ? script.send(state) : Promise.resolve()).then((outcome) => { state.sending = false; return outcome ?? "sent"; });
    },
    postStatus: (): Promise<SendOutcome> => {
      if (state.statusPosting) return Promise.resolve("refused");
      state.statusPosting = true; state.statusPosts++; state.sentTexts.push(state.statusDraft);
      return (script.post ? script.post(state) : Promise.resolve()).then((outcome) => { state.statusPosting = false; return outcome ?? "sent"; });
    },
    toast: (text: string) => { state.toasts.push(text); return state.toasts.length; },
    updateToast: (_id: number, text: string) => { state.toasts.push(text); },
    reviewVideo: async (file: File, _position: unknown, _scope: string, context: string) => {
      state.reviews.push([file.name, context]);
      return script.review ? script.review(file, state) : { kind: "original" as const };
    },
    addSharedFile: async (file: File, folder: string, name: string) => {
      state.uploads.push([file.name, name]);
      state.folders.push(folder);
      if (script.upload) await script.upload(file, name, state);
      return `cid${++cid}`;
    },
  };
  const parts = UNDER_TEST.map((name) => `const ${name} = (${stripTypeScriptTypes(functionText(name), { mode: "strip" }).trim()});`);
  const fns = new Function("state", `with (state) { ${parts.join("\n")} return { ${UNDER_TEST.join(", ")} }; }`)(scope) as {
    embedMedia: (target: "chat" | "status", items: { file: File; name: string }[], asIs?: boolean) => Promise<void>;
    wikiEmbed: (files: File[] | null, asIs?: boolean) => Promise<void>;
    uploadFile: (files: File[] | null, asIs?: boolean) => Promise<void>;
  };
  return { state, ...fns };
}

const items = (...files: File[]) => files.map((file) => ({ file, name: file.name }));

test("an untouched video and an image both upload and attach in order", async () => {
  const { state, embedMedia } = harness();
  await embedMedia("chat", items(video("a.mp4"), image("b.png")));
  assert.deepEqual(state.reviews, [["a.mp4", "chat"]], "only the video is reviewed, as a chat attachment");
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
  assert.deepEqual(state.reviews.map(([n]) => n), ["a.mp4"], "nothing after the move is reviewed");
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

// --- Shift: as is, without the editor ------------------------------------------------------------

test("Shift skips the editor for the whole batch in every upload path", async () => {
  const chat = harness();
  await chat.embedMedia("chat", items(video("a.mp4"), image("b.png")), true);
  assert.deepEqual(chat.state.reviews, []);
  assert.deepEqual(chat.state.uploads.map(([n]) => n), ["a.mp4", "b.png"]);

  const wiki = harness();
  await wiki.wikiEmbed([video("a.mp4")], true);
  assert.deepEqual(wiki.state.reviews, []);
  assert.deepEqual(wiki.state.wikiBody, ["![a.mp4](cid:cid1)"]);

  const files = harness();
  await files.uploadFile([video("a.mp4")], true);
  assert.deepEqual(files.state.reviews, []);
  assert.deepEqual(files.state.uploads.map(([n]) => n), ["a.mp4"]);
});

// --- Post now ------------------------------------------------------------------------------------

test("post now attaches the video and then sends the chat message", async () => {
  const { state, embedMedia } = harness({ review: async () => ({ kind: "original", post: true }) });
  state.draft = "look at this";
  await embedMedia("chat", items(video("a.mp4")));
  assert.deepEqual(state.sentTexts, ["look at this ![a.mp4](cid:cid1)"], "attached, then sent");
  assert.ok(state.toasts.some((t) => /Attached a\.mp4, sending/.test(t)));
});

test("post now sends nothing that was typed while the video uploaded", async () => {
  const { state, embedMedia } = harness({
    review: async () => ({ kind: "original", post: true }),
    upload: async (_file, _name, s) => { s.draft = "hang on, I meant to say"; },
  });
  state.draft = "look at this";
  await embedMedia("chat", items(video("a.mp4")));
  assert.equal(state.sends, 0, "a different message is not sent on the person's behalf");
  assert.equal(state.draft, "hang on, I meant to say ![a.mp4](cid:cid1)", "the video is still attached");
  assert.ok(state.toasts.some((t) => /not sent: the message changed while it uploaded/.test(t)));
});

test("post now does not send when the reply target changed while the video uploaded", async () => {
  const { state, embedMedia } = harness({
    review: async () => ({ kind: "original", post: true }),
    upload: async (_file, _name, s) => { s.replyingTo = "msg-42"; },
  });
  await embedMedia("chat", items(video("a.mp4")));
  assert.equal(state.sends, 0);
});

test("post now says so when a send already under way refuses it", async () => {
  const { state, embedMedia } = harness({
    review: async () => ({ kind: "original", post: true }),
    upload: async (_file, _name, s) => { s.sending = true; },
  });
  await embedMedia("chat", items(video("a.mp4")));
  assert.equal(state.sends, 0);
  assert.ok(state.toasts.some((t) => /not sent yet: press Send when ready/.test(t)));
});

test("post now from the announcement composer posts the announcement", async () => {
  const edited = new File([new Uint8Array(2)], "x", { type: "video/mp4" });
  const { state, embedMedia } = harness({ review: async () => ({ kind: "edited", file: edited, post: true }) });
  await embedMedia("status", items(video("a.mp4")));
  assert.equal(state.statusDraft, "![a.mp4](cid:cid1)");
  assert.equal(state.statusPosts, 1);
  assert.equal(state.sends, 0);
});

/** App's sends keep the composer until their persistence wait is over, then clear it only if it
 * still holds what was sent (its draft-revision protection). `gate` holds that wait open. */
function slowSend(field: "draft" | "statusDraft", gate: Promise<void>) {
  return async (s: State) => {
    const sent = s[field];
    await gate;
    if (s[field] === sent) s[field] = "";
  };
}
async function flush() { for (let i = 0; i < 20; i++) await Promise.resolve(); }

test("post now in a batch sends the video's message, then attaches the next file to the emptied composer", async () => {
  // Found in review: the batch attached the next file while the send still owned the composer, so
  // the send's protection kept the draft, and the sent text and video stayed behind beside it.
  for (const target of ["chat", "status"] as const) {
    let release!: () => void;
    const gate = new Promise<void>((r) => { release = r; });
    const field = target === "chat" ? "draft" : "statusDraft";
    const { state, embedMedia } = harness({
      review: async () => ({ kind: "original", post: true }),
      ...(target === "chat" ? { send: slowSend(field, gate) } : { post: slowSend(field, gate) }),
    });
    state[field] = "look at these";
    const run = embedMedia(target, items(video("a.mp4"), image("b.png")));
    await flush();
    assert.deepEqual(state.sentTexts, ["look at these ![a.mp4](cid:cid1)"], `${target}: one message, the text and the video`);
    assert.equal(state.uploads.length, 1, `${target}: the image waits while the send owns the composer`);
    release();
    await run;
    assert.equal(state[field], "![b.png](cid:cid2)", `${target}: the next draft holds only the image`);
  }
});

test("a post-now send is reported from what it came to, whatever the composer holds afterwards", async () => {
  // Found in review: the outcome was guessed from the composer's text. A sent message whose draft
  // was kept (typed in and back out while it sent: same words, newer revision) was called "not
  // sent yet: press Send", which sent it twice; an announcement refused after more was typed
  // still said "sending".
  // `typed`: more text typed while it was out, which a composer guess would read as "sent".
  const cases: Array<[SendOutcome, boolean, string]> = [
    ["sent", false, "Attached a.mp4, sent"],
    ["uncertain", false, "Attached a.mp4, not confirmed yet: don't send it again"],
    ["failed", true, "Attached a.mp4, not sent"],
    ["refused", false, "Attached a.mp4, not sent yet: press Send when ready"],
  ];
  for (const target of ["chat", "status"] as const) {
    for (const [outcome, typed, chatToast] of cases) {
      if (target === "status" && outcome === "uncertain") continue; // an announcement is never uncertain
      // An announcement has no retry token: one that threw may have posted, so no "not sent".
      const toast = target === "status" && outcome === "failed" ? "Attached a.mp4, may not have posted: check the feed before posting again" : chatToast;
      const field = target === "chat" ? "draft" : "statusDraft";
      const settle = async (s: State): Promise<SendOutcome> => { if (typed) s[field] += " and more"; return outcome; };
      const { state, embedMedia } = harness({
        review: async () => ({ kind: "original", post: true }),
        ...(target === "chat" ? { send: settle } : { post: settle }),
      });
      // The draft is left exactly as it was sent: only the outcome can say what happened.
      state[field] = "look";
      await embedMedia(target, items(video("a.mp4")));
      assert.equal(state.toasts.at(-1), toast, `${target}, ${outcome}`);
    }
  }
});

test("a move away while a post-now send runs ends the batch there", async () => {
  let release!: () => void;
  const gate = new Promise<void>((r) => { release = r; });
  const { state, embedMedia } = harness({ review: async () => ({ kind: "original", post: true }), send: slowSend("draft", gate) });
  state.draft = "look";
  const run = embedMedia("chat", items(video("a.mp4"), image("b.png"), image("c.png")));
  await flush();
  move(state, "open|1|1:elsewhere");
  release();
  await run;
  assert.equal(state.uploads.length, 1, "nothing more is uploaded for a channel that is no longer on screen");
  assert.ok(state.toasts.some((t) => /2 files were not attached/.test(t)));
  assert.ok(state.toasts.includes("Attached a.mp4, sent"), "the send's own outcome is still reported");
});

test("a lock during a post-now send shows no outcome behind the lock screen", async () => {
  // Found in re-review: the outcome toast fired behind the lock screen, unlike the batch's others.
  const { state, embedMedia } = harness({
    review: async () => ({ kind: "original", post: true }),
    send: async (s: State): Promise<SendOutcome> => { s.locked = true; move(s, "locked|1|1:general"); return "uncertain"; },
  });
  state.draft = "look";
  await embedMedia("chat", items(video("a.mp4"), image("b.png")));
  assert.deepEqual(state.toasts.filter((t) => /not confirmed|sent$|not sent|attached:/.test(t)), [], state.toasts.join(" | "));
  assert.equal(state.uploads.length, 1, "and the batch stopped");
});

test("an ordinary attach does not send", async () => {
  const { state, embedMedia } = harness();
  await embedMedia("chat", items(video("a.mp4")));
  assert.equal(state.sends, 0);
});

test("post now does not send once the person has moved on", async () => {
  const { state, embedMedia } = harness({
    review: async () => ({ kind: "original", post: true }),
    upload: async (_file, _name, s) => { move(s, "open|1|1:random"); },
  });
  await embedMedia("chat", items(video("a.mp4")));
  assert.equal(state.sends, 0, "nothing is sent into the channel now on screen");
});

// --- Wiki and Files ------------------------------------------------------------------------------

test("a wiki video goes through the editor as a wiki embed and lands under its page", async () => {
  const edited = new File([new Uint8Array(2)], "x", { type: "video/mp4" });
  const { state, wikiEmbed } = harness({ review: async () => ({ kind: "edited", file: edited }) });
  await wikiEmbed([new File([new Uint8Array(4)], "tour.mov", { type: "video/quicktime" }), image("map.png")]);
  assert.deepEqual(state.reviews, [["tour.mov", "wiki"]]);
  assert.deepEqual(state.folders, ["wiki/Home", "wiki/Home"]);
  assert.deepEqual(state.wikiBody, ["![tour.mp4](cid:cid1)", "![map.png](cid:cid2)"]);
});

test("a wiki upload finishing after the page changed is shared but not inserted elsewhere", async () => {
  const { state, wikiEmbed } = harness({ upload: async (_file, _name, s) => { s.wikiPage = "Other"; } });
  await wikiEmbed([image("a.png"), image("b.png")]);
  assert.deepEqual(state.folders, ["wiki/Home"], "the next file is not uploaded");
  assert.deepEqual(state.wikiBody, []);
  assert.ok(state.toasts.some((t) => /Shared a\.png, not embedded: the page changed/.test(t)));
  assert.ok(state.toasts.some((t) => /1 file was not embedded/.test(t)));
});

test("a wiki page change while the editor is open counts the reviewed file as not embedded", async () => {
  // A rename or delete of the page lands while the person edits: no scope change closed the
  // editor, so nothing else reports this file.
  const { state, wikiEmbed } = harness({
    review: async (_file, s) => { s.wikiPage = "Renamed"; return { kind: "original" }; },
  });
  await wikiEmbed([video("a.mp4"), image("b.png")]);
  assert.deepEqual(state.uploads, []);
  assert.ok(state.toasts.some((t) => /2 files were not embedded: the page changed/.test(t)), state.toasts.join(" | "));
});

test("a Files video goes through the editor and every file lands in the folder it was dropped into", async () => {
  const { state, uploadFile } = harness({ upload: async (_file, _name, s) => { s.folder = "elsewhere"; } });
  await uploadFile([video("clip.mp4"), image("b.png")]);
  assert.deepEqual(state.reviews, [["clip.mp4", "files"]]);
  assert.deepEqual(state.folders, ["photos", "photos"], "the folder open when the files were dropped");
});

test("a skipped Files video is not shared and the rest still are", async () => {
  const { state, uploadFile } = harness({ review: async () => ({ kind: "skip" }) });
  await uploadFile([video("clip.mp4"), image("b.png")]);
  assert.deepEqual(state.uploads.map(([n]) => n), ["b.png"]);
});

// --- postStatus re-entry -------------------------------------------------------------------------

test("an announcement is posted once even when post now and Enter overlap, and new typing survives", async () => {
  let release: () => void = () => {};
  const posted: string[] = [];
  const scope = {
    statusDraft: "news ![a.mp4](cid:1)", activeServerId: 1, statusPosting: false, error: "",
    invokeDebugged: async (_cmd: string, args: { text: string }) => {
      posted.push(args.text);
      await new Promise<void>((r) => { release = r; });
    },
    errorText: String,
  };
  const js = stripTypeScriptTypes(functionText("postStatus"), { mode: "strip" }).trim();
  const { postStatus } = new Function("state", `with (state) { const postStatus = (${js}); return { postStatus }; }`)(scope) as { postStatus: () => Promise<SendOutcome> };
  const first = postStatus();
  assert.equal(scope.statusPosting, true, "marked out before the first await");
  assert.equal(await postStatus(), "refused", "Enter while the first is out posts nothing");
  assert.deepEqual(posted, ["news ![a.mp4](cid:1)"]);
  scope.statusDraft = "next announcement";
  release();
  assert.equal(await first, "sent", "the actual post reports its outcome for post now");
  assert.equal(scope.statusPosting, false);
  assert.equal(scope.statusDraft, "next announcement", "text typed meanwhile is not cleared");

  scope.invokeDebugged = async () => { throw new Error("members may not post here"); };
  assert.equal(await postStatus(), "failed");
  assert.equal(scope.error, "Error: members may not post here");
  scope.statusDraft = "   ";
  assert.equal(await postStatus(), "refused", "nothing to post");
});

// --- Wiring the tests above cannot reach ---------------------------------------------------------
// These are source pins, in the style of the other App audits: each names one connection that, if
// dropped, would leave every behavioural test green while the feature silently stops working.

test("the drop handlers leave a text or link drop to the field and take files with their Shift", () => {
  const calls: Array<[string, unknown]> = [];
  const scope = {
    carriesFiles, dragOver: true,
    embedFiles: (target: string, files: unknown, asIs: boolean) => { calls.push([`embed:${target}`, asIs]); },
    wikiEmbed: (files: unknown, asIs: boolean) => { calls.push(["wiki", asIs]); },
    uploadFile: (files: unknown, asIs: boolean) => { calls.push(["files", asIs]); return Promise.resolve(); },
    pickEventImage: () => { calls.push(["event", null]); return Promise.resolve(); },
  };
  const names = ["onComposerDrop", "onWikiDrop", "onFilesDrop", "onEventDrop"];
  const parts = names.map((name) => `const ${name} = (${stripTypeScriptTypes(functionText(name), { mode: "strip" }).trim()});`);
  const handlers = new Function("state", `with (state) { ${parts.join("\n")} return { ${names.join(", ")} }; }`)(scope) as
    Record<string, (...args: unknown[]) => void>;
  const drop = (types: string[], shiftKey = false) => {
    const e = { defaultPrevented: false, shiftKey, dataTransfer: { types, files: [] }, preventDefault() { e.defaultPrevented = true; }, stopPropagation() {} };
    return e;
  };
  for (const [name, args] of [["onComposerDrop", ["chat"]], ["onWikiDrop", []], ["onFilesDrop", []], ["onEventDrop", []]] as const) {
    const text = drop(["text/plain"]);
    handlers[name](...args, text);
    assert.equal(text.defaultPrevented, false, `${name} cancelled a text drop the field should insert`);
  }
  assert.deepEqual(calls, [], "no upload path ran for a text drop");
  const file = drop(["Files"], true);
  handlers.onComposerDrop("chat", file);
  assert.equal(file.defaultPrevented, true);
  assert.deepEqual(calls, [["embed:chat", true]]);
});

test("every drop zone passes Shift through, and takes files only", () => {
  assert.match(functionText("onComposerDrop"), /embedFiles\(target, e\.dataTransfer\?\.files \?\? null, e\.shiftKey\)/);
  assert.match(functionText("onFilesDrop"), /uploadFile\(e\.dataTransfer\?\.files \?\? null, e\.shiftKey\)/);
  assert.match(functionText("onWikiDrop"), /wikiEmbed\(e\.dataTransfer\?\.files \?\? null, e\.shiftKey\)/);
  const zones = [...source.matchAll(/ondragover=\{([^\n]*)\}\n/g)].map((m) => m[1]);
  const fileZones = zones.filter((z) => !z.includes("draggedNameEffect"));
  assert.ok(fileZones.length >= 7, `found ${fileZones.length} file drop zones`);
  for (const zone of fileZones) assert.match(zone, /acceptFileDrag/, `a drop zone accepts more than files: ${zone}`);
  assert.match(functionText("acceptFileDrag"), /if \(!carriesFiles\(e\)\) return;/);
});

test("every upload picker reads Shift from its label and hands it to its upload path", () => {
  const pickers = [...source.matchAll(/<label[^>]*use:shiftPick=\{uploadIntent\}[^>]*>[\s\S]*?<\/label>/g)].map((m) => m[0]);
  assert.equal(pickers.length, 4, "chat, announcements, wiki and Files");
  for (const target of ['embedFiles("chat"', 'embedFiles("status"', "wikiEmbed(", "uploadFile("]) {
    const picker = pickers.find((p) => p.includes(target));
    assert.ok(picker, `no Shift-aware picker for ${target}`);
    assert.match(picker!, /uploadIntent\.takePick\(\)/, `${target} ignores the Shift reading`);
  }
});

test("the drop guard is installed on the window", () => {
  assert.match(source, /\$effect\(\(\) => installFileDropGuard\(window, document\)\);/);
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
