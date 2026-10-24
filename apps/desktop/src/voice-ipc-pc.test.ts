import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeVoicePcIntent,encodeVoicePcIntent } from "./voice-ipc-pc.ts";
const vectors=JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/pc-vectors.json",import.meta.url),"utf8"));
const expected={ parentRow:vectors.parentRow,identity:vectors.identity,handshake:vectors.handshake,key:vectors.key,
  sequence:vectors.sequence,storage:vectors.storage,sdkGeneration:vectors.sdkGeneration,sdkProof:vectors.sdkProof };
test("PC intent matches native full-u64 requests and four closed reply alternatives",()=>{
  for (const vector of vectors.requests) {
    const encoded=encodeVoicePcIntent({...expected,request:vector.name});
    assert.equal(encoded.operation,14); assert.equal(encoded.logical,expected.parentRow);
    assert.equal(Buffer.from(encoded.body).toString("hex"),vector.body);
  }
  for (const vector of vectors.replies) {
    const receipt=decodeVoicePcIntent(Buffer.from(vector.body,"hex"));
    assert.equal(receipt.state,vector.name==="closed-before-identity" ? "closed" : vector.name);
    assert.equal(receipt.identity?.proof ?? null,vector.name==="closed-before-identity" ? null : vectors.proof);
    assert.ok(Object.isFrozen(receipt)); if(receipt.identity) assert.ok(Object.isFrozen(receipt.identity));
    if(receipt.state==="admitted") { assert.equal(receipt.ready,vectors.ready); assert.equal(receipt.remainingMs,123); assert.equal(receipt.localIsLow,true); }
  }
});
test("PC reply refuses implicit Ready, malformed generations and expired admission",()=>{
  const bytes=Buffer.from(vectors.replies[1].body,"hex");
  for(let end=0;end<bytes.length;end++) assert.throws(()=>decodeVoicePcIntent(bytes.subarray(0,end)));
  assert.throws(()=>decodeVoicePcIntent(Buffer.concat([bytes,Buffer.of(0)])));
  for(const [offset,value] of [[0,8],[1,3],[2,0],[2,2],[10,0],[10,2],[43,0],[43,2],[44,2]] as const) {
    const changed=Buffer.from(bytes); changed[offset]=value; assert.throws(()=>decodeVoicePcIntent(changed));
  }
  for(const remaining of [0n,30_001n,(1n<<64n)-1n]) {
    const changed=Buffer.from(bytes); changed.writeBigUInt64BE(remaining,77); assert.throws(()=>decodeVoicePcIntent(changed));
  }
  for(const state of [0,1]) assert.throws(()=>decodeVoicePcIntent(Uint8Array.of(7,state,0)));
  const admitted=decodeVoicePcIntent(bytes); assert.equal(admitted.state,"admitted");
  const closed=decodeVoicePcIntent(Uint8Array.of(7,2,0)); assert.equal("ready" in closed,false);
});
test("PC request rejects noncanonical proofs, numbers and invented operations",()=>{
  for(const sdkGeneration of ["0","01","-1","18446744073709551616"]) assert.throws(()=>encodeVoicePcIntent({...expected,sdkGeneration,request:"admit"}));
  for(const sdkProof of ["","ab","AB".repeat(32)]) assert.throws(()=>encodeVoicePcIntent({...expected,sdkProof,request:"close"}));
  assert.throws(()=>encodeVoicePcIntent({...expected,request:"reopen" as "admit"}));
  const source={...expected,identity:{...expected.identity}};
  const changed=encodeVoicePcIntent({...source,request:"admit"}); source.identity.action="1";
  assert.equal(new DataView(changed.body.buffer).getBigUint64(1),9007199254740993n);
});
