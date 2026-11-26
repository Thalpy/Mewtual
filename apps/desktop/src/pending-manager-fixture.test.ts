import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { addPendingSend, durableStopOriginal, durableStopTerminal, retiredStopWarning, matchingPendingMutation, matchingPendingSend, samePendingIntent, pendingSendRetryBlock, resolvePendingSend, recoverQuarantinedIntent, quarantinedPendingIntent, validateMutationPayload, validateSendPayload, pendingContinuityVersion, sendRefusedUnaccepted } from "./pending-sends.ts";
import { MAX_DRAFT_CHARS, planLegacyReadMarkMigration, sanitizeUiContinuity } from "./ui-continuity.ts";
import { persistenceWarning, sendAndRefresh, sendOutcome } from "./message-send.ts";
import { closeContinuityAcknowledgement, NativeVaultLockCoordinator } from "./window-close.ts";
import { VideoReviewQueue } from "./video-review.ts";
import { createNativeTextDriver } from "./native-text-driver.ts";
import { decodeNativeReceived, decodeNativeReceivedNotice, readNativeOriginals, readNativeOwnMessage, readNativeReceivedArchive, updateNativeReceived } from "./native-text-history.ts";
import { PendingSendRetry } from "./pending-send-retry.ts";
import { decodeNativeMembershipNotice, decodeRestoredChatSession } from "./native-text-session.ts";
import { sessionContinuationCurrent } from "./viewscope.ts";

export function deferred<T = void>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
export async function settle() { for (let n = 0; n < 30; n++) await Promise.resolve(); }

const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
function production(start: string, end: string) {
  const from = source.indexOf(start), to = source.indexOf(end, from);
  assert.ok(from >= 0 && to > from, "exercise actual App functions");
  return ts.transpile(source.slice(from, to), { target: ts.ScriptTarget.ES2022 });
}

