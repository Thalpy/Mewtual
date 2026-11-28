import test from "node:test";
import assert from "node:assert/strict";
import { createNativeTextDriver, type NativeReviewSettings } from "./native-text-driver.ts";
import type { NativeTextIntents } from "./native-text-intents.ts";
const settings={displayName:"Catte",serverName:"  ",advertise:"",relay:"",rendezvous:"",isDm:false};
function fixture(){
  let intents:NativeTextIntents={},sealed:NativeTextIntents={},draws=0,current=true,fail=false;
  const calls:{command:string,args:Record<string,unknown>,sealed:NativeTextIntents}[]=[];
  let reply:(command:string,args:Record<string,unknown>)=>Promise<unknown>=async(_command,args)=>({backend:"text-v2",server:1,context:args.requestId,channel:"9",is_dm:false});
  const driver=createNativeTextDriver({read:()=>intents,replace:next=>{intents=next;},seal:async()=>{if(fail)throw new Error("disk failure");sealed=structuredClone(intents);},current:()=>current,
    token:()=> (++draws).toString(16).padStart(64,"0"),invoke:async(command,args)=>{calls.push({command,args:structuredClone(args),sealed:structuredClone(sealed)});return reply(command,args);}});
  return {driver,calls,state:()=>({intents,sealed,draws}),fail:(yes:boolean)=>{fail=yes;},current:(yes:boolean)=>{current=yes;},answer:(fn:typeof reply)=>{reply=fn;},
    reopen:()=>{intents=structuredClone(sealed);current=true;}};
}
test("native founder resolves name before sealing and dispatches those exact settings",async()=>{
  const f=fixture();const created=await f.driver.found(settings);assert.equal(created?.server.name,"Catte");
  assert.equal(f.calls[0].args.serverName,"Catte");
  const intent=Object.values(f.calls[0].sealed)[0];assert.equal(intent.kind,"found");if(intent.kind==="found")assert.equal(intent.serverName,"Catte");
  assert.deepEqual(f.state().intents,{});assert.deepEqual(f.state().sealed,{});assert.equal(f.state().draws,1);
});
test("failed critical seal dispatches nothing and retry keeps the original founder token",async()=>{
  const f=fixture();f.fail(true);await assert.rejects(f.driver.found(settings),/disk failure/);assert.equal(f.calls.length,0);
  const token=Object.keys(f.state().intents)[0];f.fail(false);await f.driver.found({...settings});
  assert.equal(f.calls[0].args.requestId,token);assert.equal(f.state().draws,1);
});
test("lost founder reply keeps exact intent and rejects edited retry settings",async()=>{
  const f=fixture();f.answer(async()=>{throw new Error("lost reply");});await assert.rejects(f.driver.found(settings));
  const original=structuredClone(f.calls[0]);
  await assert.rejects(f.driver.found({...settings,serverName:"Different"}),/previous group/);assert.equal(f.calls.length,1);
  f.answer(async(_cmd,args)=>({backend:"text-v2",server:1,context:args.requestId,channel:"9",is_dm:false}));
  await f.driver.found(settings);assert.deepEqual(f.calls[1].args,original.args);assert.equal(f.state().draws,1);
});
test("native Create uncertainty retries same body/token and requires a matching lossless receipt",async()=>{
  const f=fixture();let attempts=0;
  f.answer(async(_cmd,args)=>++attempts===1?{backend:"text-v2",disposition:"uncertain",retry_token:args.retryToken}:{backend:"text-v2",disposition:"saved-local",retry_token:args.retryToken,message_id:"a".repeat(64),ordinal:"18446744073709551615"});
  assert.deepEqual(await f.driver.send(1,"9","02".repeat(32),"original"),{disposition:"uncertain"});
  const saved=await f.driver.send(1,"9","02".repeat(32),"original");assert.equal(saved?.disposition,"saved-local");
  assert.deepEqual(f.calls[0].args,f.calls[1].args);assert.equal(f.state().draws,1);assert.deepEqual(f.state().intents,{});
});
test("native reply custody persists the target and never aliases another target or a plain send", async () => {
  const f = fixture();
  const context = "02".repeat(32), parent = "03".repeat(32), other = "04".repeat(32);
  f.answer(async (_cmd, args) => ({ backend: "text-v2", disposition: "uncertain", retry_token: args.retryToken }));
  await f.driver.send(1, "9", context, "same text", parent);
  await f.driver.send(1, "9", context, "same text", other);
  await f.driver.send(1, "9", context, "same text");
  const original = f.calls[0];
  const intent = original.sealed[String(original.args.retryToken)];
  assert.equal(intent.kind === "create" && intent.replyTo, parent);
  assert.equal(f.calls[1].args.replyTo, other);
  assert.equal(f.calls[2].args.replyTo, null);
  assert.equal(new Set(f.calls.map(call => call.args.retryToken)).size, 3);
  await f.driver.send(1, "9", context, "same text", parent);
  assert.deepEqual(f.calls[3].args, original.args);
  assert.equal(f.state().draws, 3);
});
test("native mutations seal the exact revision and retry without generating another token", async () => {
  const f = fixture();
  const target = { server: 1, channel: "9", expectedContext: "02".repeat(32), messageId: "03".repeat(32), expectedRevision: "04".repeat(32) };
  f.answer(async (_cmd, args) => ({ backend: "text-v2", disposition: "uncertain", retry_token: args.retryToken }));
  await f.driver.change({ kind: "edit", ...target, text: "Edited" });
  await f.driver.change({ kind: "delete", ...target });
  assert.deepEqual(f.calls.map(call => call.command), ["edit_message", "delete_message"]);
  const edit = f.calls[0];
  const original = edit.sealed[String(edit.args.retryToken)];
  assert.equal(original.kind === "edit" && original.expectedRevision, target.expectedRevision);
  assert.equal(Object.hasOwn(f.calls[1].args, "text"), false);
  assert.equal(original.kind, "edit");
  if (original.kind !== "edit") throw new Error("expected an edit");
  assert.equal(await f.driver.retry({ ...original, expectedRevision: target.messageId }), null);
  await f.driver.retry(original);
  assert.deepEqual(f.calls[2].args, edit.args);
  assert.equal(f.state().draws, 2);
});
test("foreign receipts and cleanup failures preserve native custody",async()=>{
  const f=fixture();f.answer(async()=>({backend:"text-v2",disposition:"saved-local",retry_token:"ff".repeat(32),message_id:"a".repeat(64),ordinal:"0"}));
  await assert.rejects(f.driver.send(1,"9","02".repeat(32),"original"),/could not be confirmed/);assert.equal(Object.keys(f.state().intents).length,1);
  f.answer(async(_cmd,args)=>{f.fail(true);return{backend:"text-v2",disposition:"saved-local",retry_token:args.retryToken,message_id:"a".repeat(64),ordinal:"0"};});
  const saved=await f.driver.send(1,"9","02".repeat(32),"original");assert.equal(saved?.disposition,"saved-local");if(saved?.disposition==="saved-local")assert.equal(saved.cleanupPending,true);
  assert.equal(Object.keys(f.state().intents).length,1);assert.equal(f.state().draws,1);
});
test("a replaced UI session never applies late success or retires its retained request",async()=>{
  const f=fixture();f.answer(async(_cmd,args)=>{f.current(false);return{backend:"text-v2",server:1,context:args.requestId,channel:"9",is_dm:false};});
  assert.equal(await f.driver.found(settings),null);assert.equal(Object.keys(f.state().intents).length,1);
});

