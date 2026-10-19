// The renderer treats exactly three native error texts as "nothing was written" (nativeNoWrite in
// studio-session.ts), resends such a request automatically, and never counts it as a delivery.
// That is only true while each text is raised before a request reaches the server actor. This pins
// all three against the Rust sources, so moving or reusing one fails here instead of silently
// turning a possibly committed Save into a reauthorable "not written" one.
import assert from "node:assert/strict";
import test from "node:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { NATIVE_NO_WRITE, StudioSession, nativeNoWrite } from "./studio-session.ts";
import { CHANNEL, ME, SERVER, fakeIpc, flipnoteContent, id32, indexContent, ordinaryView, settle } from "./studio-testkit.ts";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const bridge = (file: string) => readFileSync(join(root, "apps/desktop/src-tauri/src", file), "utf8");
/** Every Rust source of the bridge and the workspace crates (build output excluded). */
function rustSources(): { path: string; text: string }[] {
  const out: { path: string; text: string }[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      if (name === "target" || name === "node_modules" || name.startsWith(".")) continue;
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path);
      else if (name.endsWith(".rs")) out.push({ path, text: readFileSync(path, "utf8") });
    }
  };
  walk(join(root, "apps/desktop/src-tauri/src")); walk(join(root, "crates"));
  return out;
}
const sources = rustSources();
const sourcesOf = (needle: string) => sources.filter(s => s.text.includes(needle)).map(s => ({ path: s.path, count: s.text.split(needle).length - 1 }));

test("each no-write text has exactly one source in the Rust tree", () => {
  for (const text of NATIVE_NO_WRITE) {
    const found = sourcesOf(text);
    assert.equal(found.reduce((n, s) => n + s.count, 0), 1, `${text}: ${found.map(s => s.path).join(", ")}`);
  }
});

test("the actor-busy refusal is raised only where the rendezvous was lost, before any lease is sent", () => {
  const studio = bridge("studio.rs"), [busy] = NATIVE_NO_WRITE;
  const select = studio.slice(studio.indexOf("let ready = tokio::select! {"), studio.indexOf("let lease ="));
  assert.ok(select.includes(`=> return Err("${busy}".into())`), "it is the rendezvous timeout arm");
  assert.ok(studio.indexOf("let lease =") > studio.indexOf(`"${busy}"`), "the lease is sent only after the select");
});

test("the full-pool refusal is raised only when claiming a slot, before the actor is contacted", () => {
  const studio = bridge("studio.rs"), lib = bridge("lib.rs"), blobs = bridge("creative_blobs.rs"), [, full] = NATIVE_NO_WRITE;
  const capacity = lib.slice(lib.indexOf("fn require_inline_download_capacity("), lib.indexOf("fn register_inline_download("));
  assert.ok(capacity.includes(full));
  const context = studio.slice(studio.indexOf("impl InvokeContext {"), studio.indexOf("async fn invoke_with_context"));
  assert.ok(context.indexOf("claim_internal_inline_download(state)?") < context.indexOf("actor_instance_of("), "Studio IPC claims its slot before the actor is looked up");
  const publish = blobs.slice(blobs.indexOf("pub(crate) async fn publish_pix("), blobs.indexOf("pub(crate) async fn request_blob_bounded("));
  assert.ok(publish.indexOf("claim_internal_inline_download(&state)?") < publish.indexOf("actor.publish_pix("), "publish_pix claims its slot before contacting the actor");
});

test("the storage-busy refusal comes from authorize, which the renderer path calls before sending the lease", () => {
  const studio = bridge("studio.rs"), [, , storage] = NATIVE_NO_WRITE;
  const authorize = studio.slice(studio.indexOf("fn authorize("), studio.indexOf("\n}\n", studio.indexOf("fn authorize(")));
  assert.ok(authorize.includes(storage), "the text is authorize's busy refusal");
  const invoke = studio.slice(studio.indexOf("async fn invoke_with_context"), studio.indexOf("let preview_delivery"));
  const authorizing = invoke.indexOf("authorize(state, server, instance, generation)?"), executing = invoke.indexOf("execute_read(lease)");
  assert.ok(authorizing > 0 && executing > authorizing, "authorize runs before the lease is handed to the actor");
});

/** A session on a fake clock: `runTimers` advances it 3 s in 50 ms steps, firing timers in due
 * order (the backoffs, 1.75 s in all, well before any 10 s Save deadline). */
