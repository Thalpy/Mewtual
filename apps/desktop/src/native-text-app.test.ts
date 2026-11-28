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

test("received invalidations survive a Busy read and other actions wait for the same lane", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const notice = { server: 7, channel: "9", context, action: row.action };
  const other = "93".repeat(32);
  const reply = (action: string, text: string) => ({ backend: "text-v2", scope: "received-live", item: { action, text, certainty: "provisional" } });
  app.nativeHistory(reply(row.action, "Original"));
  await app.receiveNotice(notice);

  const blocked = deferred<unknown>();
  app.nativeHistory(blocked.promise, reply(other, "Second"), { backend: "text-v2", scope: "received-live", item: null });
  const changed = app.receiveNotice(notice);
  await settle();
  const queued = app.receiveNotice({ ...notice, action: other });
  await app.receiveNotice(notice);
  assert.deepEqual(app.received(), []);
  assert.deepEqual(app.state().nativeHistoryCalls.map((call: { receivedAction: string }) => call.receivedAction), [row.action, row.action]);
  blocked.reject(new Error("Chat history is busy. Try again."));
  await Promise.all([changed, queued]);
  assert.deepEqual(app.state().nativeHistoryCalls.map((call: { receivedAction: string }) => call.receivedAction), [row.action, row.action, other, row.action]);
  assert.deepEqual(app.received().map((item: { text: string }) => item.text), ["Second"]);
  app.done();
});

test("a recovered deletion supersedes an older live reply already in flight", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const notice = { server: 7, channel: "9", context, action: row.action };
  const previous = { backend: "text-v2", scope: "received-live",
    item: { action: row.action, text: "Before recovery", certainty: "provisional" } };
  const deleted = { ...previous, item: null };
  app.nativeHistory(previous);
  await app.receiveNotice(notice);
  assert.equal(app.received().length, 1);

  const delayed = deferred<unknown>();
  app.nativeHistory(delayed.promise, deleted);
  const originalRead = app.receiveNotice(notice);
  await settle();
  await app.receiveNotice(notice);
  assert.deepEqual(app.received(), []);
  delayed.resolve(previous);
  await originalRead;
  assert.deepEqual(app.received(), []);
  app.nativeHistory(deleted);
  await app.receiveNotice(notice);
  assert.deepEqual(app.received(), []);
  app.done();
});

test("joining with an approved recovery preview starts history after the join releases Busy", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.setNativeJoin("original invitation", "127.0.0.1",
    { approved_host_required: true, expires_at_ms: 1 },
    { backend: "text-v2", server: 7, context, channel: "9", name: "Joined group", display_name: "Catte", is_dm: false },
  );
  await app.nativeJoin();
  assert.deepEqual(app.nativeJoinCalls().map((call: { command: string }) => call.command), ["preview_invite"]);
  assert.equal(app.state().nativeHistoryCalls.length, 0);
  app.nativeHistory(new Error("Chat history is busy. Try again."));
  await app.nativeJoin();
  assert.deepEqual(app.nativeJoinCalls().map((call: { command: string }) => call.command), ["preview_invite", "join_server"]);
  assert.equal(app.nativeJoinCalls()[1].args.approvedHost, "127.0.0.1");
  assert.equal(app.state().activeServerId, 7);
  assert.deepEqual(app.state().nativeHistoryCalls.map((call: { expectedContext: string }) => call.expectedContext), [context, context]);
  assert.equal(app.state().nativeHistoryLoaded, true);
  app.done();
});

