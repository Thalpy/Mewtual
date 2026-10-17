import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { soundRecoveryChoices, historicalSoundOperation } from "./studio-sound-recovery.ts";
import { StudioSession } from "./studio-session.ts";
import { parseFlipnoteView, parseRecoveryVersionRead, type FrameSoundContent, type FrameSoundOp, type FlipnoteContent } from "./studio-native.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import { CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, fsource, id32, id64, indexContent, ordinaryView, recoveryListing, settle, version } from "./studio-testkit.ts";

const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
const bytes = new Uint8Array(Buffer.from(vector.pixaHex, "hex"));
const group = vector.group as string, object = id32(1), exportId = id32(2), cid = id64(3), snapshot = version(1).snapshot;
const patch = "6ef42672dfee8b336c82d5466a84dceaa6dd18f405e5a4710ea24c9db47eaf6c";
const exportOp: FrameSoundOp = { op: "set_export", export: exportId, cid, bytes: bytes.length };
function sound(): FrameSoundContent { return { records: {}, readiness: { sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: false } }; }
function put(s: FrameSoundContent, key: string, op: FrameSoundOp, n: number) { s.records[key] = { selected: { value: op, source: fsource(n) }, conflicts: [] }; }
function content(s: FrameSoundContent): FlipnoteContent { return parseFlipnoteView(ordinaryView({ ...flipnoteContent({frames:[]}), sound:s })).content; }
function historical(c: FlipnoteContent) { return { v:1 as const, kind:"recoveryVersion" as const, historical:true as const, channel:CHANNEL, group, version:version(1), content:c }; }

test("sound recovery orders recipes before dependent markers and keeps link/deletion Copy-only", () => {
  const s = sound();
  put(s, `s/${id32(9)}`, {op:"set_sfx",sfx:id32(9),frame:id32(10),patch,note:60,duration_ms:95000}, 9);
  put(s, `p/${patch}`, {op:"set_patch",patch,descriptor:legacyJamPatch("triangle")}, 8);
  put(s, `x/${exportId}/${id64(7)}`, exportOp, 7);
  put(s, "h/score", {op:"set_header",field:"score",value:id32(11)}, 6);
  put(s, `q/${id32(12)}/${id64(5)}`, {op:"remove_sfx",sfx:id32(12)}, 5);
  assert.deepEqual(soundRecoveryChoices(s,"restore").map(v=>v.choice), [8,9,7].map(n=>({kind:"soundRecord",value:id64(n)})));
  assert.deepEqual(soundRecoveryChoices(s,"copy").map(v=>v.choice), [8,9,7,6,5].map(n=>({kind:"soundRecord",value:id64(n)})));
  assert.equal(historicalSoundOperation(content(s), {kind:"soundRecord",value:id64(9)})?.op,"set_sfx");
});

test("raw winning recipe Set cannot override any retained removal during whole-version recovery", () => {
  const s=sound(); put(s,`p/${patch}`,{op:"set_patch",patch,descriptor:legacyJamPatch("triangle")},10);
  s.records[`p/${patch}`].conflicts.push({value:{op:"remove_patch",patch},source:fsource(11)});
  put(s,`s/${id32(3)}`,{op:"set_sfx",sfx:id32(3),frame:id32(4),patch,note:60,duration_ms:100},12);
  put(s,`q/${id32(3)}/${id64(13)}`,{op:"remove_sfx",sfx:id32(3)},13);
  assert.deepEqual(soundRecoveryChoices(s,"restore"),[]);
  assert.deepEqual(soundRecoveryChoices(s,"copy").map(v=>v.choice),[11,13].map(n=>({kind:"soundRecord",value:id64(n)})));
  assert.equal(historicalSoundOperation(content(s),{kind:"soundRecord",value:id64(10)})?.op,"set_patch","explicit alternative remains addressable; a walk never copies every alternative");
});

test("immutable export births choose the smallest source and never silently Restore conflicting births", () => {
  const s=sound(); put(s,`x/${exportId}/${id64(9)}`,exportOp,9); put(s,`x/${exportId}/${id64(8)}`,{...exportOp,cid:id64(5)},8);
  assert.deepEqual(soundRecoveryChoices(s,"restore"),[]);
  assert.deepEqual(soundRecoveryChoices(s,"copy").map(v=>v.choice),[{kind:"soundRecord",value:id64(8)}]);
  put(s,`z/${exportId}/${id64(10)}`,{op:"remove_export",export:exportId},10);
  assert.deepEqual(soundRecoveryChoices(s,"copy").map(v=>v.choice),[{kind:"soundRecord",value:id64(10)}]);
});

