import assert from "node:assert/strict";
import test from "node:test";
import { CallMappingSession } from "./call-mappings.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
async function until(predicate: () => boolean) {
  for (let i = 0; i < 100 && !predicate(); i++) await Promise.resolve();
  assert.ok(predicate(), "expected local IPC continuation did not run");
}
function fixture() {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const handlers = new Map<string, Array<() => Promise<unknown>>>();
  let serial = 0;
  const invoke = async (command: string, args: Record<string, unknown> = {}) => {
    calls.push({ command, args });
    const handler = handlers.get(command)?.shift();
    if (handler) return handler();
    if (command === "prepare_call_mapping_controller") return `controller-${++serial}`;
    if (command === "prepare_call_mapping_owner") return `call-${++serial}`;
    if (command === "activate_call_mapping_owner") return args.candidate;
    if (command === "map_call_port") return { grant: `grant-${++serial}`, ip: "203.0.113.4", port: args.port, mechanism: "fixture", confirmed: false };
    if (command === "end_call_mapping_owner" || command === "unmap_call_port") return null;
    throw new Error(`unexpected ${command}`);
  };
  const session = new CallMappingSession(invoke);
  return { session, calls, invoke,
    hold(command: string, promise: Promise<unknown>) { handlers.set(command, [...handlers.get(command) ?? [], () => promise]); },
    of(command: string) { return calls.filter(c => c.command === command); },
  };
}

test("current call mapping uses the native owner and retires it on leave", async () => {
  const f = fixture(); await f.session.claimController(); f.session.start(1, 3, "9");
  assert.equal((await f.session.map(1, 1234, "192.168.1.4"))?.port, 1234);
  assert.deepEqual(f.of("map_call_port")[0].args, { owner: "call-2", port: 1234, address: "192.168.1.4" });
  f.session.endCall();
  assert.equal(f.of("end_call_mapping_owner").at(-1)?.args.owner, "call-2");
  assert.equal(await f.session.map(1, 1234, null), null);
});

test("leave during prepare cancels only that candidate without activation", async () => {
  const f = fixture(); await f.session.claimController();
  const gate = deferred<string>(); f.hold("prepare_call_mapping_owner", gate.promise);
  f.session.start(1, 3, "9"); await until(() => f.of("prepare_call_mapping_owner").length === 1);
  f.session.endCall(); gate.resolve("late-candidate");
  await until(() => f.of("end_call_mapping_owner").some(c => c.args.owner === "late-candidate"));
  assert.equal(f.of("activate_call_mapping_owner").length, 1, "only controller activation ran");
});

test("leave during activation compensates with the exact returned owner", async () => {
  const f = fixture(); await f.session.claimController();
  const gate = deferred<string>(); f.hold("activate_call_mapping_owner", gate.promise);
  f.session.start(1, 3, "9"); await until(() => f.of("activate_call_mapping_owner").length === 2);
  f.session.endCall(); gate.resolve("late-owner");
  await until(() => f.of("end_call_mapping_owner").some(c => c.args.owner === "late-owner"));
  assert.equal(f.of("map_call_port").length, 0);
});

test("old same-room mapping completion cannot become a replacement join's route", async () => {
  const f = fixture(); await f.session.claimController(); f.session.start(1, 3, "9");
  const gate = deferred<unknown>(); f.hold("map_call_port", gate.promise);
  const old = f.session.map(1, 1234, null); await until(() => f.of("map_call_port").length === 1);
  f.session.start(2, 3, "9");
  const current = await f.session.map(2, 1234, null); assert.equal(current?.port, 1234);
  gate.resolve({ grant: "old-grant", ip: "203.0.113.7", port: 1234, mechanism: "fixture", confirmed: false });
  assert.equal(await old, null);
  assert.deepEqual(f.of("unmap_call_port")[0].args, { owner: "call-2", grant: "old-grant" });
  assert.equal(f.of("map_call_port")[1].args.owner, "call-3");
});

test("destroyed mount cannot activate its delayed controller claim", async () => {
  const f = fixture(); const gate = deferred<string>(); f.hold("prepare_call_mapping_controller", gate.promise);
  const old = f.session.claimController(); f.session.destroy();
  const replacement = new CallMappingSession(f.invoke); assert.ok(await replacement.claimController());
  gate.resolve("old-controller"); assert.equal(await old, null);
  assert.equal(f.of("activate_call_mapping_owner").some(c => c.args.candidate === "old-controller"), false);
  assert.ok(f.of("end_call_mapping_owner").some(c => c.args.owner === "old-controller"));
});

test("reset during a mapped request rejects its result and uses exact compensation", async () => {
  const f = fixture(); await f.session.claimController(); f.session.start(1, 3, "9");
  const gate = deferred<unknown>(); f.hold("map_call_port", gate.promise);
  const old = f.session.map(1, 1234, null); await until(() => f.of("map_call_port").length === 1);
  f.session.resetController(); gate.resolve({ grant: "old", port: 1234 });
  assert.equal(await old, null);
  assert.ok(f.of("end_call_mapping_owner").some(c => c.args.owner === "controller-1"));
  assert.deepEqual(f.of("unmap_call_port")[0].args, { owner: "call-2", grant: "old" });
});

test("failed activation has bounded IPC retries and never asks the router", async () => {
  const f = fixture(); await f.session.claimController();
  for (let i = 0; i < 3; i++) f.hold("activate_call_mapping_owner", Promise.reject(new Error("superseded")));
  f.session.start(1, 3, "9"); assert.equal(await f.session.map(1, 1234, null), null);
  assert.equal(f.of("prepare_call_mapping_owner").length, 3);
  assert.equal(f.of("map_call_port").length, 0);
  assert.equal(f.of("end_call_mapping_owner").length, 3);
});
