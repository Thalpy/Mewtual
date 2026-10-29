import assert from "node:assert/strict";
import test from "node:test";
import { NativeControlFrames, NativeControlProgress } from "../scripts/voice-native-control.ts";

test("fast Capacity attempt phases coalesce without consuming the original control allowance", () => {
  const progress = new NativeControlProgress();
  //200 attempts exceed the old85-attempt diagnostic failure, with no change to
  //the attempt/result/deadline. Only the latest phase survives each sample turn.
  for (let attempt = 0; attempt < 200; attempt++) {
    progress.update("http"); progress.update("http-return"); progress.update("idle");
  }
  assert.equal(progress.sample(), "idle");
  assert.equal(progress.sample(), null);
  progress.update("rtc");
  assert.equal(progress.sample(), "rtc", "a stalled awaited phase remains observable");
  progress.reset();
  assert.equal(progress.sample(), null, "a new step cannot inherit old diagnostic progress");
});
const encode = (value: unknown) => {
  const body = Buffer.from(JSON.stringify(value)), head = Buffer.alloc(4); head.writeUInt32BE(body.length);
  return Buffer.concat([head, body]);
};
test("coalesced reply and next step retain logical ownership checks", () => {
  const reader = new NativeControlFrames(), values: unknown[] = [];
  let callback = true, step = false;
  reader.feed(Buffer.concat([encode({tag:"reply",ok:true}),encode({tag:"step",name:"next"})]), value => {
    assert.equal(reader.pendingBytes,0); const frame = value as {tag:string};
    if (frame.tag === "reply") {assert.equal(callback,true); callback=false;}
    else {assert.equal(step,false); step=true;} values.push(frame.tag);
  });
  assert.deepEqual(values,["reply","step"]); reader.finish();
});
test("every fragmentation boundary followed by a coalesced tail is decoded once in order", () => {
  const bytes=Buffer.concat([encode({n:1}),encode({n:2}),encode({n:3})]);
  for(let at=1;at<bytes.length;at++) {
    const reader=new NativeControlFrames(), values:unknown[]=[];
    reader.feed(bytes.subarray(0,at),value=>values.push(value));
    reader.feed(bytes.subarray(at),value=>values.push(value));reader.finish();
    assert.deepEqual(values,[{n:1},{n:2},{n:3}]);
  }
});
test("one fixed frame buffer processes a larger chunk without a queued frame graph", () => {
  const reader=new NativeControlFrames(32), frames=Array.from({length:100},(_,n)=>encode({n}));let count=0;
  reader.feed(Buffer.concat(frames),value=>{assert.equal(reader.pendingBytes,0);assert.deepEqual(value,{n:count++});});
  assert.equal(count,100);reader.finish();
});
test("zero and oversized frame headers refuse before accepting a body", () => {
  for(const size of [0,4097,0xffffffff]) {
    const reader=new NativeControlFrames(),head=Buffer.alloc(4);head.writeUInt32BE(size);
    assert.throws(()=>reader.feed(head,()=>assert.fail("invalid frame accepted")),/frame length/);
    assert.throws(()=>reader.feed(encode({n:1}),()=>assert.fail()),/unavailable/);
  }
});
test("logical ownership still rejects coalesced duplicate steps", () => {
  const reader=new NativeControlFrames();let running=false;
  assert.throws(()=>reader.feed(Buffer.concat([encode({tag:"step"}),encode({tag:"step"})]),()=>{
    assert.equal(running,false,"steps are serialized");running=true;
  }),/steps are serialized/);
  assert.throws(()=>reader.finish(),/incomplete/);
});
test("partial header/body and invalid JSON fail closed", () => {
  for(const bytes of [Buffer.from([0]),encode({n:1}).subarray(0,5)]) {
    const reader=new NativeControlFrames();reader.feed(bytes,()=>assert.fail());assert.throws(()=>reader.finish(),/incomplete/);
  }
  const reader=new NativeControlFrames();assert.throws(()=>reader.feed(Buffer.from([0,0,0,1,123]),()=>assert.fail()));
  assert.throws(()=>reader.feed(encode({n:1}),()=>assert.fail()),/unavailable/);
});
test("reentrant ingress cannot reuse the current frame buffer", () => {
  const reader=new NativeControlFrames();assert.throws(()=>reader.feed(encode({n:1}),()=>{
    reader.feed(encode({n:2}),()=>assert.fail());
  }),/unavailable/);assert.throws(()=>reader.finish(),/incomplete/);
});


test("caught reentrant ingress still irreversibly fences the outer stream", () => {
  const reader=new NativeControlFrames();let accepts=0;
  assert.throws(()=>reader.feed(Buffer.concat([encode({n:1}),encode({n:3})]),()=>{
    accepts++; assert.throws(()=>reader.feed(encode({n:2}),()=>assert.fail()),/unavailable/);
  }),/unavailable/);
  assert.equal(accepts,1);assert.throws(()=>reader.feed(encode({n:4}),()=>assert.fail()),/unavailable/);
});
test("invalid UTF8 refuses before metadata admission", () => {
  const reader=new NativeControlFrames();
  assert.throws(()=>reader.feed(Buffer.from([0,0,0,2,0xc3,0x28]),()=>assert.fail("invalid UTF8 accepted")));
  assert.throws(()=>reader.feed(encode({n:1}),()=>assert.fail()),/unavailable/);
});