test("real App reads saved received bodies and suppresses them after a live change", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const index = { backend: "text-v2", scope: "received-archive-index", available: true, ids: [row.action], more: false };
  const item = { action: row.action, text: "Received before restart", certainty: "provisional" };
  const saved = { backend: "text-v2", scope: "received-archive", item };
  app.nativeArchivePages(index, saved);
  await app.nativeArchiveRefresh();
  assert.deepEqual(app.archive().items, [item]);
  assert.deepEqual(app.nativeArchiveCalls(), [
    { server: 7, channel: "9", expectedContext: context, archiveAfter: null, limit: 8 },
    { server: 7, channel: "9", expectedContext: context, archivedAction: row.action },
  ]);

  const pending = deferred<unknown>();
  app.nativeArchivePages(index, pending.promise);
  app.nativeHistory({ backend: "text-v2", scope: "received-live", item: null });
  const read = app.nativeArchiveRefresh();
  await settle();
  await app.receiveNotice({ server: 7, channel: "9", context, action: row.action });
  pending.resolve(saved);
  await read;
  assert.deepEqual(app.archive().items, []);
  app.nativeArchivePages(index, new Error("A received change is waiting to be saved."));
  await app.nativeArchiveRefresh();
  assert.deepEqual(app.archive().items, []);
  assert.match(app.archive().note, /could not be loaded/);

  const retired = deferred<unknown>();
  app.nativeArchivePages(index, retired.promise);
  const old = app.nativeArchiveRefresh();
  await settle();
  app.retireFoundUiForReauth();
  retired.resolve(saved);
  await old;
  assert.equal(app.archive(), null);
  app.done();
});

test("the requested first archive page waits for an active send to retire", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  app.type("Message in the previous conversation");
  const submitting = deferred<unknown>();
  app.answer(async () => submitting.promise);
  const send = app.send();
  await settle();
  assert.equal(app.state().submissions.length, 1);
  const replacement = "06".repeat(32);
  const item = { action: row.action, text: "Recovered in the new conversation", certainty: "provisional" };
  app.nativeConversation(8, "10", replacement);
  app.nativeArchivePages(
    { backend: "text-v2", scope: "received-archive-index", available: true, ids: [row.action], more: false },
    { backend: "text-v2", scope: "received-archive", item },
  );
  app.requestNativeArchive();
  await app.loadRequestedNativeArchive();
  assert.deepEqual(app.nativeArchiveCalls(), []);
  submitting.resolve(saved(app.state().submissions[0].args));
  await send;
  await app.loadRequestedNativeArchive();
  assert.deepEqual(app.archive().items, [item]);
  await app.loadRequestedNativeArchive();
  assert.equal(app.nativeArchiveCalls().length, 2);
  assert.ok(app.nativeArchiveCalls().every((args: Record<string, unknown>) => args.server === 8 && args.expectedContext === replacement));
  app.done();
});

test("an invalidation requests a fresh archive page and active removal retires its in-flight reply", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const notice = { server: 7, channel: "9", context, action: null };
  const index = { backend: "text-v2", scope: "received-archive-index", available: true, ids: [row.action], more: false };
  const item = { action: row.action, text: "Old saved text", certainty: "provisional" };
  const saved = { backend: "text-v2", scope: "received-archive", item };
  const delayed = deferred<unknown>();
  app.nativeArchivePages(index, delayed.promise);
  app.requestNativeArchive();
  const first = app.loadRequestedNativeArchive();
  await settle();
  await app.receiveNotice(notice);
  app.nativeArchivePages(index, { ...saved, item: null });
  await app.loadRequestedNativeArchive();
  await app.receiveNotice(notice);
  await app.loadRequestedNativeArchive();
  assert.equal(app.nativeArchiveCalls().length, 2, "only the retiring archive read may own IPC");
  delayed.resolve(saved);
  await first;
  await app.loadRequestedNativeArchive();
  assert.deepEqual(app.archive().items, []);

  const removed = deferred<unknown>();
  await app.receiveNotice(notice);
  app.nativeArchivePages(index, removed.promise);
  const last = app.loadRequestedNativeArchive();
  await settle();
  app.receiveMembership({ server: 7, context, removed: true });
  removed.resolve(saved);
  await last;
  assert.equal(app.archive(), null);
  await app.nativeArchiveRefresh();
  assert.equal(app.nativeArchiveCalls().length, 6);
  app.done();
});