function productionCloseCallback() {
  const from = source.indexOf("      appWindow.onCloseRequested(async (event) => {");
  const to = source.indexOf("      // Capture is a native setting,", from);
  assert.ok(from >= 0 && to > from, "register the actual production close callback");
  return ts.transpile("[" + source.slice(from, to) + "];", { target: ts.ScriptTarget.ES2022 });
}
function productionRetryUpdate(){
  const match=source.match(/\$effect\(\(\) => \{\s*(pendingSendRetry\.update\([\s\S]*?)\s*\}\);/);
  assert.ok(match,"exercise actual App retry scheduler inputs");
  return ts.transpile(`function syncRetrySchedule(){${match[1]}}`,{target:ts.ScriptTarget.ES2022});
}

/** One simulated warm native store, optionally shared across App instances.
 * This models serialized saved bytes, not native authentication or credentials. */
export function pendingManagerBackend(initial:unknown){return{sealed:JSON.stringify(sanitizeUiContinuity(initial)),nativeWrites:Promise.resolve(),nativeLocked:false,nativeGeneration:1};}
/** Shared test fixture for real App save queue, hydration, decisions and submit/retry. */
export function pendingManagerApp(initial: unknown, backend=pendingManagerBackend(initial)) {
  const dependencies = { initial, backend, addPendingSend, durableStopOriginal, durableStopTerminal, retiredStopWarning, matchingPendingMutation, matchingPendingSend, samePendingIntent, pendingSendRetryBlock, resolvePendingSend, recoverQuarantinedIntent, quarantinedPendingIntent, validateMutationPayload, validateSendPayload, pendingContinuityVersion, sendRefusedUnaccepted,
    MAX_DRAFT_CHARS, planLegacyReadMarkMigration, sanitizeUiContinuity, persistenceWarning, sendAndRefresh, sendOutcome, NativeVaultLockCoordinator, closeContinuityAcknowledgement, VideoReviewQueue, createNativeTextDriver, decodeNativeReceived, decodeNativeReceivedNotice, readNativeOriginals, readNativeOwnMessage, readNativeReceivedArchive, updateNativeReceived, PendingSendRetry, decodeNativeMembershipNotice, decodeRestoredChatSession, sessionContinuationCurrent };
  return new Function(...Object.keys(dependencies), `
    let locked = false, uiStateReady = false, uiStateLoadGeneration = 1, uiStateHydrationAttempt = 0, uiStateSaveTimer,uiSessionRetired=false;
    let uiStateSaveChain = Promise.resolve(), uiStateSaveFailed = false, uiStateFailureToast = 0;
    let pendingSends = {}, recoveredSendDrafts = {}, nativeTextIntents = {}, pendingSendErrors = {}, pendingSendResolution = null, uiContinuityVersion = 1;
    let retryingPendingSends = false, pendingManagerOpen = false, sending = false;
    let chatBackend = "legacy", nativeOriginals = [], nativeCursor = null, nativeHistoryLoading = false, nativeHistoryLoaded = false, nativeHistoryError = null, nativeHistoryRevision = 0, nativeRetryGeneration = null, viewGeneration = 1;
    let nativePages = [], nativeHistoryCalls = [];
    let nativeOwnMessages = {}, nativeOwnLoading = false, nativeOwnPages = [], nativeOwnCalls = [];
    let nativeReceived = [], nativeReceivedScope = null, nativeReceivedRevision = 0;
    const nativeReceivedReads = new Map();
    let nativeReceivedPump = null;
    let nativeArchive = null, nativeArchiveRevision = 0;
    let nativeRemovedContexts = {}, membersOpen = false;
    let busy=false,showAdd=false,startTab="join";
    let displayName="Catte",newServerName="Group",advertise="",relay="",rendezvous="";
    let joinInvite = "", joinPreview = null, joinPreviewCode = "", approvedJoinHost = "", joinServerName = "", joinError = "";
    let nativeJoinReplies = [], nativeJoinCalls = [];
    function resetJoinAttempt() { joinPreview = null; joinPreviewCode = ""; approvedJoinHost = ""; }
    const studioExit={begin(){const generation=viewGeneration;return{current:()=>generation===viewGeneration};}};
    // The navigation's dispatch decision is tested separately against the
    // actual switchServer. Here retain the real history read after selection.
    async function switchServer(id,_then,route){if(route&&!route.current())return false;activeServerId=id;cur=servers.find(row=>row.id===id);await refreshNativeHistory();return true;}
    let windowCloseInFlight = false, unlocking = false, unlockAttempt = 0, closeAfterContinuityError = false, closeContinuityRevision = null;
    // This fixture is an already-known vault; lifecycle tests exercise actual gate reads.
    let vaultExists = true, vaultGateReads = 0;
    const refreshVaultGate = () => { if (vaultExists !== null) throw new Error("unexpected startup gate read"); vaultGateReads++; return Promise.resolve(); };
    let videoScopeEpoch = 0;
    const videoEditorScope = "closing", videoReviews = new VideoReviewQueue(() => {});
    // These chat tests leave the lazy Studio chunk unmounted. Keep its real continuity
    // fallback and close hooks in scope so the extracted App functions run unchanged.
    let studioSnapshot = null, studioRestore = null, restoredStudioDraft = null;
    let studioBeginClose = null, studioSave = null;
    // The Studio checkpoint handles App declares beside its continuity state (unmounted here).
    let studioSnapshotAuthor = null, studioAuthorToken = null, studioSealedAuthor = null, studioCheckpointTimer;
    let drafts = {}, readMarks = {}, statusCursors = {}, fileTrustPolicies = {}, latePast = {}, embedAutoLoad = false;
    let servers = [], cur = null, activeServerId = null, draft = "", replyingTo = "", draftRevisions = {};
    let pendingSendNonce = 0, mentionQuery = null, chatStickToBottom = false, tailLoaded = false;
    let messageWindowScope = "", messages = [], pageTotal = 0, replyingToRow, error = "", warnings = [];
    let writes = [], savesFail = false, gates = [], readGates = [], submissions = [], stops = [];
    let closeHandler, closeCalls = [], destroyCalls = 0, restartCalls = 0, closeTimers = new Map();
    const hostSetTimeout = globalThis.setTimeout, hostClearTimeout = globalThis.clearTimeout;
    const saveTimers=new Map();
    const setTimeout = (callback, delay) => {
      if (delay === 10000) { const key = {}; closeTimers.set(key, callback); return key; }
      if (delay === 250) { const key={};saveTimers.set(key,callback);return key; }
      return hostSetTimeout(callback, delay);
    };
    const clearTimeout = handle => { if (!closeTimers.delete(handle)&&!saveTimers.delete(handle)) hostClearTimeout(handle); };
    const retryTimers = new Map();
    const pendingSendRetry = new PendingSendRetry(async session=>{if(!locked && uiStateReady && session===uiStateLoadGeneration)await retryPendingSends(true);},
      {setTimeout(callback,delay){const key={};retryTimers.set(key,{callback,delay});return key;},clearTimeout(key){retryTimers.delete(key);}});
    const callLifecycleSession = { invalidate() {} }, micCaptureSession = callLifecycleSession;
    const videoCaptureSession = callLifecycleSession, screenAudioCaptureSession = callLifecycleSession;
    const inCall = false, leaveVoice = () => {};
    const appWindow = { onCloseRequested(handler) { closeHandler = handler; }, async destroy() { destroyCalls++; } };
    const relaunch = async () => { restartCalls++; };
    let submitAnswer = async () => ({ accepted: false, persistence: { status: "pending", reason: "write_failed" } });
    let stopAnswer = async () => ({ accepted: false, persistence: { status: "durable" }, disposition: "stopped-before-commit" });
    let contextGate = null, contextCalls = 0;
    const localStorage = { getItem: () => null, removeItem: () => {} }, flushPendingStatusMarks = () => {};
    const console = { warn() {} }, toast = text => { warnings.push(text); return 1; }, updateToast = () => {}, errorText = String, refresh = async () => {};
    const chatScopeKey = (s,c) => s + ":" + c;
    const chanKey = () => activeServerId === null || !cur?.active ? null : chatScopeKey(activeServerId,cur.active);
    const invoke = async (command, args) => {
      if (command === "preview_invite" || command === "join_server") {
        nativeJoinCalls.push({ command, args: structuredClone(args) });
        const next = nativeJoinReplies.shift();
        if (next instanceof Error) throw next;
        return await next;
      }
      if (command === "get_message_page" && "ownAction" in args) {
        nativeOwnCalls.push(structuredClone(args));
        const next = nativeOwnPages.shift();
        if (next instanceof Error) throw next;
        return await (next ?? {backend:"text-v2",scope:"own-message-revision",action:args.ownAction,scanning:false,item:null});
      }
      if(command === "get_message_page") { nativeHistoryCalls.push(structuredClone(args));const next=nativePages.shift();if(next instanceof Error)throw next;return await(next ?? {backend:"text-v2",scope:"own-saved-originals",next:"0",end_exclusive:"0",items:[]}); }
      if (command === "get_ui_state") { const captured = backend.sealed; const gate = readGates.shift(); if (gate) await gate; return captured; }
      if (command === "durable_send_context") { contextCalls++; if (contextGate) await contextGate; return "a".repeat(64); }
      if (command === "durable_mutation_context") { contextCalls++; if (contextGate) await contextGate; return {context: "a".repeat(64), target: "b".repeat(64)}; }
      if (command === "close_vault_window" || command === "lock_session") {
        if (command === "close_vault_window") closeCalls.push(structuredClone(args));
        const write = backend.nativeWrites.then(() => {
          if (args.uiStateJson !== null) backend.sealed = args.uiStateJson;
          backend.nativeLocked = true; backend.nativeGeneration++;
          return { continuity_error: null, deferred: false, destroy_error: null };
        });
        backend.nativeWrites = write.catch(() => {}); return write;
      }
      if (command !== "save_ui_state") throw new Error("unexpected command " + command);
      const generation = backend.nativeGeneration;
      const write = backend.nativeWrites.then(async () => {
        if (backend.nativeLocked || generation !== backend.nativeGeneration) throw new Error("native session changed");
        writes.push(JSON.parse(args.json));
        const gate = gates.shift();
        if (gate) await gate;
        if (savesFail) throw new Error("injected disk failure");
        if (new TextEncoder().encode(args.json).length > 1024*1024) throw new Error("UI state exceeds vault size limit");
        backend.sealed = args.json;
      });
      backend.nativeWrites = write.catch(() => {}); return write;
    };
    const invokeDebugged = async (command, args) => {
      const call = { command, args: structuredClone(args), sealed: JSON.parse(backend.sealed) };
      if (command === "stop_durable_chat") { stops.push(call); return { value: await stopAnswer(args) }; }
      submissions.push(call);
      return { value: await submitAnswer(args) };
    };
    ${production("  function queueUiStateSave(", "  function chanKey(")}
    ${production("  function resetNativeTextForLock(", "  function nativeTextDriver(")}
    ${production("  function receiveNativeMembership(", "  $effect(")}
    ${production("  function nativeTextDriver(", "  async function found(")}
    ${production("  async function found(", "  async function join(")}
    ${production("  function restoreReloaded(", "  async function unlock(")}
    ${production("  function retireUiSessionOnUnmount(", "</script>")}
    ${productionRetryUpdate()}
    ${production("  async function submitPendingSend(", "  // Inline edit of one of your own messages.")}
    ${production("  // Inline edit of one of your own messages.", "  // Emoji reactions:").replaceAll('$state(', '(')}
    const nativeVaultLock = new NativeVaultLockCoordinator(snapshot => invoke("lock_session", { uiStateJson: snapshot }));
    // Unrelated media/view redaction is replaced, but uses the same immediate generation
    // invalidation and immutable native-lock snapshot as the actual lockScreen path.
    function lockScreen(nativeAlreadyLocked = false) {
      clearTimeout(uiStateSaveTimer);
      if (locked) return;
      if (!nativeAlreadyLocked) void nativeVaultLock.begin(uiStateReady ? continuityJson() : null);
      locked = true; uiStateReady = false; uiStateLoadGeneration++; pendingSendResolution = null;
      cancelEdit(); // the actual lock path calls this through clearServerView
      pendingSends = {}; recoveredSendDrafts = {}; drafts = {}; draft = "";resetNativeTextForLock();
    }
    ${productionCloseCallback()}
    return {
      hydrate: () => loadUiContinuity(uiStateLoadGeneration),
      resolve: resolvePendingMessage, remove: removeRecoveredSendDraft, use: useRecoveredSendDraft,
      submit: token => submitPendingSend(pendingSends[token], uiStateLoadGeneration), retry: retryPendingSends, send,
      save: () => queueUiStateSave(continuityJson),
      saveImmediately: saveUiStateImmediately,
      replaceSealed(value) { backend.sealed = JSON.stringify(value); },
      holdSave: promise => gates.push(promise), failSaves: yes => savesFail = yes,
      holdLoad: promise => readGates.push(promise),
      holdContext: promise => contextGate = promise,
      answer: fn => submitAnswer = fn,
      answerStop: fn => stopAnswer = fn,
      mutation: queueMessageMutation, startEdit, saveEdit, cancelEdit,
      edit(text) { editDraft = text; editRevision++; },
      manager: value => pendingManagerOpen = value,
      conversation(server, channel) { activeServerId = server; cur = channel === null ? null : { active: channel }; draft = chanKey() ? drafts[chanKey()] ?? "" : ""; },
      nativeConversation(server,channel,context) {chatBackend="text-v2";activeServerId=server;cur={id:server,backend:"text-v2",active:channel,context};servers=[cur];draft=drafts[chanKey()]??"";},
      nativeHistory(...pages) {nativePages=pages;},
      nativeOwnHistory(...pages) { nativeOwnPages = pages; },
      nativeOwn() { return { messages: structuredClone(nativeOwnMessages), calls: nativeOwnCalls }; },
      nativeChange: changeNativeOwnMessage,
      nativeRefresh: refreshNativeHistory,
      nativeArchiveRefresh: refreshNativeArchive,
      archive() { return structuredClone(nativeArchive); },
      receiveNotice: refreshNativeReceived,
      receiveMembership: receiveNativeMembership,
      nativeRemoved: nativeConversationRemoved,
      received() { return structuredClone(nativeReceived); },
      nativeRetry: retryNativeTextRequests,
      nativeFound: found,
      nativeJoin: joinNative,
      setNativeJoin(code, host, ...replies) { chatBackend = "text-v2"; joinInvite = code; approvedJoinHost = host; nativeJoinReplies = replies; },
      nativeJoinCalls: () => nativeJoinCalls,
      dispose:retireUiSessionOnUnmount,
      debounce:scheduleUiStateSave,
      pendingDebounce:()=>saveTimers.values().next().value,
      // Only native auth/rendering are replaced: exercise the same production
      // native redaction, then establish a new UI generation as lock/restore do.
      retireFoundUiForReauth(){resetNativeTextForLock();uiStateLoadGeneration++;viewGeneration++;chatBackend="text-v2";},
      restoreNative(value){locked=true;return restoreReloaded(value);},
      syncRetry: syncRetrySchedule,
      retryTimers:()=>[...retryTimers.values()].map(value=>value.delay),
      fireRetry(){const first=retryTimers.entries().next().value;if(first){retryTimers.delete(first[0]);first[1].callback();}},
      type(text) { draft = text; const key = chanKey(); if (key) { drafts[key] = text; draftRevisions[key] = (draftRevisions[key] ?? 0) + 1; } },
      reply(id) { replyingTo = id; },
      add(intent) { pendingSends = addPendingSend(pendingSends, intent); },
      bulkyPreferences() { latePast = { fixture: "x".repeat(1024*1024) }; },
      lock() { lockScreen(); },
      close() { return closeHandler({ preventDefault() {} }); },
      studioClose(save, author) { studioSave = save; studioAuthorToken = author; },
      unknownVaultGate() { vaultExists = null; },
      review(file) { return videoReviews.request(file, { index: 1, count: 1 }, "open", "chat"); },
      expireCloseWait() { for (const callback of [...closeTimers.values()]) callback(); },
      replaceUiGeneration() { uiStateLoadGeneration++; },
      async unlock() { await nativeVaultLock.settle(); nativeVaultLock.reset(); backend.nativeLocked = false; backend.nativeGeneration++; locked = false; uiStateLoadGeneration++; return loadUiContinuity(uiStateLoadGeneration); },
      done() { clearTimeout(uiStateSaveTimer);pendingSendRetry.cancel(); },
      state() { return { uiSessionRetired,busy,activeServerId,startTab,nativeHistoryLoading,nativeRetryGeneration,uiStateLoadGeneration,nativeHistoryLoaded,nativeHistoryError,nativeCursor,nativeOriginals:structuredClone(nativeOriginals),nativeHistoryCalls,nativeTextIntents: structuredClone(nativeTextIntents), uiContinuityVersion, pendingSends: structuredClone(pendingSends), recoveredSendDrafts: structuredClone(recoveredSendDrafts), drafts: structuredClone(drafts), draft, editingId, editDraft, sealed: JSON.parse(backend.sealed), writes, submissions, stops, warnings, pendingSendResolution, locked, error, closeCalls, windowCloseInFlight, destroyCalls, restartCalls, contextCalls, uiStateReady, videoScopeEpoch, unlockAttempt, vaultGateReads, videoReview: videoReviews.current }; }
    };
  `)(...Object.values(dependencies));
}
