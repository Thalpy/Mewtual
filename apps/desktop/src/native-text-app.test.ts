import test from "node:test";
import assert from "node:assert/strict";
import { pendingManagerApp, deferred, settle } from "./pending-manager-fixture.test.ts";
const context="03".repeat(32);
const row={ordinal:"0",action:"01".repeat(32),signer:"02".repeat(32),channel:"9",presented_ms:"123",status:"pending",body:{kind:"create",text:"original",reply_to:null}};
test("actual App Native send seals critical4 before its checked route and skips Legacy context lookup",async()=>{
 const app=pendingManagerApp({});await app.hydrate();app.nativeConversation(7,"9",context);app.type("original");
 app.nativeHistory({backend:"text-v2",scope:"own-saved-originals",next:"1",end_exclusive:"1",items:[row]});
 app.answer(async args=>({backend:"text-v2",disposition:"saved-local",retry_token:args.retryToken,message_id:row.action,ordinal:"0"}));
 assert.equal(await app.send(),"sent");const state=app.state();assert.equal(state.contextCalls,0);assert.equal(state.submissions.length,1);
 const sent=state.submissions[0];assert.equal(sent.sealed.version,4);const intent=Object.values(sent.sealed.nativeTextIntents)[0] as {token:string;text:string};
 assert.equal(intent.token,sent.args.retryToken);assert.equal(intent.text,"original");assert.equal(sent.args.expectedContext,context);
 assert.equal(state.draft,"");assert.deepEqual(state.nativeTextIntents,{});assert.equal(state.nativeOriginals.length,1);
 assert.equal(state.nativeHistoryCalls[0].expectedContext,context);assert.equal(state.nativeHistoryCalls[0].anchor,undefined);app.done();
});
test("actual App native seal failure keeps composer and exact intent without any invocation",async()=>{
 const app=pendingManagerApp({});await app.hydrate();app.nativeConversation(7,"9",context);app.type("original");app.failSaves(true);
 assert.equal(await app.send(),"uncertain");const state=app.state();assert.equal(state.draft,"original");assert.equal(state.submissions.length,0);assert.equal(state.contextCalls,0);assert.equal(Object.keys(state.nativeTextIntents).length,1);app.done();
});
const token="04".repeat(32);
const retained={version:4,drafts:{"7:9":"original"},readMarks:{},nativeTextIntents:{[token]:{kind:"create",token,server:7,channel:"9",expectedContext:context,text:"original"}}};
const saved=(args:Record<string,unknown>)=>({backend:"text-v2",disposition:"saved-local",retry_token:args.retryToken,message_id:row.action,ordinal:"0"});
test("confirmed removal retires only the original conversation before native dispatch", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  app.type("keep this draft");
  const gate = deferred();
  app.holdSave(gate.promise);
  const send = app.send();
  await settle();
  assert.equal(app.state().writes.length, 1);
  const otherContext = "05".repeat(32);
  app.receiveMembership({ server: 7, context: otherContext, removed: true });
  assert.equal(app.nativeRemoved(7, context), false);

  app.receiveMembership({ server: 7, context, removed: true });
  app.receiveMembership({ server: 7, context, removed: false });
  gate.resolve();
  assert.equal(await send, "uncertain");
  assert.equal(app.state().draft, "keep this draft");
  await app.nativeRetry();
  app.syncRetry();
  assert.deepEqual(app.retryTimers(), []);
  assert.equal(await app.send(), "refused");
  assert.equal(app.state().submissions.length, 0);
  const [retainedToken] = Object.keys(app.state().sealed.nativeTextIntents);
  assert.equal(app.state().sealed.nativeTextIntents[retainedToken].expectedContext, context);

  app.nativeConversation(7, "9", otherContext);
  app.type("different conversation");
  app.answer(async args => saved(args));
  assert.equal(await app.send(), "sent");
  assert.equal(app.state().submissions[0].args.expectedContext, otherContext);
  assert.deepEqual(Object.keys(app.state().sealed.nativeTextIntents), [retainedToken]);
  app.done();
});
test("native App seals only a mutation for its displayed revision and discards retired reads", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const history = { backend: "text-v2", scope: "own-saved-originals", next: "1", end_exclusive: "1", items: [row] };
  const revision = "09".repeat(32);
  const visible = { backend: "text-v2", scope: "own-message-revision", action: row.action, scanning: false,
    item: { revision, text: "Current text", status: "pending" } };
  app.nativeHistory(history);
  app.nativeOwnHistory({ ...visible, scanning: true, item: null }, visible);
  await app.nativeRefresh();
  assert.equal(app.nativeOwn().messages[row.action].item.revision, revision);
  assert.deepEqual(app.nativeOwn().calls, [0, 1].map(() => ({ server: 7, channel: "9", expectedContext: context, ownAction: row.action })));
  await app.nativeChange(row.action, row.action, "stale edit");
  assert.equal(app.state().submissions.length, 0);
  app.answer(async args => saved(args));
  const late = deferred<unknown>();
  app.nativeHistory(history);
  app.nativeOwnHistory(late.promise);
  const mutation = app.nativeChange(row.action, revision, "Next edit");
  await settle();
  const submit = app.state().submissions[0];
  assert.equal(submit.command, "edit_message");
  assert.equal(submit.args.expectedRevision, revision);
  assert.equal(submit.sealed.nativeTextIntents[submit.args.retryToken].expectedRevision, revision);
  app.retireFoundUiForReauth();
  late.resolve(visible);
  await mutation;
  assert.deepEqual(app.nativeOwn().messages, {});
  app.done();
});
test("native composer and automatic retry carry the same sealed reply parent", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  app.type("a reply");
  app.reply(row.action);
  app.answer(async args => ({ backend: "text-v2", disposition: "uncertain", retry_token: args.retryToken }));
  assert.equal(await app.send(), "uncertain");
  const first = app.state().submissions[0];
  const retryToken = String(first.args.retryToken);
  assert.equal(first.args.replyTo, row.action);
  assert.equal(first.sealed.nativeTextIntents[retryToken].replyTo, row.action);
  app.type("a reply");
  app.answer(async args => saved(args));
  await app.nativeRetry();
  const next = app.state();
  assert.equal(next.draft, "a reply", "the identical new plain draft is independent");
  assert.deepEqual(next.submissions[1].args, first.args);
  assert.deepEqual(next.nativeTextIntents, {});
  app.done();
});
test("actual native scheduler retries retained custody without a gesture and retires its restored composer",async()=>{
 const app=pendingManagerApp(retained);await app.hydrate();app.nativeConversation(7,"9",context);app.answer(async args=>saved(args));
 app.syncRetry();assert.deepEqual(app.retryTimers(),[5000]);app.fireRetry();await settle();await settle();
 assert.equal(app.state().submissions.length,1);assert.equal(app.state().submissions[0].args.retryToken,token);
 assert.deepEqual(app.state().nativeTextIntents,{});assert.equal(app.state().draft,"");
 assert.equal(await app.send(),"refused");assert.equal(app.state().submissions.length,1);app.done();
});
test("native automatic retry owns admission and preserves a draft edited while its seal waits",async()=>{
 const app=pendingManagerApp(retained);await app.hydrate();app.nativeConversation(7,"9",context);const gate=deferred();app.holdSave(gate.promise);app.answer(async args=>saved(args));
 const retry=app.nativeRetry();await settle();app.type("different draft");
 assert.equal(await app.send(),"refused");assert.equal(app.state().submissions.length,0);
 gate.resolve();await retry;assert.equal(app.state().draft,"different draft");assert.equal(app.state().submissions.length,1);app.done();
});
test("an old native retry cannot block or finish a new generation's retry",async()=>{
 const app=pendingManagerApp(retained);await app.hydrate();app.nativeConversation(7,"9",context);
 const oldReply=deferred<unknown>(),newReply=deferred<unknown>();let calls=0;
 app.answer(async()=>++calls===1?oldReply.promise:newReply.promise);
 const old=app.nativeRetry();await settle();app.lock();await app.unlock();app.nativeConversation(7,"9",context);
 const newer=app.nativeRetry();await settle();assert.equal(app.state().submissions.length,2);
 assert.equal(app.state().submissions[1].args.retryToken,token);const generation=app.state().uiStateLoadGeneration;
 oldReply.resolve(saved(app.state().submissions[0].args));await old;
 assert.equal(app.state().nativeRetryGeneration,generation);assert.equal(await app.send(),"refused");
 newReply.resolve(saved(app.state().submissions[1].args));await newer;
 assert.equal(app.state().nativeRetryGeneration,null);assert.deepEqual(app.state().nativeTextIntents,{});app.done();
});
test("actual native history distinguishes a failed read and empty filtered progress from absence",async()=>{
 const app=pendingManagerApp({});await app.hydrate();app.nativeConversation(7,"9",context);app.nativeHistory(new Error("read failed"));await app.nativeRefresh();
 assert.equal(app.state().nativeHistoryLoaded,false);assert.match(app.state().nativeHistoryError!,/could not be loaded/);
 app.nativeHistory(...Array.from({length:16},(_,i)=>({backend:"text-v2",scope:"own-saved-originals",next:String(i+1),end_exclusive:"1000",items:[]})));
 await app.nativeRefresh();assert.equal(app.state().nativeHistoryLoaded,true);assert.equal(app.state().nativeCursor?.next,"16");assert.equal(app.state().nativeCursor?.end,"1000");assert.deepEqual(app.state().nativeOriginals,[]);app.done();
});

