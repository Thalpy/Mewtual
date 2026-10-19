import assert from "node:assert/strict";
import test from "node:test";
import { STUDIO_ACTOR_BUSY, STUDIO_BUSY_RETRY_MS, STUDIO_STORAGE_BUSY, busyRetrying } from "./studio-busy.ts";
import { NATIVE_NO_WRITE, nativeNoWrite } from "./studio-session.ts";
import type { StudioIpc } from "./studio-native.ts";

/** A bridge whose answers a test scripts per call, with the timers the wrapper asked for. */
function bridge(answers: Array<unknown | Error>) {
  const calls: { command: string; args: unknown }[] = [];
  const timers: { fn: () => void; ms: number }[] = [];
  let generation = 0;
  const ipc: StudioIpc = {
    invoke: async <T>(command: string, args: Record<string, unknown> = {}) => {
      calls.push({ command, args });
      const next = answers.shift();
      if (next instanceof Error) throw next;
      return next as T;
    },
    listen: async () => () => {},
  };
  const wrapped = busyRetrying(ipc, (fn, ms) => { timers.push({ fn, ms }); return () => {}; }, () => generation);
  const fire = async () => { for (const t of timers.splice(0)) t.fn(); await new Promise((r) => setTimeout(r, 0)); };
  return { wrapped, calls, timers, fire, bump: () => { generation++; } };
}

test("a storage-busy refusal is repeated with the same arguments after each scheduled pause", async () => {
  const b = bridge([new Error(STUDIO_STORAGE_BUSY), new Error(STUDIO_STORAGE_BUSY), { ok: true }]);
  const args = { server: 1, channel: "7", object: "a".repeat(32) };
  const result = b.wrapped.invoke("studio_read", args);
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(b.timers.map((t) => t.ms), [STUDIO_BUSY_RETRY_MS[0]]);
  await b.fire(); await b.fire();
  assert.deepEqual(await result, { ok: true });
  assert.equal(b.calls.length, 3);
  assert.ok(b.calls.every((c) => c.command === "studio_read" && c.args === args), "byte-identical repeats");
});

test("the schedule is bounded: after the last pause the refusal is reported as it was", async () => {
  const b = bridge(Array.from({ length: STUDIO_BUSY_RETRY_MS.length + 1 }, () => new Error(STUDIO_STORAGE_BUSY)));
  const result = b.wrapped.invoke("studio_apply", {}).catch((e: unknown) => e);
  for (let i = 0; i <= STUDIO_BUSY_RETRY_MS.length; i++) await b.fire();
  const error = await result;
  assert.ok(error instanceof Error && error.message === STUDIO_STORAGE_BUSY);
  assert.equal(b.calls.length, STUDIO_BUSY_RETRY_MS.length + 1);
  assert.equal(b.timers.length, 0, "no further pause is scheduled");
});

test("a reset or scope change ends the repeats; other errors and the actor-busy refusal are never repeated", async () => {
  const stale = bridge([new Error(STUDIO_STORAGE_BUSY), { ok: true }]);
  const pending = stale.wrapped.invoke("studio_list", {}).catch((e: unknown) => e);
  await new Promise((r) => setTimeout(r, 0));
  stale.bump(); await stale.fire();
  assert.ok((await pending) instanceof Error, "the orphaned request reports its refusal instead of repeating");
  assert.equal(stale.calls.length, 1);

  for (const text of [STUDIO_ACTOR_BUSY, "Studio request belongs to a locked or changed UI session", "transport closed"]) {
    const b = bridge([new Error(text), { ok: true }]);
    await assert.rejects(b.wrapped.invoke("studio_read", {}), { message: text });
    assert.equal(b.calls.length, 1, text);
    assert.equal(b.timers.length, 0, text);
  }
});

test("the refusal this wrapper repeats is one the session classifies as never executed", () => {
  // Repeating a request is only safe when its refusal proves nothing ran. The session's
  // `nativeNoWrite` list is the one pinned to the Rust sources (studio-native-no-write.test.ts), so
  // the text repeated here, and the actor-busy text named here, must both be on it.
  assert.ok((NATIVE_NO_WRITE as readonly string[]).includes(STUDIO_STORAGE_BUSY));
  assert.ok((NATIVE_NO_WRITE as readonly string[]).includes(STUDIO_ACTOR_BUSY));
});

test("pre-execution refusals are recognised by their exact text, as a string or an Error", () => {
  assert.equal(nativeNoWrite(STUDIO_STORAGE_BUSY), true);
  assert.equal(nativeNoWrite(new Error(STUDIO_ACTOR_BUSY)), true);
  assert.equal(nativeNoWrite(`${STUDIO_STORAGE_BUSY} (detail)`), false, "a prefix match could mistake a later fence for a refusal");
  assert.equal(nativeNoWrite(new Error("outcome=uncertain; the Save may have completed")), false);
  assert.equal(nativeNoWrite(null), false);
});
