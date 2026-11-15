import test from "node:test";
import assert from "node:assert/strict";
import { pendingManagerApp } from "./pending-manager-fixture.test.ts";
const token="01".repeat(32);
const nativeTextIntents={[token]:{kind:"create",token,server:7,channel:"9",expectedContext:"02".repeat(32),text:"exact retained text"}};
const raw={version:4,drafts:{"7:9":"draft"},readMarks:{},nativeTextIntents,unknownNeighbor:{preserve:"exact original upgrade"}};
test("actual App hydrate/save and fixture lock/unlock retain native intents and sticky4",async()=>{
  const app=pendingManagerApp({});app.replaceSealed(raw);await app.hydrate();
  assert.equal(app.state().uiStateReady,true);assert.deepEqual(app.state().nativeTextIntents,nativeTextIntents);
  assert.deepEqual(app.state().sealed,raw,"upgrade writes the entire original root");
  await app.save();assert.equal(app.state().sealed.version,4);assert.deepEqual(app.state().sealed.nativeTextIntents,nativeTextIntents);
  // This shared fixture models lock redaction; it does not execute the full
  // Svelte lockScreen media teardown. Hydrate/save below use actual handlers.
  app.lock();assert.deepEqual(app.state().nativeTextIntents,{});await app.unlock();
  assert.deepEqual(app.state().nativeTextIntents,nativeTextIntents);
  app.conversation(7,"9");app.type("legacy send");app.reply("r".repeat(150));
  app.answer(async()=>({accepted:true,persistence:{status:"durable"},disposition:"committed"}));
  assert.equal(await app.send(),"sent");assert.equal(app.state().sealed.version,4);
  assert.deepEqual(app.state().sealed.nativeTextIntents,nativeTextIntents);app.done();
});
test("actual App refuses future native custody and never overwrites the sealed record",async()=>{
  for(const native of [{...nativeTextIntents,[token]:{...nativeTextIntents[token],kind:"future"}},null,[]]){
    const app=pendingManagerApp({});const original={...raw,nativeTextIntents:native};app.replaceSealed(original);
    await app.hydrate();assert.equal(app.state().uiStateReady,false);assert.deepEqual(app.state().sealed,original);
    assert.equal(await app.saveImmediately(),false);assert.deepEqual(app.state().sealed,original);assert.equal(app.state().writes.length,0);app.done();
  }
});