test("received text is fetched for the exact installed view and discarded after retirement", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const notice = { server: 7, channel: "9", context, action: row.action };
  await app.receiveNotice({ ...notice, context: "aa".repeat(32) });
  assert.equal(app.state().nativeHistoryCalls.length, 0);
  const received = { action: row.action, text: "Remote original", certainty: "provisional" };
  const reply = { backend: "text-v2", scope: "received-live", item: received };
  app.nativeHistory(reply);
  await app.receiveNotice(notice);
  assert.deepEqual(app.state().nativeHistoryCalls[0], {
    server: 7, channel: "9", expectedContext: context, receivedAction: row.action,
  });
  assert.deepEqual(app.received(), [received]);
  assert.deepEqual(app.state().nativeOriginals, []);

  const previousView = deferred<unknown>();
  app.nativeHistory(previousView.promise);
  const previous = app.receiveNotice(notice);
  await settle();
  await app.receiveNotice({ ...notice, action: null });
  previousView.resolve(reply);
  await previous;
  assert.deepEqual(app.received(), []);

  const pending = deferred<unknown>();
  app.nativeHistory(pending.promise);
  const read = app.receiveNotice(notice);
  await settle();
  app.retireFoundUiForReauth();
  pending.resolve(reply);
  await read;
  assert.deepEqual(app.received(), []);
  app.done();
});