test("a guarded archive deletion retires an older live reply even if a later body stays Busy", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const deleted = row.action;
  const other = "11".repeat(32);
  const old = { backend: "text-v2", scope: "received-live", item: { action: deleted, text: "Old live body", certainty: "provisional" } };
  const keep = { action: other, text: "Different live body", certainty: "provisional" };
  const notice = { server: 7, channel: "9", context };
  app.nativeHistory(old, { ...old, item: keep });
  await app.receiveNotice({ ...notice, action: deleted });
  await app.receiveNotice({ ...notice, action: other });
  const delayed = deferred<unknown>();
  app.nativeHistory(delayed.promise);
  const stale = app.receiveNotice({ ...notice, action: deleted });
  await settle();
  const busy = new Error("Chat history is busy. Try again.");
  app.nativeArchivePages(
    { backend: "text-v2", scope: "received-archive-index", available: true, ids: [deleted, other], more: false },
    { backend: "text-v2", scope: "received-archive", item: null },
    busy, busy, busy, busy,
  );
  await app.nativeArchiveRefresh();
  delayed.resolve(old);
  await stale;
  assert.deepEqual(app.received(), [keep]);
  assert.deepEqual(app.archive().items, []);
  assert.equal(app.archive().after, null);
  app.done();
});

test("an archive body that stays Busy preserves the cursor for an exact retry", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const index = { backend: "text-v2", scope: "received-archive-index", available: true, ids: [row.action], more: false };
  const busy = new Error("Chat history is busy. Try again.");
  app.nativeArchivePages(index, busy, busy, busy, busy);
  await app.nativeArchiveRefresh();
  assert.equal(app.archive().after, null);
  assert.match(app.archive().note, /could not be loaded/);
  assert.equal(app.nativeArchiveCalls().length, 5);
  const item = { action: row.action, text: "Retry uses the same cursor", certainty: "provisional" };
  app.nativeArchivePages(index, { backend: "text-v2", scope: "received-archive", item });
  await app.nativeArchiveRefresh();
  assert.equal(app.nativeArchiveCalls()[5].archiveAfter, null);
  assert.deepEqual(app.archive().items, [item]);
  app.done();
});

test("an invalidation that cannot enter the full live queue schedules a fresh guarded page", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const notice = { server: 7, channel: "9", context };
  const action = "ff".repeat(32);
  const item = { action, text: "The selected saved body", certainty: "provisional" };
  const index = { backend: "text-v2", scope: "received-archive-index", available: true, ids: [action], more: false };
  const delayed = deferred<unknown>();
  app.nativeArchivePages(index, delayed.promise);
  const archive = app.nativeArchiveRefresh();
  await settle();

  const busy = deferred<unknown>();
  app.nativeHistory(busy.promise);
  const live = Array.from({ length: 64 }, (_, index) =>
    app.receiveNotice({ ...notice, action: index.toString(16).padStart(2, "0").repeat(32) }),
  );
  await settle();
  await app.receiveNotice({ ...notice, action });
  assert.equal(app.archive(), null);
  await app.loadRequestedNativeArchive();
  assert.equal(app.nativeArchiveCalls().length, 2);

  app.nativeArchivePages(index, { backend: "text-v2", scope: "received-archive", item });
  delayed.resolve({ backend: "text-v2", scope: "received-archive", item: { ...item, text: "Older body" } });
  busy.resolve({ backend: "text-v2", scope: "received-live", item: null });
  await archive;
  await Promise.all(live);
  await app.loadRequestedNativeArchive();
  assert.deepEqual(app.archive().items, [item]);
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


test("actual App retains ordinary history while a stale reviewed cursor restarts", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const ordinary = { action: row.action, text: "Ordinary history", certainty: "proven" };
  const approved = { action: "12".repeat(32), text: "Approved after removal", certainty: "proven", replyAllowed: false };
  const cursor = "17".repeat(219);
  const index = { backend: "text-v2", scope: "reviewed-archive-index" };
  const body = { backend: "text-v2", scope: "received-archive" };
  app.nativeArchivePages(
    { backend: "text-v2", scope: "received-archive-index", available: true, ids: [ordinary.action], more: false },
    { ...body, item: ordinary },
  );
  app.nativeReviewedPages({ ...index, status: "current", ids: [], next: cursor });
  await app.nativeArchiveRefresh();
  assert.deepEqual(app.archive().items, [ordinary]);
  assert.deepEqual(app.archive().reviewed, []);
  assert.equal(app.archive().reviewedAfter, cursor);

  app.nativeReviewedPages(
    { ...index, status: "stale", ids: [], next: null },
    { ...index, status: "current", ids: [approved.action], next: null },
  );
  app.nativeArchivePages({ ...body, item: approved });
  await app.nativeArchiveRefresh(true);
  assert.deepEqual(app.nativeReviewedCalls().map((args: Record<string, unknown>) => args.reviewedAfter), [null, cursor, null]);
  assert.equal(app.nativeArchiveCalls().length, 3, "ordinary pagination did not restart");
  assert.deepEqual(app.archive().items, [ordinary]);
  assert.deepEqual(app.archive().reviewed, [approved]);
  assert.equal(app.archive().reviewedAfter, undefined);
  app.done();
});

