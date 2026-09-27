import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { addPendingSend, matchingPendingSend, pendingSendRetryBlock, resolvePendingSend } from "./pending-sends.ts";
import { MAX_DRAFT_CHARS, planLegacyReadMarkMigration, sanitizeUiContinuity } from "./ui-continuity.ts";
import { persistenceWarning, sendAndRefresh } from "./message-send.ts";

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

/** Shared test fixture for real App save queue, hydration, decisions and submit/retry. */
export function pendingManagerApp(initial: unknown) {
  const dependencies = { initial, addPendingSend, matchingPendingSend, pendingSendRetryBlock, resolvePendingSend,
    MAX_DRAFT_CHARS, planLegacyReadMarkMigration, sanitizeUiContinuity, persistenceWarning, sendAndRefresh };
  return new Function(...Object.keys(dependencies), `
    let locked = false, uiStateReady = false, uiStateLoadGeneration = 1, uiStateSaveTimer;
    let uiStateSaveChain = Promise.resolve(), uiStateSaveFailed = false, uiStateFailureToast = 0;
    let pendingSends = {}, recoveredSendDrafts = {}, pendingSendErrors = {}, pendingSendResolution = null;
    let retryingPendingSends = false, pendingManagerOpen = false, sending = false;
    let drafts = {}, readMarks = {}, statusCursors = {}, fileTrustPolicies = {}, latePast = {}, embedAutoLoad = false;
    let servers = [], cur = null, activeServerId = null, draft = "", replyingTo = "", draftRevisions = {};
    let pendingSendNonce = 0, mentionQuery = null, chatStickToBottom = false, tailLoaded = false;
    let messageWindowScope = "", messages = [], pageTotal = 0, replyingToRow, error = "";
    let sealed = JSON.stringify(sanitizeUiContinuity(initial)), writes = [], savesFail = false, gates = [], submissions = [];
    let submitAnswer = async () => ({ accepted: false, persistence: { status: "pending", reason: "write_failed" } });
    const localStorage = { getItem: () => null, removeItem: () => {} }, flushPendingStatusMarks = () => {};
    const console = { warn() {} }, toast = () => 1, updateToast = () => {}, errorText = String, refresh = async () => {};
    const chatScopeKey = (s,c) => s + ":" + c;
    const chanKey = () => activeServerId === null || !cur?.active ? null : chatScopeKey(activeServerId,cur.active);
    const invoke = async (command, args) => {
      if (command === "get_ui_state") return sealed;
      if (command === "durable_send_context") return "a".repeat(64);
      if (command !== "save_ui_state") throw new Error("unexpected command " + command);
      const generation = uiStateLoadGeneration;
      writes.push(JSON.parse(args.json));
      const gate = gates.shift();
      if (gate) await gate;
      if (savesFail) throw new Error("injected disk failure");
      if (locked || generation !== uiStateLoadGeneration) throw new Error("native session changed");
      if (new TextEncoder().encode(args.json).length > 1024*1024) throw new Error("UI state exceeds vault size limit");
      sealed = args.json;
    };
    const invokeDebugged = async (_, args) => {
      submissions.push({ args: structuredClone(args), sealed: JSON.parse(sealed) });
      return { value: await submitAnswer(args) };
    };
    ${production("  function queueUiStateSave(", "  function chanKey(")}
    ${production("  async function submitPendingSend(", "  // Inline edit of one of your own messages.")}
    return {
      hydrate: () => loadUiContinuity(uiStateLoadGeneration),
      resolve: resolvePendingMessage, remove: removeRecoveredSendDraft, use: useRecoveredSendDraft,
      submit: token => submitPendingSend(pendingSends[token], uiStateLoadGeneration), retry: retryPendingSends, send,
      save: () => queueUiStateSave(continuityJson),
      holdSave: promise => gates.push(promise), failSaves: yes => savesFail = yes,
      answer: fn => submitAnswer = fn,
      manager: value => pendingManagerOpen = value,
      conversation(server, channel) { activeServerId = server; cur = channel === null ? null : { active: channel }; draft = chanKey() ? drafts[chanKey()] ?? "" : ""; },
      type(text) { draft = text; const key = chanKey(); if (key) { drafts[key] = text; draftRevisions[key] = (draftRevisions[key] ?? 0) + 1; } },
      add(intent) { pendingSends = addPendingSend(pendingSends, intent); },
      bulkyPreferences() { latePast = { fixture: "x".repeat(1024*1024) }; },
      lock() { clearTimeout(uiStateSaveTimer); locked = true; uiStateReady = false; uiStateLoadGeneration++; pendingSendResolution = null; pendingSends = {}; recoveredSendDrafts = {}; drafts = {}; draft = ""; },
      unlock() { locked = false; uiStateLoadGeneration++; return loadUiContinuity(uiStateLoadGeneration); },
      done() { clearTimeout(uiStateSaveTimer); },
      state() { return { pendingSends: structuredClone(pendingSends), recoveredSendDrafts: structuredClone(recoveredSendDrafts), drafts: structuredClone(drafts), draft, sealed: JSON.parse(sealed), writes, submissions, pendingSendResolution, locked, error }; }
    };
  `)(...Object.values(dependencies));
}