test("historical native group survives parsing losslessly, and malformed group never becomes an alias", () => {
  const raw=historical(content(sound())); assert.equal(parseRecoveryVersionRead(raw).group,group);
  for(const invalid of [7,"ABC","a","gg",""]) assert.throws(()=>parseRecoveryVersionRead({...raw,group:invalid}),/stable group/);
  const {group:_,...old}=raw; assert.equal(parseRecoveryVersionRead(old).group,undefined,"legacy art-only recovery remains readable");
});

async function harness() {
  const s=sound(); put(s,`x/${exportId}/${id64(7)}`,exportOp,7);
  const c=content(s), ipc=fakeIpc(), timers:(()=>void)[]=[]; let n=100;
  const session=new StudioSession({ipc,me:ME,newId:()=>id32(++n),schedule:fn=>{timers.push(fn);return()=>{};}});
  const current=()=>({...ordinaryView(content(sound())),group});
  ipc.on("studio_list",()=>ordinaryView(indexContent())); ipc.on("studio_read",current);
  ipc.on("studio_recovery_list",()=>recoveryListing({object,versions:[version(1)]}));
  ipc.on("studio_recovery_read",()=>historical(c));
  ipc.on("studio_recovery_preview",()=>({v:1,kind:"recoveryPreview",snapshot,epochId:id32(0xe1),expectedProjection:id64(70),disposition:"ready",body:JSON.stringify(exportOp),originalAuthor:ME}));
  ipc.on("request_blob_bounded",()=>({bytes:bytes.length,bytes_b64:b64(bytes)}));
  ipc.on("publish_pixa",()=>({cid,bytes:bytes.length}));
  ipc.on("studio_recovery_apply",()=>({v:1,kind:"recoveryApplied",contentSaved:true,alreadySaved:false,provisional:true,pointerRestored:false}));
  const sync=async()=>{for(let pass=0;pass<4;pass++){for(const fn of timers.splice(0))fn();await settle(4);}};
  await session.attach(); session.setScope({server:SERVER,channel:CHANNEL});session.open(object);session.watchRecovery(object);await sync();
  return {session,ipc,sync,c,current,calls:(cmd:string)=>ipc.calls.filter(c=>c.cmd===cmd)};
}

test("real session historical walk fetches and validates exact PIXA before fresh reference Apply",async()=>{
  const h=await harness(); await h.session.runRecovery(snapshot,"restore");
  assert.equal(h.session.recoveryRun?.items[0].state,"applied");
  assert.deepEqual(h.calls("request_blob_bounded")[0].args,{server:SERVER,cid,maxBytes:bytes.length});
  const names=h.ipc.calls.map(c=>c.cmd); assert.ok(names.indexOf("publish_pixa")<names.indexOf("studio_recovery_apply"));
  const edit=h.calls("studio_recovery_apply")[0].args.edit as {choice:unknown;body:string};
  assert.deepEqual(edit.choice,{kind:"soundRecord",value:id64(7)});assert.deepEqual(JSON.parse(edit.body),exportOp);
  assert.equal(h.session.blob(cid),undefined,"PIXA must not enter the PIX-only thumbnail cache");
});

test("missing, invalid, foreign-group or mismatched-publication PIXA never reaches Apply",async()=>{
  for(const fault of ["missing","invalid","foreign","cid","preview"]){
    const h=await harness();
    if(fault==="missing")h.ipc.on("request_blob_bounded",()=>null);
    if(fault==="invalid")h.ipc.on("request_blob_bounded",()=>({bytes:bytes.length,bytes_b64:b64(new Uint8Array(bytes.length))}));
    if(fault==="foreign")h.ipc.on("studio_recovery_read",()=>({...historical(h.c),group:"ffff"}));
    if(fault==="cid")h.ipc.on("publish_pixa",()=>({cid:id64(99),bytes:bytes.length}));
    if(fault==="preview")h.ipc.on("studio_recovery_preview",()=>({v:1,kind:"recoveryPreview",snapshot,epochId:id32(0xe1),expectedProjection:id64(70),disposition:"ready",body:JSON.stringify({...exportOp,cid:id64(99)}),originalAuthor:ME}));
    await h.session.runRecovery(snapshot,"restore"); assert.equal(h.session.recoveryRun?.items[0].state,"error",fault);assert.equal(h.calls("studio_recovery_apply").length,0,fault);
  }
});

