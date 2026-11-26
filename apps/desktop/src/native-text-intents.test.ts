import { test } from "node:test";
import assert from "node:assert/strict";
import { decodeNativeTextIntents, retainNativeTextIntent, MAX_NATIVE_TEXT_INTENTS, type NativeTextCreateIntent } from "./native-text-intents.ts";
const token = "01".repeat(32);
const create: NativeTextCreateIntent = { kind: "create", token, server: 2, channel: "340282366920938463463374607431768211455", expectedContext: "02".repeat(32), text: "exact text\n🐈" };
test("native malformed text is refused before UTF-8 allocation",()=>{
  const original=TextEncoder.prototype.encode;let calls=0;
  TextEncoder.prototype.encode=function(value){calls++;return original.call(this,value);};
  try{
    for(const text of ["x".repeat(8193),"\ud800","\udc00"]){assert.throws(()=>decodeNativeTextIntents({[token]:{...create,text}},4));}
    assert.equal(calls,0,"cheap length and surrogate checks precede encoding");
    assert.throws(()=>decodeNativeTextIntents({[token]:{...create,text:"💜".repeat(2049)}},4));
    assert.equal(calls,1,"bounded well-formed input still gets the native byte check");
  }finally{TextEncoder.prototype.encode=original;}
});
test("native requests retain exact text, 32-byte token and full u128 channel", () => {
  const queue = retainNativeTextIntent({}, create);
  assert.deepEqual(decodeNativeTextIntents(JSON.parse(JSON.stringify(queue)), 4), queue);
  assert.deepEqual(retainNativeTextIntent(queue, create), queue);
  assert.ok(Object.isFrozen(queue[token]));
  assert.throws(() => retainNativeTextIntent(queue, { ...create, text: "edited retry" }));
  assert.equal(queue[token].kind, "create");
});
test("saved replies hold one exact target while existing plain records retain their shape", () => {
  const reply = { ...create, replyTo: "03".repeat(32) };
  const queue = retainNativeTextIntent({}, reply);
  assert.deepEqual(decodeNativeTextIntents(JSON.parse(JSON.stringify(queue)), 4)[token], reply);
  assert.throws(() => retainNativeTextIntent(queue, { ...reply, replyTo: "04".repeat(32) }));
  assert.throws(() => retainNativeTextIntent(queue, create));
  for (const replyTo of [null, "", "00".repeat(32), "A1".repeat(32)]) {
    assert.throws(() => decodeNativeTextIntents({ [token]: { ...create, replyTo } }, 4));
  }
});
test("native mutation records distinguish original identity, causal revision, and body kind", () => {
  const identity = { token, server: 2, channel: "9", expectedContext: "02".repeat(32), messageId: "03".repeat(32), expectedRevision: "04".repeat(32) };
  const edit = { kind: "edit", ...identity, text: "Edited text" } as const;
  const remove = { kind: "delete", ...identity } as const;
  const queue = retainNativeTextIntent({}, edit);
  assert.deepEqual(decodeNativeTextIntents({ [token]: remove }, 4)[token], remove);
  assert.throws(() => retainNativeTextIntent(queue, { ...edit, expectedRevision: identity.messageId }));
  assert.throws(() => retainNativeTextIntent(queue, remove));
  assert.throws(() => decodeNativeTextIntents({ [token]: { ...remove, text: "" } }, 4));
  assert.throws(() => decodeNativeTextIntents({ [token]: { ...edit, expectedRevision: null } }, 4));
});
test("native future/malformed records refuse the whole queue without mutating it", () => {
  const original = { [token]: create };
  for (const patch of [{ kind: "future" }, { token: "a".repeat(32) }, { token: "00".repeat(32) }, { channel: "01" },
    { channel: "340282366920938463463374607431768211456" }, { server: Number.MAX_SAFE_INTEGER + 1 },
    { text: "" }, { text: "💜".repeat(2049) }, { text: "\ud800" }, { extra: true }]) {
    const queue = { ...original, [token]: { ...create, ...patch } }, before = JSON.stringify(queue);
    assert.throws(() => decodeNativeTextIntents(queue, 4));
    assert.equal(JSON.stringify(queue), before);
  }
  for (const value of [null, [], "", 1]) assert.throws(() => decodeNativeTextIntents(value, 4));
  assert.deepEqual(original[token], create);
});
test("native queues require sticky version4 and never truncate capacity", () => {
  for (const version of [1, 2, 3, 5]) assert.throws(() => decodeNativeTextIntents({ [token]: create }, version));
  const queue = Object.fromEntries(Array.from({ length: MAX_NATIVE_TEXT_INTENTS + 1 }, (_, i) => {
    const token = (i + 1).toString(16).padStart(64, "0"); return [token, { ...create, token }];
  }));
  assert.throws(() => decodeNativeTextIntents(queue, 4));
  assert.equal(Object.keys(queue).length, MAX_NATIVE_TEXT_INTENTS + 1);
});
test("native founder settings stay exact and obey native byte limits", () => {
  const found = { kind: "found", token, displayName: "Catte", serverName: "Group", advertise: "", relay: "relay", rendezvous: "", isDm: false };
  assert.deepEqual(decodeNativeTextIntents({ [token]: found }, 4)[token], found);
  for (const patch of [{ displayName: "" }, { serverName: "💜".repeat(65) }, { advertise: "a".repeat(4097) }, { isDm: 1 }]) {
    assert.throws(() => decodeNativeTextIntents({ [token]: { ...found, ...patch } }, 4));
  }
});
