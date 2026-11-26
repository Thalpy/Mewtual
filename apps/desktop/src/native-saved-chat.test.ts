import test from "node:test";
import assert from "node:assert/strict";
import { mount,unmount,flushSync } from "svelte";
import NativeSavedChat from "./NativeSavedChat.svelte";
import type { NativeOriginal } from "./native-text-history.ts";
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

test("saved received messages retain their status and coexist with a partial page", async () => {
  const target = document.createElement("div");
  document.body.append(target);
  const calls: boolean[] = [];
  const app = mount(NativeSavedChat, { target, props: {
    rows: [], archive: {
      items: [{ action: "73".repeat(32), text: "Saved after receiving", certainty: "provisional" }],
      note: "Some saved received messages could not be loaded.", loading: false, more: true,
    },
    readArchive: append => calls.push(append),
    loading: false, loaded: true, failure: null, more: false, text: "", busy: false,
    send: () => {}, refresh: () => {}, loadMore: () => {}, change: () => {},
  } });
  flushSync();
  assert.match(target.textContent ?? "", /Saved after receiving/);
  assert.match(target.textContent ?? "", /Saved received · Provisional/);
  assert.match(target.textContent ?? "", /Some saved received messages could not be loaded/);
  assert.doesNotMatch(target.textContent ?? "", /Nothing saved here yet/);
  [...target.querySelectorAll("button")].find(button => button.textContent === "Load more received messages")!.click();
  assert.deepEqual(calls, [true]);
  await unmount(app);
  target.remove();
});

test("the native reply control selects and clears the displayed action", async () => {
  const target = document.createElement("div");
  document.body.append(target);
  const action = "73".repeat(32);
  const selected: (string | null)[] = [];
  const app = mount(NativeSavedChat, { target, props: {
    rows: [], received: [{ action, text: "The exact parent", certainty: "provisional" }],
    loading: false, loaded: true, failure: null, more: false, text: "The reply", busy: false,
    send: () => {}, refresh: () => {}, loadMore: () => {}, change: () => {},
    replyTo: action, reply: value => selected.push(value),
  } });
  flushSync();
  assert.match(target.textContent ?? "", /Replying to The exact parent/);
  const buttons = [...target.querySelectorAll("button")];
  buttons.find(button => button.textContent === "Reply")!.click();
  buttons.find(button => button.textContent === "Cancel reply")!.click();
  assert.deepEqual(selected, [action, null]);
  await unmount(app);
  target.remove();
});

test("own message controls use the selected revision and hide original and deleted bodies", async () => {
  const target = document.createElement("div");
  document.body.append(target);
  const action = "24".repeat(32), revision = "25".repeat(32), deleted = "26".repeat(32);
  const rows: NativeOriginal[] = [action, deleted].map((action, ordinal) => ({
    action, ordinal: String(ordinal), signer: "93".repeat(32), channel: "1", presented_ms: "3", status: "pending",
    body: { kind: "create", text: `Historical body ${ordinal}`, reply_to: null },
  }));
  const changes: [string, string, string | null][] = [];
  const app = mount(NativeSavedChat, { target, props: {
    rows, own: { [action]: { scanning: false, item: { revision, text: "Selected edit", status: "pending" } },
      [deleted]: { scanning: false, item: null } },
    loading: false, loaded: true, failure: null, more: false, text: "", busy: false,
    send: () => {}, refresh: () => {}, loadMore: () => {}, change: () => {}, mutate: (...change) => changes.push(change),
  } });
  flushSync();
  assert.match(target.textContent ?? "", /Selected edit/);
  assert.doesNotMatch(target.textContent ?? "", /Historical body/);
  assert.equal(target.querySelectorAll("article").length, 1);
  const click = (label: string) => {
    [...target.querySelectorAll("button")].find(button => button.textContent === label)!.click();
    flushSync();
  };
  const submit = () => {
    target.querySelector("article form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    flushSync();
  };
  click("Edit");
  const edit = target.querySelector("article textarea")!;
  assert.equal(edit.value, "Selected edit");
  edit.value = "Next edit";
  edit.dispatchEvent(new Event("input", { bubbles: true }));
  submit();
  click("Delete");
  assert.match(target.textContent ?? "", /Delete this message/);
  submit();
  assert.deepEqual(changes, [[action, revision, "Next edit"], [action, revision, null]]);
  await unmount(app);
  target.remove();
});