test("navigation across held-package fetch fences publication and reference Apply",async()=>{
  const h=await harness(), held=deferred<unknown>(), started=deferred<void>();
  h.ipc.on("request_blob_bounded",()=>{started.resolve();return held.promise;});
  const run=h.session.runRecovery(snapshot,"restore");await started.promise;h.session.setScope({server:SERVER+1,channel:CHANNEL});
  held.resolve({bytes:bytes.length,bytes_b64:b64(bytes)});await run;
  assert.equal(h.calls("publish_pixa").length,0);assert.equal(h.calls("studio_recovery_apply").length,0);
});

test("navigation after native publication cannot redirect its historical reference",async()=>{
  const h=await harness(), ack=deferred<unknown>(), started=deferred<void>();
  h.ipc.on("publish_pixa",()=>{started.resolve();return ack.promise;});
  const run=h.session.runRecovery(snapshot,"restore");await started.promise;
  h.session.setScope({server:SERVER+1,channel:CHANNEL});ack.resolve({cid,bytes:bytes.length});await run;
  assert.equal(h.calls("studio_recovery_apply").length,0,"the historical blob may be held, but no redirected reference is written");
});

test("uncertain historical sound Apply retries its frozen identity without fetching or publishing again",async()=>{
  const h=await harness();let attempts=0;
  h.ipc.on("studio_recovery_apply",()=>{if(++attempts===1)throw new Error("delivery lost");return {v:1,kind:"recoveryApplied",contentSaved:true,alreadySaved:true,provisional:true,pointerRestored:false};});
  await h.session.runRecovery(snapshot,"restore");assert.equal(h.session.saves[0]?.status,"uncertain");
  const original=h.calls("studio_recovery_apply")[0].args;
  h.session.retry(h.session.saves[0].id);await h.sync();
  assert.deepEqual(h.calls("studio_recovery_apply")[1].args,original);assert.equal(h.calls("publish_pixa").length,1);assert.equal(h.calls("request_blob_bounded").length,1);assert.equal(h.session.saves.length,0);
});

test("retained Unconfirmed draft lists sound and holds its export before Copy without transferring authority",async()=>{
  const h=await harness();
  h.ipc.on("studio_overlay_read",()=>({v:1,kind:"local-draft",channel:CHANNEL,object,eligibility:"manual",manualReason:"unconfirmed",handoffAttempt:null,unconfirmedState:"awaitingSource",basis:id64(1),branch:id64(2),contentId:id64(3),generation:"1",accepted:1,transferState:"active",provenance:"unconfirmed",replayable:true,readOnly:true,contentEpoch:"3",group,content:h.c}));
  h.ipc.on("studio_overlay_copy_preview",()=>({v:1,kind:"overlayCopyPreview",source:{channel:CHANNEL,object},destination:{channel:CHANNEL,object},epochId:id32(0xe1),expectedProjection:id64(70),disposition:"ready",body:JSON.stringify(exportOp),originalAuthor:ME,sourceOps:[id64(7)],preservesBranch:false,provisional:true}));
  h.ipc.on("studio_overlay_copy_apply",()=>({v:1,kind:"overlayCopyApplied",channel:CHANNEL,object,contentSaved:true,alreadySaved:false,provisional:true,branchPreserved:false}));
  await h.session.refreshOverlay(object);const choice=h.session.overlayChoices("copy")[0];assert.equal(choice.choice.kind,"soundRecord");
  await h.session.copyOverlayChoice(object,choice.choice,choice.label,"copy");
  const names=h.ipc.calls.map(c=>c.cmd);assert.ok(names.indexOf("publish_pixa")<names.indexOf("studio_overlay_copy_apply"));
  assert.equal(h.session.overlay?.copy?.status,"applied");assert.equal(h.calls("studio_unconfirmed_overlay_save").length,0);
});
