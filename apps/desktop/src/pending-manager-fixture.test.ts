import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { addPendingSend, durableStopOriginal, durableStopTerminal, retiredStopWarning, matchingPendingMutation, matchingPendingSend, samePendingIntent, pendingSendRetryBlock, resolvePendingSend, recoverQuarantinedIntent, quarantinedPendingIntent, validateMutationPayload, validateSendPayload, pendingContinuityVersion, sendRefusedUnaccepted } from "./pending-sends.ts";
import { MAX_DRAFT_CHARS, planLegacyReadMarkMigration, sanitizeUiContinuity } from "./ui-continuity.ts";
import { persistenceWarning, sendAndRefresh, sendOutcome } from "./message-send.ts";
import { NativeVaultLockCoordinator } from "./window-close.ts";
import { VideoReviewQueue } from "./video-review.ts";

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

/** Shared test fixture for real App save queue, hydration, decisions and submit/retry. */
export function pendingManagerApp(initial: unknown) {
  const dependencies = { initial, addPendingSend, durableStopOriginal, durableStopTerminal, retiredStopWarning, matchingPendingMutation, matchingPendingSend, samePendingIntent, pendingSendRetryBlock, resolvePendingSend, recoverQuarantinedIntent, quarantinedPendingIntent, validateMutationPayload, validateSendPayload, pendingContinuityVersion, sendRefusedUnaccepted,
    MAX_DRAFT_CHARS, planLegacyReadMarkMigration, sanitizeUiContinuity, persistenceWarning, sendAndRefresh, sendOutcome, NativeVaultLockCoordinator, VideoReviewQueue };
  return new Function(...Object.keys(dependencies), `
    let locked = false, uiStateReady = false, uiStateLoadGeneration = 1, uiStateHydrationAttempt = 0, uiStateSaveTimer;
    let uiStateSaveChain = Promise.resolve(), uiStateSaveFailed = false, uiStateFailureToast = 0;
    let pendingSends = {}, recoveredSendDrafts = {}, pendingSendErrors = {}, pendingSendResolution = null, uiContinuityVersion = 1;
    let retryingPendingSends = false, pendingManagerOpen = false, sending = false;
    let windowCloseInFlight = false, unlocking = false, unlockAttempt = 0, closeAfterContinuityError = false;
    // This fixture is an already-known vault; lifecycle tests exercise actual gate reads.
    const vaultExists = true, refreshVaultGate = () => { throw new Error("unexpected startup gate read"); };
    let videoScopeEpoch = 0;
    const videoEditorScope = "closing", videoReviews = new VideoReviewQueue(() => {});
    // These chat tests leave the lazy Studio chunk unmounted. Keep its real continuity
    // fallback and close hooks in scope so the extracted App functions run unchanged.
    let studioSnapshot = null, studioRestore = null, restoredStudioDraft = null;
    const studioBeginClose = null, studioSave = null;
    let drafts = {}, readMarks = {}, statusCursors = {}, fileTrustPolicies = {}, latePast = {}, embedAutoLoad = false;
    let servers = [], cur = null, activeServerId = null, draft = "", replyingTo = "", draftRevisions = {};
    let pendingSendNonce = 0, mentionQuery = null, chatStickToBottom = false, tailLoaded = false;
    let messageWindowScope = "", messages = [], pageTotal = 0, replyingToRow, error = "", warnings = [];
    let sealed = JSON.stringify(sanitizeUiContinuity(initial)), writes = [], savesFail = false, gates = [], readGates = [], submissions = [], stops = [];
    let nativeWrites = Promise.resolve(), nativeLocked = false, nativeGeneration = 1;
    let closeHandler, closeCalls = [], destroyCalls = 0, restartCalls = 0, closeTimers = new Map();
    const hostSetTimeout = globalThis.setTimeout, hostClearTimeout = globalThis.clearTimeout;
    const setTimeout = (callback, delay) => {
      if (delay === 10000) { const key = {}; closeTimers.set(key, callback); return key; }
      return hostSetTimeout(callback, delay);
    };
    const clearTimeout = handle => { if (!closeTimers.delete(handle)) hostClearTimeout(handle); };
    const pendingSendRetry = { cancel() {} };
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
      if (command === "get_ui_state") { const captured = sealed; const gate = readGates.shift(); if (gate) await gate; return captured; }
      if (command === "durable_send_context") { contextCalls++; if (contextGate) await contextGate; return "a".repeat(64); }
      if (command === "durable_mutation_context") { contextCalls++; if (contextGate) await contextGate; return {context: "a".repeat(64), target: "b".repeat(64)}; }
      if (command === "close_vault_window" || command === "lock_session") {
        if (command === "close_vault_window") closeCalls.push(structuredClone(args));
        const write = nativeWrites.then(() => {
          if (args.uiStateJson !== null) sealed = args.uiStateJson;
          nativeLocked = true; nativeGeneration++;
          return { continuity_error: null, deferred: false, destroy_error: null };
        });
        nativeWrites = write.catch(() => {}); return write;
      }
      if (command !== "save_ui_state") throw new Error("unexpected command " + command);
      const generation = nativeGeneration;
      const write = nativeWrites.then(async () => {
        if (nativeLocked || generation !== nativeGeneration) throw new Error("native session changed");
        writes.push(JSON.parse(args.json));
        const gate = gates.shift();
        if (gate) await gate;
        if (savesFail) throw new Error("injected disk failure");
        if (new TextEncoder().encode(args.json).length > 1024*1024) throw new Error("UI state exceeds vault size limit");
        sealed = args.json;
      });
      nativeWrites = write.catch(() => {}); return write;
    };
    const invokeDebugged = async (command, args) => {
      const call = { command, args: structuredClone(args), sealed: JSON.parse(sealed) };
      if (command === "stop_durable_chat") { stops.push(call); return { value: await stopAnswer(args) }; }
      submissions.push(call);
      return { value: await submitAnswer(args) };
    };
    ${production("  function queueUiStateSave(", "  function chanKey(")}
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
      pendingSends = {}; recoveredSendDrafts = {}; drafts = {}; draft = "";
    }
    ${productionCloseCallback()}
    return {
      hydrate: () => loadUiContinuity(uiStateLoadGeneration),
      resolve: resolvePendingMessage, remove: removeRecoveredSendDraft, use: useRecoveredSendDraft,
      submit: token => submitPendingSend(pendingSends[token], uiStateLoadGeneration), retry: retryPendingSends, send,
      save: () => queueUiStateSave(continuityJson),
      saveImmediately: saveUiStateImmediately,
      replaceSealed(value) { sealed = JSON.stringify(value); },
      holdSave: promise => gates.push(promise), failSaves: yes => savesFail = yes,
      holdLoad: promise => readGates.push(promise),
      holdContext: promise => contextGate = promise,
      answer: fn => submitAnswer = fn,
      answerStop: fn => stopAnswer = fn,
      mutation: queueMessageMutation, startEdit, saveEdit, cancelEdit,
      edit(text) { editDraft = text; editRevision++; },
      manager: value => pendingManagerOpen = value,
      conversation(server, channel) { activeServerId = server; cur = channel === null ? null : { active: channel }; draft = chanKey() ? drafts[chanKey()] ?? "" : ""; },
      type(text) { draft = text; const key = chanKey(); if (key) { drafts[key] = text; draftRevisions[key] = (draftRevisions[key] ?? 0) + 1; } },
      reply(id) { replyingTo = id; },
      add(intent) { pendingSends = addPendingSend(pendingSends, intent); },
      bulkyPreferences() { latePast = { fixture: "x".repeat(1024*1024) }; },
      lock() { lockScreen(); },
      close() { return closeHandler({ preventDefault() {} }); },
      review(file) { return videoReviews.request(file, { index: 1, count: 1 }, "open", "chat"); },
      expireCloseWait() { for (const callback of [...closeTimers.values()]) callback(); },
      replaceUiGeneration() { uiStateLoadGeneration++; },
      async unlock() { await nativeVaultLock.settle(); nativeVaultLock.reset(); nativeLocked = false; nativeGeneration++; locked = false; uiStateLoadGeneration++; return loadUiContinuity(uiStateLoadGeneration); },
      done() { clearTimeout(uiStateSaveTimer); },
      state() { return { pendingSends: structuredClone(pendingSends), recoveredSendDrafts: structuredClone(recoveredSendDrafts), drafts: structuredClone(drafts), draft, editingId, editDraft, sealed: JSON.parse(sealed), writes, submissions, stops, warnings, pendingSendResolution, locked, error, closeCalls, windowCloseInFlight, destroyCalls, restartCalls, contextCalls, uiStateReady, videoScopeEpoch, videoReview: videoReviews.current }; }
    };
  `)(...Object.values(dependencies));
}
