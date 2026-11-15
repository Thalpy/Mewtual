import test from "node:test";
import assert from "node:assert/strict";
import {pendingManagerApp,pendingManagerBackend,deferred,settle} from "./pending-manager-fixture.test.ts";
import {readFileSync} from "node:fs";
import {mount,unmount} from "svelte";
import RetirementProbe from "../scripts/test-ui-retirement.svelte";
const context="03".repeat(32);

test("retiring a component cancels its debounce and an already queued timer cannot overwrite a remount",async()=>{
 const backend=pendingManagerBackend({version:4,nativeTextIntents:{}});
 const old=pendingManagerApp({},backend);await old.hydrate();old.nativeConversation(7,"9",context);old.type("old component draft");old.debounce();
 const callback=old.pendingDebounce();assert.equal(typeof callback,"function");const generation=old.state().uiStateLoadGeneration;
 old.dispose();old.dispose();assert.equal(old.state().uiStateLoadGeneration,generation+1);assert.equal(old.state().uiSessionRetired,true);assert.equal(old.pendingDebounce(),undefined);
 const current=pendingManagerApp({},backend);await current.hydrate();current.nativeConversation(7,"9",context);current.type("new component draft");await current.save();
 const expected=backend.sealed;callback();await settle();assert.equal(backend.sealed,expected);
 assert.equal(JSON.parse(backend.sealed).drafts["7:9"],"new component draft");assert.equal(backend.nativeLocked,false);assert.equal(backend.nativeGeneration,1);
 await assert.rejects(old.save(),/view|session/);assert.equal(backend.sealed,expected);old.done();current.done();
});

test("late native send completion from a disposed component cannot clean up the new component's sealed intent",async()=>{
 const backend=pendingManagerBackend({version:4,nativeTextIntents:{}});
 const old=pendingManagerApp({},backend);await old.hydrate();old.nativeConversation(7,"9",context);old.type("old signed request");
 const reply=deferred<unknown>();old.answer(async()=>reply.promise);const sending=old.send();await settle();assert.equal(old.state().submissions.length,1);
 const token=old.state().submissions[0].args.retryToken;
 old.dispose();const current=pendingManagerApp({},backend);await current.hydrate();current.nativeConversation(7,"9",context);current.type("new unsent draft");await current.save();
 assert.equal(current.state().nativeTextIntents[token].text,"old signed request");const expected=backend.sealed;const oldWrites=old.state().writes.length;
 reply.resolve({backend:"text-v2",disposition:"saved-local",retry_token:token,message_id:"04".repeat(32),ordinal:"0"});await sending;
 assert.equal(backend.sealed,expected);assert.equal(old.state().writes.length,oldWrites);assert.equal(old.state().nativeHistoryCalls.length,0);
 assert.equal(JSON.parse(backend.sealed).drafts["7:9"],"new unsent draft");assert.equal(backend.nativeLocked,false);assert.equal(backend.nativeGeneration,1);old.done();current.done();
});

test("component retirement fences queued and newly attempted saves while allowing already-submitted native work to settle",async()=>{
 const backend=pendingManagerBackend({});const old=pendingManagerApp({},backend);await old.hydrate();old.type("old draft");
 const gate=deferred();old.holdSave(gate.promise);const first=old.save();await settle();const queued=old.save();old.dispose();
 const outcomes=Promise.all([assert.rejects(first,/session/),assert.rejects(queued,/session/)]);gate.resolve();await outcomes;
 assert.equal(old.state().writes.length,1,"already-native-admitted write may finish; queued JS write never invokes native");
 const current=pendingManagerApp({},backend);await current.hydrate();current.nativeConversation(7,"9",context);current.type("replacement draft");await current.save();const expected=backend.sealed;
 await assert.rejects(old.save(),/view|session/);assert.equal(await old.saveImmediately(),false);assert.equal(backend.sealed,expected);
 assert.equal(backend.nativeLocked,false);assert.equal(backend.nativeGeneration,1);old.done();current.done();
});

test("the actual retirement hook precedes throwing Svelte cleanup and still fences new stale saves",async()=>{
 const source=readFileSync(new URL("./App.svelte",import.meta.url),"utf8");
 const registration=source.indexOf("  onDestroy(retireUiSessionOnUnmount);");assert.ok(registration>0);
 // Tie the real Svelte probe's order to ALL current App cleanup registrations.
 for(const match of source.matchAll(/\n  (?:onDestroy|onMount)\(/g)){assert.ok(registration<=match.index+1,"retirement must be the first registered teardown");}
 const backend=pendingManagerBackend({});const old=pendingManagerApp({},backend);await old.hydrate();const target=document.createElement("div");document.body.append(target);
 const probe=mount(RetirementProbe,{target,props:{retire:old.dispose,fail:()=>{throw new Error("media teardown failed");}}});
 await Promise.resolve();
 await assert.rejects(Promise.resolve().then(()=>unmount(probe)),/media teardown failed/);
 assert.equal(old.state().uiSessionRetired,true);assert.equal(old.state().uiStateReady,false);
 await assert.rejects(old.save(),/view|session/);assert.equal(backend.nativeLocked,false);target.remove();old.done();
});
