import { test } from "node:test";
import assert from "node:assert/strict";
import { VoiceHandleCell } from "./voice-handle-cell.ts";

test("detachment severs live dispatch and queued thunks become inert", async()=>{
  const target={calls:0}, cell=new VoiceHandleCell(target);
  // This factory's only captured input is the cell, never target or its constructor scope.
  function thunk(cell:VoiceHandleCell<{calls:number}>){return()=>cell.dispatch(value=>++value.calls,0);}
  const queued=thunk(cell); assert.equal(queued(),1); await cell.detach();
  assert.equal(cell.detached,true); assert.equal(queued(),0); assert.equal(target.calls,1);
});
test("reentrant detach fences new views and joins actual entered dispatch", async()=>{
  const cell=new VoiceHandleCell({value:7}); let joined=false, pending:Promise<void>|null=null;
  const answer=cell.dispatch(target=>{
    pending=cell.detach(); void pending.then(()=>{joined=true;});
    for(let n=0;n<1000;n++) assert.equal(cell.detach(),pending);
    assert.equal(cell.detached,false); assert.equal(joined,false);
    assert.equal(cell.dispatch(value=>value.value,-1),-1);
    return target.value;
  },-1);
  assert.equal(answer,-1); assert.equal(joined,false); assert.equal(cell.detached,true);
  await pending; assert.equal(joined,true);
});
test("throwing dispatch still returns its actual owner before detachment completes", async()=>{
  const cell=new VoiceHandleCell({secret:7}); let pending:Promise<void>|null=null;
  assert.throws(()=>cell.dispatch(()=>{pending=cell.detach();throw new Error("entered callback failed");},0));
  await pending; assert.equal(cell.detached,true); assert.equal(cell.dispatch(value=>value.secret,0),0);
});
