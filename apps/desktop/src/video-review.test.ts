import assert from "node:assert/strict";
import test from "node:test";
import { VideoReviewQueue, type OpenReview } from "./video-review.ts";
import type { VideoEditorResult } from "./video-export.ts";

const file = (name: string) => new File([new Uint8Array(1)], name, { type: "video/mp4" });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function queue() {
  const shown: Array<string | null> = [];
  const q = new VideoReviewQueue((open: OpenReview | null) => { shown.push(open ? open.file.name : null); });
  return { q, shown };
}

test("a second request waits for the open review instead of closing it", async () => {
  const { q, shown } = queue();
  const answers: Array<[string, VideoEditorResult]> = [];
  void q.request(file("a.mp4"), { index: 1, count: 1 }, "s", "chat").then((r) => answers.push(["a", r]));
  void q.request(file("b.mp4"), { index: 1, count: 1 }, "s", "chat").then((r) => answers.push(["b", r]));
  assert.deepEqual(shown, ["a.mp4"], "only the first editor is shown");
  assert.equal(q.waiting, 1);
  const first = q.current!;
  assert.equal(q.finish(first.id, { kind: "original" }), true);
  await settle();
  assert.deepEqual(answers, [["a", { kind: "original" }]], "b was not answered on a's behalf");
  assert.deepEqual(shown, ["a.mp4", "b.mp4"]);
  q.finish(q.current!.id, { kind: "skip" });
  await settle();
  assert.deepEqual(answers.map(([n]) => n), ["a", "b"]);
  assert.deepEqual(shown.at(-1), null, "nothing is shown once the queue is empty");
});

test("an answer from an editor that is no longer open is ignored", async () => {
  const { q } = queue();
  const answers: VideoEditorResult[] = [];
  void q.request(file("a.mp4"), { index: 1, count: 2 }, "s", "chat").then((r) => answers.push(r));
  void q.request(file("b.mp4"), { index: 2, count: 2 }, "s", "chat").then((r) => answers.push(r));
  const a = q.current!;
  q.finish(a.id, { kind: "skip" });
  const edited = new File([new Uint8Array(2)], "a.mp4", { type: "video/mp4" });
  // a's editor, torn down, answers late with its export: it must not become b's upload.
  assert.equal(q.finish(a.id, { kind: "edited", file: edited }), false);
  assert.equal(q.finish(999, { kind: "original" }), false);
  await settle();
  assert.deepEqual(answers, [{ kind: "skip" }]);
  assert.equal(q.current?.file.name, "b.mp4");
});

test("a scope change settles the open review and every waiting one bound elsewhere as skip", async () => {
  const { q, shown } = queue();
  const answers: Array<[string, VideoEditorResult]> = [];
  void q.request(file("a.mp4"), { index: 1, count: 2 }, "old", "wiki").then((r) => answers.push(["a", r]));
  void q.request(file("b.mp4"), { index: 2, count: 2 }, "old", "wiki").then((r) => answers.push(["b", r]));
  void q.request(file("c.mp4"), { index: 1, count: 1 }, "new", "chat").then((r) => answers.push(["c", r]));
  const result = q.rescope("new");
  assert.deepEqual(result, { closedOpen: true, closedContext: "wiki", dropped: 1 }, "the closed editor's context words its message");
  await settle();
  assert.deepEqual(answers, [["a", { kind: "skip" }], ["b", { kind: "skip" }]]);
  assert.equal(q.current?.file.name, "c.mp4", "a review bound to the new scope opens next");
  assert.deepEqual(shown, ["a.mp4", "c.mp4"]);
});

test("a scope change that matches the open review changes nothing", () => {
  const { q, shown } = queue();
  void q.request(file("a.mp4"), { index: 1, count: 1 }, "s", "chat");
  assert.deepEqual(q.rescope("s"), { closedOpen: false, closedContext: null, dropped: 0 });
  assert.equal(q.current?.file.name, "a.mp4");
  assert.deepEqual(shown, ["a.mp4"], "no remount");
});

test("every opening gets a fresh id, even for the same file", () => {
  const { q } = queue();
  const same = file("a.mp4");
  void q.request(same, { index: 1, count: 1 }, "s", "chat");
  const first = q.current!.id;
  q.finish(first, { kind: "skip" });
  void q.request(same, { index: 1, count: 1 }, "s", "chat");
  assert.notEqual(q.current!.id, first);
});

test("review overflow retains only sixteen requests and teardown settles every accepted one", async () => {
  const { q } = queue();
  const requests = Array.from({ length: 40 }, (_, i) => q.request(file(`${i}.mp4`), { index: 1, count: 1 }, "old", "chat"));
  try {
    assert.equal(q.waiting, 15, "the open request and fifteen waiters are the full inventory");
    assert.deepEqual(await requests[16], { kind: "skip" }, "overflow does not wait or retain its file");
  } finally {
    q.rescope("closed");
  }
  assert.equal(q.current, null); assert.equal(q.waiting, 0);
  assert.ok((await Promise.all(requests)).every(result => result.kind === "skip"));
  const next = q.request(file("after.mp4"), { index: 1, count: 1 }, "new", "chat");
  q.finish(q.current!.id, { kind: "original" });
  assert.deepEqual(await next, { kind: "original" });
});
