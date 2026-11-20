import test from "node:test";
import assert from "node:assert/strict";
import { createNativeTextDriver } from "./native-text-driver.ts";
import type { NativeTextIntents } from "./native-text-intents.ts";
const settings={displayName:"Catte",serverName:"  ",advertise:"",relay:"",rendezvous:"",isDm:false};
function fixture(){
  let intents:NativeTextIntents={},sealed:NativeTextIntents={},draws=0,current=true,fail=false;
  const calls:{command:string,args:Record<string,unknown>,sealed:NativeTextIntents}[]=[];
  let reply:(command:string,args:Record<string,unknown>)=>Promise<unknown>=async(_command,args)=>({backend:"text-v2",server:1,context:args.requestId,channel:"9",is_dm:false});
  const driver=createNativeTextDriver({read:()=>intents,replace:next=>{intents=next;},seal:async()=>{if(fail)throw new Error("disk failure");sealed=structuredClone(intents);},current:()=>current,
    token:()=> (++draws).toString(16).padStart(64,"0"),invoke:async(command,args)=>{calls.push({command,args:structuredClone(args),sealed:structuredClone(sealed)});return reply(command,args);}});
  return {driver,calls,state:()=>({intents,sealed,draws}),fail:(yes:boolean)=>{fail=yes;},current:(yes:boolean)=>{current=yes;},answer:(fn:typeof reply)=>{reply=fn;}};
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
