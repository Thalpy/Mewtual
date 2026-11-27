import test from "node:test";
import assert from "node:assert/strict";
import { decodeNativeOriginalPage, decodeNativeOwnMessage, decodeNativeReceived, decodeNativeReceivedNotice, readNativeAudit, readNativeOriginals, readNativeOwnMessage, updateNativeReceived } from "./native-text-history.ts";
const item={ordinal:"0",action:"01".repeat(32),signer:"02".repeat(32),channel:"9",presented_ms:"18446744073709551615",status:"pending",body:{kind:"create",text:"original",reply_to:null}};
test("the founder audit reader binds each revision and drops replies for a departed view", async () => {
  const scope = { server: 1, channel: "9", context: "03".repeat(32) };
  const create = { action: item.action, signer: item.signer, original_digest: "04".repeat(32),
    admission: "05".repeat(32), presented_ms: item.presented_ms, parents: [], body: item.body };
  const deletion = { ...create, action: "11".repeat(32), parents: [create.action], body: { kind: "delete", create: create.action } };
  const reply = { backend: "text-v2", scope: "selected-shared-history-audit", target: create.action,
    next: null, items: [create, deletion] };
  const read = await readNativeAudit(async (command, args) => {
    assert.equal(command, "get_message_page");
    assert.deepEqual(args, { server: 1, channel: "9", expectedContext: scope.context, auditAction: create.action, auditAfter: null, limit: 8 });
    return reply;
  }, scope, create.action, null, () => true);
  assert.deepEqual(read, { items: [create, deletion], next: null });
  for (const invalid of [
    { ...reply, next: deletion.action },
    { ...reply, items: [deletion, create] },
    { ...reply, items: [create, { ...deletion, body: { ...deletion.body, create: item.signer } }] },
    { ...reply, target: item.signer },
  ]) {
    await assert.rejects(readNativeAudit(async () => invalid, scope, create.action, null, () => true));
  }
  let current = true;
  assert.equal(await readNativeAudit(async () => { current = false; return reply; }, scope, create.action, null, () => current), null);
});
test("native own-history keeps lossless cursors and rejects foreign scope or malformed bodies",()=>{
 const page={backend:"text-v2",scope:"own-saved-originals",next:"1",end_exclusive:"18446744073709551615",items:[item]};
 assert.deepEqual(decodeNativeOriginalPage(page,"0",null,"9"),page);
 for(const patch of [{backend:"legacy"},{scope:"group"},{next:"0"},{next:"01"},{next:"18446744073709551616"},{extra:true}])assert.throws(()=>decodeNativeOriginalPage({...page,...patch},"0",null,"9"));
 for(const patch of [{channel:"8"},{ordinal:"1"},{status:"received"},{body:{kind:"create",text:"",reply_to:null}},{body:{kind:"future"}},{body:{...item.body,text:"\ud800"}}])assert.throws(()=>decodeNativeOriginalPage({...page,items:[{...item,...patch}]},"0",null,"9"));
 assert.throws(()=>decodeNativeOriginalPage(page,"0","2","9"));
});
test("empty filtered pages advance the context cursor and reuse the exact first end",async()=>{
 const requests:Record<string,unknown>[]=[];
 const result=await readNativeOriginals(async(_cmd,args)=>{requests.push(args);return requests.length===1?{backend:"text-v2",scope:"own-saved-originals",next:"8",end_exclusive:"9",items:[]}:{backend:"text-v2",scope:"own-saved-originals",next:"9",end_exclusive:"9",items:[{...item,ordinal:"8"}]};},{server:1,channel:"9",context:"03".repeat(32)},()=>true);
 assert.equal(requests.length,2);assert.equal(requests[0].endExclusive,null);assert.equal(requests[1].endExclusive,"9");assert.equal(requests[1].start,"8");assert.equal(result?.items.length,1);assert.equal(result?.next,"9");
});
test("native history bounds each batch and discards a replaced view's late response",async()=>{
 let calls=0;const result=await readNativeOriginals(async(_cmd,args)=>{calls++;return{backend:"text-v2",scope:"own-saved-originals",next:(BigInt(args.start as string)+1n).toString(),end_exclusive:"1000",items:[]};},{server:1,channel:"9",context:"03".repeat(32)},()=>true);
 assert.equal(calls,16);assert.equal(result?.next,"16");assert.equal(result?.end,"1000");
 let current=true;assert.equal(await readNativeOriginals(async()=>{current=false;return{};},{server:1,channel:"9",context:"03".repeat(32)},()=>current),null);
});

test("a live received projection binds its action and remains within the native view limit", () => {
  const notice = { server: 7, channel: "9", context: "03".repeat(32), action: item.action };
  assert.deepEqual(decodeNativeReceivedNotice(notice), notice);
  assert.throws(() => decodeNativeReceivedNotice({ ...notice, text: "unsolicited plaintext" }));
  const first = { action: item.action, text: "received", certainty: "provisional" };
  const reply = { backend: "text-v2", scope: "received-live", item: first };
  assert.deepEqual(decodeNativeReceived(reply, item.action), first);
  assert.throws(() => decodeNativeReceived(reply, item.signer));
  assert.throws(() => decodeNativeReceived({ ...reply, item: { ...first, certainty: "saved-local" } }, item.action));
  const current = Array.from({ length: 64 }, (_, i) => ({
    action: i.toString(16).padStart(64, "0"),
    text: "received",
    certainty: "provisional" as const,
  }));
  const inserted = updateNativeReceived(current, item.action, decodeNativeReceived(reply, item.action));
  assert.equal(inserted.length, 64);
  assert.deepEqual(inserted.at(-1), first);
  assert.equal(updateNativeReceived(inserted, item.action, null).some(row => row.action === item.action), false);
});

test("own message reads bind the Create and keep partial and retired plaintext out of the UI", async () => {
  const target = { server: 1, channel: "9", context: "03".repeat(32) };
  const selected = { backend: "text-v2", scope: "own-message-revision", action: item.action, scanning: false,
    item: { revision: "7a".repeat(32), text: "exact edited text", status: "submitted" } };
  assert.deepEqual(decodeNativeOwnMessage(selected, item.action), { scanning: false, item: selected.item });
  assert.throws(() => decodeNativeOwnMessage(selected, item.signer));
  assert.throws(() => decodeNativeOwnMessage({ ...selected, scanning: true }, item.action));
  let calls = 0;
  const scanning = await readNativeOwnMessage(async (_command, args) => {
    calls++;
    assert.deepEqual(args, { server: 1, channel: "9", expectedContext: target.context, ownAction: item.action });
    return { ...selected, scanning: true, item: null };
  }, target, item.action, () => true);
  assert.equal(calls, 16);
  assert.deepEqual(scanning, { scanning: true, item: null });
  let current = true;
  assert.equal(await readNativeOwnMessage(async () => { current = false; return selected; }, target, item.action, () => current), null);
});
