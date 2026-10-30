import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { NativeVaultLockCoordinator } from "./window-close.ts";

const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
function production(start: string, end: string) {
  const from = source.indexOf(start), to = source.indexOf(end, from);
  assert.ok(from >= 0 && to > from, `exercise actual App source: ${start}`);
  return ts.transpile(source.slice(from, to), { target: ts.ScriptTarget.ES2022 });
}
function deferred<T = any>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
async function settle() { for (let n = 0; n < 40; n++) await Promise.resolve(); }
const rows = [{ server: 1, channel: "1", name: "Original", channels: [{ id: "1", name: "general" }], invite: "", is_dm: false }];

/** Actual entry/restore/startup/locked cancellation and close handlers; only I/O/UI rendering differ. */
function entry() {
  const closeStart = source.indexOf("      appWindow.onCloseRequested(async (event) => {");
  const closeEnd = source.indexOf("      // Capture is a native setting,", closeStart);
  assert.ok(closeStart >= 0 && closeEnd > closeStart);
  const close = ts.transpile("[" + source.slice(closeStart, closeEnd) + "];", { target: ts.ScriptTarget.ES2022 });
  return new Function("NativeVaultLockCoordinator", "rows", `
    let locked = true, unlocking = false, unlockAttempt = 0, windowCloseInFlight = false;
    let uiStateLoadGeneration = 0, uiStateReady = false, vaultExists = true, error = "";
    let passphrase = "first secret", revealPassphrase = true, unlockMethod = "sigil";
    let sigilStrokes = [1], sigilDrawing = [2], sigilColors = [], sigilEmojis = [3], sigilWord = "private", melodySeq = [4];
    let inSetup = false, setupStep = "secret", setupFirst = "setup secret", setupPath = "sync", syncIntent = false;
    let closeAfterContinuityError = false, servers = [], restores = 0, loads = 0, resets = 0, pairs = 0, summons = 0, aborts = 0;
    let studio = null, unmounts = 0, events = [], warnings = [], nativePending = null, closeHandler;
    const unlockGates = [], lockGates = [], resumeGates = [], existsGates = [], closeGates = [], unlockPromises = [];
    const explicit = new Map();
    const sessionStorage = { getItem: key => explicit.get(key) ?? null, setItem: (key,value) => explicit.set(key,value), removeItem: key => explicit.delete(key) };
    const unlockSecret = () => passphrase;
    const startSummon = () => { summons++; }, abortSummon = () => { aborts++; };
    const stopPlayback = () => {}, setupCapture = () => {}, setupConfirm = () => {};
    const clearUnlockEntry = () => { passphrase = ""; revealPassphrase = false; };
    const pairBegin = () => { pairs++; };
    const toast = (...args) => warnings.push(args), console = { warn() {} }, vaultUnlockErrorText = String, errorText = String;
    const pendingSendRetry = { cancel() {} }, callMappingSession = { resetController() {}, claimController() {} };
    const callLifecycleSession = { invalidate() {} }, micCaptureSession = callLifecycleSession, videoCaptureSession = callLifecycleSession;
    const screenAudioCaptureSession = callLifecycleSession;
    let mediaUrls = {}, emojiUrls = {};
    const storageHealthCache = new Map(), imageSrcCache = new Map(), stopSearchWorker = () => {};
    const switchServer = async () => {}, rebuildAllUnread = () => {}, retryPendingSends = async () => {};
    const loadNews = async () => {}, refreshAllDmRequests = () => {}, loadInbox = () => {}, refreshAllServerIcons = () => {};
    const loadUiContinuity = async generation => {
      loads++; uiStateReady = false;
      // Sentinel for the actual readiness edge which conditionally unmounts Studio.
      // Entry tests execute the real restore function; rendering/storage are separate fixtures.
      if (studio !== null) { unmounts++; studio = null; }
      await Promise.resolve();
      if (!locked && generation === uiStateLoadGeneration) uiStateReady = true;
    };
    const invoke = async (command, args) => {
      events.push({ command, args });
      if (command === "resume_session") return await (resumeGates.shift()?.promise ?? Promise.resolve(null));
      if (command === "vault_exists") return await (existsGates.shift()?.promise ?? Promise.resolve(true));
      if (command === "close_vault_window") return await (closeGates.shift()?.promise ?? Promise.resolve({ deferred: true, continuity_error: "saved warning", destroy_error: null }));
      throw new Error("unexpected command " + command);
    };
    const invokeDebugged = (command, args) => {
      events.push({ command, args });
      assertCommand(command);
      return (unlockGates.shift()?.promise ?? Promise.resolve(rows)).then(value => ({ value }));
    };
    function assertCommand(command) { if (command !== "unlock") throw new Error("unexpected auth command"); }
    const nativeVaultLock = new NativeVaultLockCoordinator(async json => {
      events.push({ command: "lock_session", args: { json } });
      // Model the DECLARED native custody contract: valid pending JSON is registered
      // before lock commit; a null cancellation does not overwrite failed native debt.
      if (json !== null) nativePending = json;
      const result = await (lockGates.shift()?.promise ?? Promise.resolve({ continuity_error: null }));
      if (!result.continuity_error) nativePending = null;
      return result;
    });
    const originalReset = nativeVaultLock.reset.bind(nativeVaultLock);
    nativeVaultLock.reset = () => { resets++; originalReset(); };
    ${production("  function gateSubmit()", "  function setupRestart()")}
    ${production("  async function setupFinish()", "  // --- Unlock minigames")}
    ${production("  async function refreshVaultGate()", "  // Ctrl+L: drop back")}
    ${production("  function lockScreen(", "  async function found()")}
    const originalRestore = restoreReloaded;
    restoreReloaded = input => { const result = originalRestore(input); if (result) restores++; return result; };
    const originalUnlock = unlock;
    unlock = (...args) => { const promise = originalUnlock(...args); unlockPromises.push(promise); return promise; };
    function startup() {
      ${production("    // F5/HMR remounts only this webview;", "    // Where the stream is,")}
    }
    let videoScopeEpoch = 0, videoEditorScope = "gate", videoReviews = { rescope() {} };
    const studioBeginClose = null, studioSave = null, inCall = false, leaveVoice = () => {};
    let uiStateSaveTimer;
    let closeDrains = true;
    const drainUiStateForClose = async () => closeDrains, continuityJson = () => "{}";
    const appWindow = { onCloseRequested(handler) { closeHandler = handler; }, async destroy() {} }, relaunch = async () => {};
    ${close}
    return {
      gateSubmit, unlock, setupFinish, startup, lock: () => lockScreen(), restore: restoreReloaded,
      close: () => closeHandler({ preventDefault() {} }),
      beginLock: snapshot => nativeVaultLock.begin(snapshot), snapshot: () => nativeVaultLock.snapshot(),
      holdUnlock: gate => unlockGates.push(gate), holdLock: gate => lockGates.push(gate),
      holdResume: gate => resumeGates.push(gate), holdExists: gate => existsGates.push(gate), holdClose: gate => closeGates.push(gate),
      secret(value) { passphrase = value; }, gateUnknown() { vaultExists = null; },
      explicitLock() { explicit.set("catcoms.explicit-lock", "1"); },
      closing(value) { windowCloseInFlight = value; },
      refuseCloseDrain() { closeDrains = false; },
      openStudio(value = "retained unsaved strokes") { if (!locked && uiStateReady) studio = value; else throw new Error("Studio not ready"); },
      finish: () => Promise.all([...unlockPromises]),
      state() { return { locked, unlocking, unlockAttempt, uiStateLoadGeneration, uiStateReady, vaultExists, passphrase, error,
        restores, loads, resets, pairs, summons, aborts, studio, unmounts, setupFirst, syncIntent, events: [...events], nativePending,
        windowCloseInFlight, explicit: explicit.get("catcoms.explicit-lock") }; }
    };
  `)(NativeVaultLockCoordinator, rows);
}

