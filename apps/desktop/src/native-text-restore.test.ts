import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import ts from "typescript";
import {decodeRestoredChatSession} from "./native-text-session.ts";
import {createStudioExit} from "./studio-exit.ts";
import {deferred,settle} from "./pending-manager-fixture.test.ts";
const source=readFileSync(new URL("./App.svelte",import.meta.url),"utf8");
const script=source.slice(source.indexOf(">")+1,source.indexOf("</script>"));
const parsed=ts.createSourceFile("App.ts",script,ts.ScriptTarget.Latest,true,ts.ScriptKind.TS);
function production(name:string){const node=parsed.statements.find(node=>ts.isFunctionDeclaration(node)&&node.name?.text===name);assert.ok(node,`actual App ${name}`);return ts.transpile(node.getText(parsed),{target:ts.ScriptTarget.ES2022});}
const row={backend:"text-v2",server:1,context:"01".repeat(32),channel:"9",name:"Group",display_name:"Catte",is_dm:false};
const exitStart=source.indexOf("  const studioExit = createStudioExit({"),exitEnd=source.indexOf("  const leaveStudioThen",exitStart);
assert.ok(exitStart>0&&exitEnd>exitStart);
const actualExit=ts.transpile(source.slice(exitStart,exitEnd),{target:ts.ScriptTarget.ES2022});
test("actual restore sequences first native read before retry and preserves empty native mode",async()=>{
 for(const rows of [[row],[]]){
  const gate=deferred();const calls:string[]=[];
  const f=new Function("decodeRestoredChatSession","createStudioExit","gate","calls",`
    let locked=true,windowCloseInFlight=false,servers=[],chatBackend=null,uiStateLoadGeneration=0,startTab="join",uiSessionRetired=false,nativeRejoinRequired=false;
    let studioNavigation=0,view="chat",studioHasWork=null,studioClaimNavigation=null,studioSave=null;
    ${actualExit}
    const callMappingSession={claimController(){calls.push("legacy-call");}},toast=()=>{},sessionStorage={removeItem(){}};
    async function loadUiContinuity(){};
    async function switchServer(_id,_committed,route){if(!route.current())return false;calls.push("read-start");await gate.promise;calls.push("read-end");return true;}
    async function retryNativeTextRequests(){if(calls.includes("read-start")&&!calls.includes("read-end"))throw new Error("one native read is already active");calls.push("retry");}
    const rebuildAllUnread=()=>calls.push("legacy-unread"),retryPendingSends=()=>calls.push("legacy-retry"),loadNews=()=>calls.push("legacy-news");
    const refreshAllDmRequests=()=>calls.push("legacy-dm"),loadInbox=()=>calls.push("legacy-inbox"),refreshAllServerIcons=()=>calls.push("legacy-icons");
    ${production("restoreReloaded")}
    return{restore:restoreReloaded,state:()=>({chatBackend,servers,startTab})};
  `)(decodeRestoredChatSession,createStudioExit,gate,calls);
  assert.equal(f.restore({backend:"text-v2",servers:rows}),true);await settle();
  assert.deepEqual(calls,rows.length?["read-start"]:["retry"]);
  assert.equal(f.state().chatBackend,"text-v2");assert.equal(f.state().startTab,"found");gate.resolve();await settle();
  assert.deepEqual(calls,rows.length?["read-start","read-end","retry"]:["retry"]);
 }
});

test("actual removed-vault restore makes no chat or continuity request", () => {
  const inspect = new Function("decodeRestoredChatSession", `
    let locked=true,windowCloseInFlight=false,servers=[],chatBackend=null,uiStateLoadGeneration=0,startTab="join",uiSessionRetired=false,nativeRejoinRequired=false,uiStateReady=false;
    const sessionStorage={removeItem(){}};
    ${production("restoreReloaded")}
    const restored=restoreReloaded({backend:"text-v2",servers:[],recovery:"removed"});
    return {restored,locked,uiStateReady,nativeRejoinRequired,servers,generation:uiStateLoadGeneration};
  `)(decodeRestoredChatSession);
  assert.deepEqual(inspect, {
    restored: true, locked: false, uiStateReady: false, nativeRejoinRequired: true, servers: [], generation: 1,
  });
});

test("actual Native startup actions and keyboard navigation cannot enter Legacy joining or DMs",async()=>{
 const f=new Function(`let chatBackend="text-v2",startTab="found";
   ${["join","applyJoinReply","newDm","addFriend","openNewDm","openAddFriend","enterDmHome","openInbox","startTabArrows","addChannel","channelMenu"].map(production).join("\n")}
   return async()=>{await join();await applyJoinReply();await newDm();await addFriend();await addChannel();if(channelMenu({id:"1",name:"general"}).length)throw new Error("Legacy channel menu");openNewDm();openAddFriend();enterDmHome();openInbox();startTabArrows({key:"ArrowLeft"});return startTab;};
 `)();
 assert.equal(await f(),"found");
 assert.ok(source.includes('chatBackend === "text-v2" && (servers.length === 0 || showAdd)'));
});
test("actual native navigation and visible-view timer never request Legacy surfaces",async()=>{
 const tickStart=source.indexOf("    const tick = setInterval(() => {");const tickEnd=source.indexOf("    }, 60_000);",tickStart);assert.ok(tickStart>=0&&tickEnd>tickStart);
 const tick=ts.transpile(`async function tick(){${source.slice(source.indexOf("{",tickStart)+1,tickEnd)}}`,{target:ts.ScriptTarget.ES2022});
 const f=new Function(`
    let cur={id:1,backend:"text-v2"},servers=[cur],chatBackend="text-v2",activeServerId=1,serverSettingsPage="overview",view="chat",windowCloseInFlight=false,nowTick=0;
    const calls=[];const invoke=()=>{calls.push("legacy-ipc");throw new Error("unsupported Native IPC");};
    const studioExit={begin:()=>({current:()=>true})},pruneTicker=()=>{},memberRoutesVisible=()=>true,refreshMemberRoutes=()=>invoke();
    let cacheStatusSeq=0,cacheStatus={},locked=false;
    ${["switchView","openServerSettings","toggleSpace","refreshFiles","refreshConnectivity","ensureWikiMap","refreshCacheStatus","refreshChannels","rebuildUnread","refreshDelivery"].map(production).join("\n")}
    ${tick}
    return{run:async()=>{switchView("files");switchView("connectivity");switchView("studio");openServerSettings(1);toggleSpace();await refreshFiles();await refreshConnectivity();await ensureWikiMap();await refreshCacheStatus();await refreshChannels(1);await rebuildUnread(1);await refreshDelivery();view="connectivity";await tick();return calls;}};
 `)();
 assert.deepEqual(await f.run(),[]);
});