function harness(autosaveMs = 0) {
  const ipc = fakeIpc(), object = id32(1), timers = new Set<{ due: number; fn: () => void }>();
  let epoch = id32(0xe1), now = 0;
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent()), group: "deadbeef" }));
  ipc.on("studio_read", () => ({ ...ordinaryView(flipnoteContent({ frames: [] }), { epochId: epoch }), group: "deadbeef" }));
  const session = new StudioSession({ ipc, me: ME, autosaveMs, now: () => now,
    schedule: (fn, ms) => { const timer = { due: now + ms, fn }; timers.add(timer); return () => { timers.delete(timer); }; } });
  const runTimers = async () => {
    for (const end = now + 3000; now < end;) {
      now += 50;
      for (const timer of [...timers].sort((a, b) => a.due - b.due)) if (timers.has(timer) && timer.due <= now) { timers.delete(timer); timer.fn(); }
      await settle(2);
    }
  };
  return { ipc, session, object, runTimers, rotate: () => { epoch = id32(0xe2); } };
}

test("a pre-dispatch refusal is resent automatically and lands without counting the visit", async () => {
  const h = harness(); let applies = 0;
  h.ipc.on("studio_apply", () => { if (++applies <= 2) throw NATIVE_NO_WRITE[0]; return ordinaryView(flipnoteContent({ fps: 7, frames: [] })); });
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); h.session.open(h.object); await settle(8);
  h.session.setFps(h.object, 7); await settle(8);
  assert.equal(h.session.saves[0].status, "queued", "cooling before its resend, never blocked or uncertain");
  await h.runTimers();
  assert.equal(applies, 3); assert.equal(h.session.saves.length, 0);
  const sent = h.ipc.calls.filter(c => c.cmd === "studio_apply").map(c => c.args);
  assert.deepEqual(sent[2], sent[0], "the same request each time");
});

test("exhausted resends settle as not written, uncounted; any other native error stays uncertain", async () => {
  for (const [error, expected] of [...NATIVE_NO_WRITE.map(text => [text, "blocked"] as const), ["Studio view request was superseded; refresh", "uncertain"] as const]) {
    const h = harness();
    h.ipc.on("studio_apply", () => { throw error; }); // Tauri rejects with the bridge's string
    h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); h.session.open(h.object); await settle(8);
    h.session.setFps(h.object, 7); await settle(8); await h.runTimers();
    const [record] = h.session.saves;
    assert.equal(record.status, expected, error); assert.equal(record.mayHaveCommitted, expected === "uncertain", error);
    assert.equal(record.attempts, expected === "blocked" ? 0 : 1, `${error}: a refused send is not a delivery`);
    assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, expected === "blocked" ? 4 : 1);
    assert.equal(nativeNoWrite(error), expected === "blocked");
    h.session.reset();
  }
});

test("a retry after an exhausted pre-dispatch refusal is admitted afresh against the current history", async () => {
  const h = harness(5000);
  h.ipc.on("studio_apply", () => { throw NATIVE_NO_WRITE[0]; });
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); h.session.open(h.object); await settle(8);
  h.session.setFps(h.object, 7);
  const saving = h.session.save().then(() => "saved", e => String(e));
  await settle(8); await h.runTimers();
  assert.match(await saving, /Could not confirm the save/, "the explicit Save reports the refusal once the resends are used up");
  const [record] = h.session.saves;
  assert.deepEqual([record.status, record.attempts], ["blocked", 0]);
  h.rotate(); await h.session.refreshDoc(h.object); // the document moved to a newer epoch meanwhile
  const sends = h.ipc.calls.filter(c => c.cmd === "studio_apply").length;
  h.session.retry(record.id); await settle(8);
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, sends, "nothing was sent into the changed history");
  assert.match(h.session.saves[0].error, /Shared history changed/);
  h.session.reset();
});

test("a read refused before dispatch is asked again rather than shown as a read error", async () => {
  const h = harness(); let reads = 0;
  h.ipc.on("studio_read", () => { if (++reads === 1) throw NATIVE_NO_WRITE[1]; return ordinaryView(flipnoteContent({ fps: 9, frames: [] })); });
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); h.session.open(h.object); await settle(8);
  assert.equal(h.session.doc?.error, "", "no read error while the read waits to be asked again");
  await h.runTimers();
  assert.equal(reads, 2); assert.equal(h.session.doc?.model?.fps, 9); assert.equal(h.session.doc?.error, "");
});