test("actual double Enter sends one unlock and restores once; another Enter cannot unmount live Studio", async () => {
  const app = entry(), reply = deferred(); app.holdUnlock(reply);
  app.gateSubmit(); app.gateSubmit(); await settle();
  assert.equal(app.state().events.filter(e => e.command === "unlock").length, 1);
  reply.resolve(rows); await app.finish(); await settle(); app.openStudio();
  app.gateSubmit(); await app.unlock(); assert.equal(app.restore(rows), false);
  assert.equal(app.state().restores, 1); assert.equal(app.state().loads, 1);
  assert.equal(app.state().studio, "retained unsaved strokes"); assert.equal(app.state().unmounts, 0);
  assert.equal(app.state().unlocking, false);
});

test("locked cancellation before prior lock settles keeps its exact snapshot and dispatches no unlock", async () => {
  const app = entry(), lock = deferred(); app.holdLock(lock);
  const snapshot = '{"version":3,"drafts":{"room":"latest"},"readMarks":{}}';
  const prior = app.beginLock(snapshot), opening = app.unlock(); await settle(); app.lock();
  assert.equal(app.snapshot(), snapshot); assert.equal(app.state().resets, 0);
  assert.deepEqual(app.state().events.filter(e => e.command === "lock_session").map(e => e.args.json), [snapshot]);
  lock.resolve({ continuity_error: null }); await prior; assert.equal(await opening, null);
  assert.equal(app.state().events.filter(e => e.command === "unlock").length, 0);
  assert.equal(app.state().locked, true); assert.equal(app.state().explicit, "1");
});

