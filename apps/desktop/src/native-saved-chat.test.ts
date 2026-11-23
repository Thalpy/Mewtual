import test from "node:test";
import assert from "node:assert/strict";
import { mount,unmount,flushSync } from "svelte";
import NativeSavedChat from "./NativeSavedChat.svelte";
const w=window as Window & typeof globalThis;
Object.assign(globalThis,{HTMLMediaElement:w.HTMLMediaElement,HTMLInputElement:w.HTMLInputElement,HTMLTextAreaElement:w.HTMLTextAreaElement,Event:w.Event});
test("native local history renders absence only after a complete successful empty read",async()=>{
 for(const state of [{loaded:false,more:false,failure:"Saved messages could not be loaded."},{loaded:true,more:true,failure:null},{loaded:true,more:false,failure:null}]){
  const target=document.createElement("div");document.body.append(target);
  const app=mount(NativeSavedChat,{target,props:{rows:[],loading:false,text:"",busy:false,send:()=>{},refresh:()=>{},loadMore:()=>{},change:()=>{},...state}});flushSync();
  assert.equal(target.textContent?.includes("Nothing saved here yet"),state.loaded&&!state.more&&!state.failure);
  if(state.more)assert.match(target.textContent??"",/More history is available/);
  if(state.failure)assert.match(target.textContent??"",/could not be loaded/);
  await unmount(app);target.remove();
 }
});

test("native chat displays an authenticated live projection with provisional status", async () => {
  const target = document.createElement("div");
  document.body.append(target);
  const app = mount(NativeSavedChat, { target, props: {
    rows: [], received: [{ action: "9a".repeat(32), text: "A received message", certainty: "provisional" }],
    loading: false, loaded: true, failure: null, more: false, text: "", busy: false,
    send: () => {}, refresh: () => {}, loadMore: () => {}, change: () => {},
  } });
  flushSync();
  assert.match(target.textContent ?? "", /A received message/);
  assert.match(target.textContent ?? "", /Received · Provisional/);
  assert.doesNotMatch(target.textContent ?? "", /Nothing saved here yet|Saved here/);
  await unmount(app);
  target.remove();
});
