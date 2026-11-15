import test from "node:test";
import assert from "node:assert/strict";
import { decodeNativeOriginalPage,readNativeOriginals } from "./native-text-history.ts";
const item={ordinal:"0",action:"01".repeat(32),signer:"02".repeat(32),channel:"9",presented_ms:"18446744073709551615",status:"pending",body:{kind:"create",text:"original",reply_to:null}};
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