test("locked cancellation after dispatch owns a new negative lock; stale error/finally cannot touch a newer attempt", async () => {
  const app = entry(), old = deferred(), negative = deferred(), fresh = deferred();
  app.holdUnlock(old); const opening = app.unlock(); await settle();
  app.holdLock(negative); app.lock(); app.secret("new secret"); app.holdUnlock(fresh);
  const newer = app.unlock(); await settle();
  assert.equal(app.state().events.filter(e => e.command === "unlock").length, 1, "new attempt joins exact cancellation lock");
  negative.resolve({ continuity_error: null }); await settle();
  assert.equal(app.state().events.filter(e => e.command === "unlock").length, 2);
  const current = app.state(); old.reject(new Error("stale wrong secret")); assert.equal(await opening, null);
  assert.equal(app.state().unlocking, true); assert.equal(app.state().passphrase, "new secret");
  assert.equal(app.state().error, current.error); assert.equal(app.state().aborts, current.aborts); assert.equal(app.state().resets, current.resets);
  fresh.resolve(rows); assert.ok(await newer); await settle();
  assert.equal(app.state().restores, 1); assert.equal(app.state().unlocking, false);
});

test("stale successful unlock after a newer session opens cannot reset its coordinator or live Studio", async () => {
  const app = entry(), old = deferred(); app.holdUnlock(old); const opening = app.unlock(); await settle(); app.lock();
  app.secret("new secret"); assert.ok(await app.unlock()); await settle(); app.openStudio();
  const current = app.state(); old.resolve(rows); assert.equal(await opening, null); await settle();
  assert.equal(app.state().resets, current.resets); assert.equal(app.state().restores, current.restores);
  assert.equal(app.state().studio, current.studio); assert.equal(app.state().unmounts, 0);
});

test("closing guards both pre-dispatch settle and returned replies without UI resurrection", async () => {
  for (const duringSettle of [true, false]) {
    const app = entry(), gate = deferred();
    let prior: Promise<unknown> | undefined;
    if (duringSettle) { app.holdLock(gate); prior = app.beginLock('{"drafts":{},"readMarks":{}}'); }
    else app.holdUnlock(gate);
    const opening = app.unlock(); await settle(); app.closing(true); const current = app.state();
    gate.resolve(duringSettle ? { continuity_error: null } : rows); await prior; assert.equal(await opening, null);
    assert.equal(app.state().locked, true); assert.equal(app.state().restores, 0);
    assert.equal(app.state().passphrase, current.passphrase); assert.equal(app.state().resets, current.resets);
    assert.equal(app.state().events.filter(e => e.command === "unlock").length, duringSettle ? 0 : 1);
    app.closing(false); app.secret("fresh"); assert.ok(await app.unlock());
  }
});

test("actual close refuses an in-progress unlock instead of racing the native session", async () => {
  const app = entry(), reply = deferred(); app.holdUnlock(reply); const opening = app.unlock(); await settle();
  await app.close(); assert.equal(app.state().windowCloseInFlight, false);
  assert.match(app.state().error, /Wait for this unlock attempt/);
  assert.equal(app.state().events.filter(e => e.command === "close_vault_window").length, 0);
  reply.resolve(rows); assert.ok(await opening);
});

test("setupFinish completes only its own successful session, never a newer unlock", async () => {
  const app = entry(), reply = deferred(); app.holdUnlock(reply); const setup = app.setupFinish(); await settle(); app.lock();
  app.secret("new secret"); assert.ok(await app.unlock()); reply.resolve(rows); await setup;
  assert.equal(app.state().pairs, 0); assert.equal(app.state().syncIntent, false);
  const fresh = entry(); await fresh.setupFinish(); assert.equal(fresh.state().pairs, 1); assert.equal(fresh.state().setupFirst, "");
});