test("actual App review uses the exact native fields and serializes review across navigation", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const original = { server: 7, channel: "9", context };
  const held = { caseId: "21".repeat(32), action: row.action, originalDigest: "22".repeat(32),
    removalHeader: "23".repeat(32), removalEpoch: "4", decision: { status: "undecided" } };
  const reply = deferred<unknown>();
  app.answer(() => reply.promise);
  const write = deferred();
  app.holdSave(write.promise);
  const first = app.nativeDecide(original, held, "reject");
  await settle();
  assert.equal(app.state().submissions.length, 0);
  write.resolve();
  await settle();
  const call = app.state().submissions[0];
  assert.equal(call.command, "decide_native_chat_held_post");
  const [intent] = Object.values(call.sealed.nativeTextIntents) as { token: string }[];
  assert.deepEqual(call.args, {
    server: 7, channel: "9", expectedContext: context, caseId: held.caseId, action: held.action,
    originalDigest: held.originalDigest, removalHeader: held.removalHeader, removalEpoch: "4",
    previousDecision: null, choice: "reject", intent: intent.token.slice(32),
  });
  const second = { server: 8, channel: "10", context: "31".repeat(32) };
  app.nativeConversation(second.server, second.channel, second.context);
  assert.equal(await app.nativeDecide(second, held, "approve"), null);
  assert.equal(app.state().submissions.length, 1);
  reply.resolve({ status: "superseded", intent: call.args.intent, caseId: held.caseId, choice: "reject" });
  assert.equal(await first, "superseded");
  assert.deepEqual(app.state().sealed.nativeTextIntents, {});
  app.answer(async args => ({ status: "saved", intent: args.intent, caseId: args.caseId, choice: args.choice,
    decisionBodyId: "24".repeat(32), streamEntryId: "25".repeat(32) }));
  assert.equal(await app.nativeDecide(second, held, "approve"), "saved");
  assert.equal(app.state().submissions.length, 2);
  assert.notEqual(app.state().submissions[1].args.intent, call.args.intent);
  app.done();
});


test("an old-generation founder reply cannot release a new generation's exact decision retry", async () => {
  const app = pendingManagerApp({});
  await app.hydrate();
  app.nativeConversation(7, "9", context);
  const scope = { server: 7, channel: "9", context };
  const held = { caseId: "21".repeat(32), action: row.action, originalDigest: "22".repeat(32),
    removalHeader: "23".repeat(32), removalEpoch: "4", decision: { status: "undecided" } };
  const old = deferred<unknown>(), replacement = deferred<unknown>();
  let calls = 0;
  app.answer(() => (++calls === 1 ? old : replacement).promise);
  const first = app.nativeDecide(scope, held, "reject");
  await settle();
  const call = app.state().submissions[0];
  const [intent] = Object.values(call.sealed.nativeTextIntents);
  const receipt = { status: "saved", intent: call.args.intent, caseId: held.caseId, choice: "reject",
    decisionBodyId: "24".repeat(32), streamEntryId: "25".repeat(32) };
  app.lock();
  await app.unlock();
  app.nativeConversation(7, "9", context);
  const second = app.nativeRetryDecision(intent);
  await settle();
  assert.deepEqual(app.state().submissions[1].args, call.args);
  old.resolve(receipt);
  assert.equal(await first, null);
  assert.equal(await app.nativeRetryDecision(intent), null);
  assert.equal(app.state().submissions.length, 2);
  assert.deepEqual(Object.values(app.state().sealed.nativeTextIntents), [intent]);
  replacement.resolve(receipt);
  assert.equal(await second, "saved");
  assert.deepEqual(app.state().sealed.nativeTextIntents, {});
  app.done();
});