test("real App reads saved received bodies and suppresses them after a live change", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const index = { backend: "text-v2", scope: "received-archive-index", available: true, ids: [row.action], more: false };
  const item = { action: row.action, text: "Received before restart", certainty: "provisional" };
  const saved = { backend: "text-v2", scope: "received-archive", item };
  app.nativeHistory(index, saved);
  await app.nativeArchiveRefresh();
  assert.deepEqual(app.archive().items, [item]);
  assert.deepEqual(app.state().nativeHistoryCalls, [
    { server: 7, channel: "9", expectedContext: context, archiveAfter: null, limit: 8 },
    { server: 7, channel: "9", expectedContext: context, archivedAction: row.action },
  ]);

  const pending = deferred<unknown>();
  app.nativeHistory(index, pending.promise, { backend: "text-v2", scope: "received-live", item: null });
  const read = app.nativeArchiveRefresh();
  await settle();
  await app.receiveNotice({ server: 7, channel: "9", context, action: row.action });
  pending.resolve(saved);
  await read;
  assert.deepEqual(app.archive().items, []);
  app.nativeHistory(index, new Error("A received change is waiting to be saved."));
  await app.nativeArchiveRefresh();
  assert.deepEqual(app.archive().items, []);
  assert.match(app.archive().note, /could not be loaded/);

  const retired = deferred<unknown>();
  app.nativeHistory(index, retired.promise);
  const old = app.nativeArchiveRefresh();
  await settle();
  app.retireFoundUiForReauth();
  retired.resolve(saved);
  await old;
  assert.equal(app.archive(), null);
  app.done();
});