test("review decisions seal before dispatch and recover the original intent after lock and lost reply", async () => {
  const f = fixture();
  const input: NativeReviewSettings = {
    server: 1, channel: "9", expectedContext: "02".repeat(32), caseId: "03".repeat(32),
    action: "04".repeat(32), originalDigest: "05".repeat(32), removalHeader: "06".repeat(32),
    removalEpoch: "7", previousDecision: "08".repeat(32), choice: "reopen",
  };
  const receipt = (args: Record<string, unknown>) => ({ status: "saved", caseId: args.caseId, intent: args.intent,
    choice: args.choice, decisionBodyId: "09".repeat(32), streamEntryId: "0a".repeat(32) });
  f.fail(true);
  await assert.rejects(f.driver.decideReview(input), /disk failure/);
  assert.equal(f.calls.length, 0);
  f.fail(false);
  f.answer(async (_command, args) => { f.current(false); return receipt(args); });
  assert.equal(await f.driver.decideReview(input), null);
  assert.equal(f.calls[0].command, "decide_native_chat_held_post");
  const original = Object.values(f.calls[0].sealed)[0];
  assert.equal(original.kind, "review");
  if (original.kind !== "review") throw new Error("expected an original decision");
  assert.deepEqual(f.calls[0].args, { ...input, intent: original.token.slice(32) });
  f.reopen();
  await assert.rejects(f.driver.decideReview({ ...input, previousDecision: "0b".repeat(32) }), /pending decision/);
  assert.equal(f.calls.length, 1);
  f.answer(async (_command, args) => ({ ...receipt(args), choice: "reject" }));
  await assert.rejects(f.driver.retryReview(original), /could not be confirmed/);
  assert.deepEqual(Object.values(f.state().sealed), [original]);
  f.answer(async (_command, args) => receipt(args));
  assert.deepEqual(await f.driver.retryReview(original), { status: "saved", cleanupPending: false });
  assert.deepEqual(f.calls[2].args, f.calls[0].args);
  assert.equal(f.state().draws, 1);
  assert.deepEqual(f.state().sealed, {});
});


test("only a matching superseded proof retires a stale review intent", async () => {
  const f = fixture();
  const input: NativeReviewSettings = {
    server: 1, channel: "9", expectedContext: "02".repeat(32), caseId: "03".repeat(32),
    action: "04".repeat(32), originalDigest: "05".repeat(32), removalHeader: "06".repeat(32),
    removalEpoch: "7", previousDecision: "08".repeat(32), choice: "reopen",
  };
  const superseded = (args: Record<string, unknown>) => ({ status: "superseded", caseId: args.caseId, intent: args.intent, choice: args.choice });
  f.answer(async (_command, args) => ({ ...superseded(args), intent: "ff".repeat(16) }));
  await assert.rejects(f.driver.decideReview(input), /could not be confirmed/);
  const [original] = Object.values(f.state().sealed);
  assert.equal(original.kind, "review");
  if (original.kind !== "review") throw new Error("expected a pending review");
  f.answer(async (_command, args) => ({ ...superseded(args), decisionBodyId: "09".repeat(32) }));
  await assert.rejects(f.driver.retryReview(original), /could not be confirmed/);
  f.answer(async (_command, args) => { f.fail(true); return superseded(args); });
  assert.deepEqual(await f.driver.retryReview(original), { status: "superseded", cleanupPending: true });
  assert.deepEqual(Object.values(f.state().intents), [original]);
  assert.deepEqual(Object.values(f.state().sealed), [original]);
  f.fail(false);
  f.answer(async (_command, args) => superseded(args));
  assert.deepEqual(await f.driver.retryReview(original), { status: "superseded", cleanupPending: false });
  assert.deepEqual(f.state().sealed, {});
  assert.equal(f.state().draws, 1);
});