test("rejected settle retains JS snapshot for retry; fulfilled valid IO warning retains native debt through wrong secret/null cancel", async () => {
  const rejected = entry(), failure = deferred(); rejected.holdLock(failure);
  const json = '{"version":3,"drafts":{"room":"retained"},"readMarks":{}}';
  const prior = rejected.beginLock(json), observed = assert.rejects(prior); const opening = rejected.unlock(); await settle();
  failure.reject(new Error("bridge reply lost")); await observed; assert.equal(await opening, null);
  assert.equal(rejected.state().resets, 0); assert.equal(rejected.snapshot(), json);
  rejected.secret("again"); assert.ok(await rejected.unlock());
  assert.deepEqual(rejected.state().events.filter(e => e.command === "lock_session").map(e => e.args.json), [json, json]);

  const app = entry(), warning = deferred(), wrong = deferred(); app.holdLock(warning); const lock = app.beginLock(json);
  warning.resolve({ continuity_error: "valid JSON write failed" }); await lock;
  app.holdUnlock(wrong); const bad = app.unlock(); await settle(); wrong.reject(new Error("wrong secret")); assert.equal(await bad, null);
  assert.equal(app.state().nativePending, json, "host custody survives JS coordinator reset and failed authentication");
  const cancel = deferred(); app.holdLock(cancel); app.lock();
  assert.equal(app.state().events.filter(e => e.command === "lock_session").at(-1).args.json, null);
  assert.equal(app.state().nativePending, json, "null cancellation does not replace native pending JSON");
  cancel.resolve({ continuity_error: "still pending" }); await settle();
});

test("late initial resume or chooseGate cannot reinitialize a manually opened Studio", async () => {
  for (const chooseGate of [false, true]) {
    const app = entry(), reply = deferred();
    if (chooseGate) { app.explicitLock(); app.holdExists(reply); } else app.holdResume(reply);
    app.startup(); await settle(); assert.ok(await app.unlock()); await settle(); app.openStudio();
    const current = app.state(); reply.resolve(chooseGate ? false : rows); await settle();
    assert.equal(app.state().restores, current.restores); assert.equal(app.state().loads, current.loads);
    assert.equal(app.state().vaultExists, true); assert.equal(app.state().studio, current.studio); assert.equal(app.state().unmounts, 0);
  }
});

test("even a failed manual attempt retires startup's old resume result and error fallback", async () => {
  for (const failResume of [false, true]) {
    const app = entry(), resume = deferred(), auth = deferred(); app.holdResume(resume); app.startup();
    app.holdUnlock(auth); const opening = app.unlock(); await settle(); auth.reject(new Error("current failure")); await opening;
    if (failResume) resume.reject(new Error("old startup failure")); else resume.resolve(rows);
    await settle(); assert.equal(app.state().restores, 0); assert.equal(app.state().locked, true);
    assert.equal(app.state().events.filter(e => e.command === "vault_exists").length, 0);
    assert.match(app.state().error, /current failure/);
  }
});

test("canceling initial resume/gate reads replaces them with a fresh rightful gate read", async () => {
  for (const gateRead of [false, true]) {
    const app = entry(), old = deferred(), fresh = deferred(); app.gateUnknown();
    if (gateRead) { app.explicitLock(); app.holdExists(old); } else app.holdResume(old);
    app.startup(); await settle(); app.holdExists(fresh); app.lock(); await settle();
    old.resolve(gateRead ? true : rows); await settle(); assert.equal(app.state().vaultExists, null); assert.equal(app.state().restores, 0);
    fresh.resolve(false); await settle(); assert.equal(app.state().vaultExists, false);
  }
});

test("a surviving deferred close refreshes an unknown gate after closing is released", async () => {
  const app = entry(), resume = deferred(), closing = deferred(), exists = deferred(); app.gateUnknown();
  app.holdResume(resume); app.startup(); app.holdClose(closing); const close = app.close(); await settle();
  resume.resolve(rows); await settle(); assert.equal(app.state().restores, 0); app.holdExists(exists);
  closing.resolve({ deferred: true, continuity_error: "warning", destroy_error: null }); await close;
  assert.equal(app.state().windowCloseInFlight, false); assert.equal(app.state().vaultExists, null);
  exists.resolve(true); await settle(); assert.equal(app.state().vaultExists, true); assert.equal(app.state().locked, true);
});

test("a refused close retires startup even after its closing flag clears", async () => {
  const app = entry(), resume = deferred(); app.gateUnknown(); app.holdResume(resume); app.startup();
  app.refuseCloseDrain(); await app.close(); await settle();
  assert.equal(app.state().windowCloseInFlight, false); assert.equal(app.state().vaultExists, true);
  resume.resolve(rows); await settle();
  assert.equal(app.state().locked, true); assert.equal(app.state().restores, 0);
});