test("a slow initial history read excludes the real automatic scheduler and manual send until it retires",async()=>{
 const app=pendingManagerApp(retained);await app.hydrate();app.nativeConversation(7,"9",context);const gate=deferred<unknown>();
 app.nativeHistory(gate.promise);const read=app.nativeRefresh();await settle();app.syncRetry();
 assert.equal(app.state().nativeHistoryLoading,true);assert.deepEqual(app.retryTimers(),[]);app.fireRetry();await settle();
 assert.equal(await app.send(),"refused");await app.nativeRetry();assert.equal(app.state().submissions.length,0);
 gate.resolve({backend:"text-v2",scope:"own-saved-originals",next:"0",end_exclusive:"0",items:[]});await read;
 app.answer(async args=>saved(args));app.syncRetry();assert.deepEqual(app.retryTimers(),[5000]);app.fireRetry();await settle();await settle();
 assert.equal(app.state().submissions.length,1);assert.equal(app.state().submissions[0].args.retryToken,token);app.done();
});

test("actual empty Native restore opens a recovered first Found and reads its local history",async()=>{
 const founder="05".repeat(32);const app=pendingManagerApp({version:4,nativeTextIntents:{[founder]:{kind:"found",token:founder,displayName:"Catte",serverName:"Group",advertise:"",relay:"",rendezvous:"",isDm:false}}});
 app.answer(async()=>({backend:"text-v2",server:7,context:founder,channel:"9",is_dm:false}));
 assert.equal(app.restoreNative({backend:"text-v2",servers:[]}),true);for(let i=0;i<8;i++)await settle();
 assert.equal(app.state().startTab,"found");assert.equal(app.state().activeServerId,7);assert.equal(app.state().submissions.length,1);
 assert.equal(app.state().submissions[0].command,"found_server");assert.equal(app.state().contextCalls,0);
 assert.equal(app.state().nativeHistoryLoaded,true);assert.equal(app.state().nativeHistoryCalls.length,1);app.done();
});

test("external history and a new selection wait for a send or retry's actual native invocation to retire",async()=>{
 for(const retry of [false,true]){
  const app=pendingManagerApp(retry?retained:{});await app.hydrate();app.nativeConversation(7,"9",context);app.type("original");
  const reply=deferred<unknown>();app.answer(async()=>reply.promise);const write=retry?app.nativeRetry():app.send();await settle();
  await app.nativeRefresh(true);assert.equal(app.state().nativeHistoryCalls.length,0);
  const replacement="06".repeat(32);app.nativeConversation(8,"10",replacement);await app.nativeRefresh();
  assert.equal(app.state().nativeHistoryCalls.length,0);
  reply.resolve(saved(app.state().submissions[0].args));await write;
  assert.equal(app.state().nativeHistoryCalls.length,1);assert.equal(app.state().nativeHistoryCalls[0].server,8);
  assert.equal(app.state().nativeHistoryCalls[0].expectedContext,replacement);app.done();
 }
});

test("a delayed old Found cannot release its replacement generation's busy owner",async()=>{
 const app=pendingManagerApp({});await app.hydrate();app.nativeConversation(7,"9",context);
 const oldReply=deferred<unknown>(),newReply=deferred<unknown>();let calls=0;
 app.answer(async()=>++calls===1?oldReply.promise:newReply.promise);
 const old=app.nativeFound();await settle();assert.equal(app.state().submissions.length,1);assert.equal(app.state().busy,true);
 app.retireFoundUiForReauth();assert.equal(app.state().busy,false,"production native lock redaction releases the old UI immediately");
 const current=app.nativeFound();await settle();assert.equal(app.state().submissions.length,2);assert.equal(app.state().busy,true);
 const response=(index:number)=>({backend:"text-v2",server:8,context:app.state().submissions[index].args.requestId,channel:"9",is_dm:false});
 oldReply.resolve(response(0));await old;assert.equal(app.state().busy,true,"old finally must preserve the live replacement owner");
 newReply.resolve(response(1));await current;assert.equal(app.state().busy,false);app.done();
});
