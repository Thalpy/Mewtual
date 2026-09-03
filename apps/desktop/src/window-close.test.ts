import assert from "node:assert/strict";
import test from "node:test";
import {
  NativeVaultLockCoordinator,
  type NativeVaultLockOutcome,
} from "./window-close.ts";

test("Ctrl+L retains the exact final snapshot for a racing native-owned close", async () => {
  let finishLock!: (outcome: NativeVaultLockOutcome) => void;
  const submitted: Array<string | null> = [];
  const coordinator = new NativeVaultLockCoordinator((uiStateJson) => {
    submitted.push(uiStateJson);
    return new Promise((resolve) => { finishLock = resolve; });
  });
  const firstLock = coordinator.begin('{"drafts":{"room":"latest"}}');
  const lockAgain = coordinator.begin('{"drafts":{"room":"already cleared"}}');
  assert.deepEqual(submitted, ['{"drafts":{"room":"latest"}}']);
  assert.equal(lockAgain, firstLock);
  assert.equal(coordinator.snapshot(), '{"drafts":{"room":"latest"}}');

  finishLock({ continuity_error: null });
  await firstLock;
});

test("a successful unlock permits a fresh native lock request", async () => {
  const submitted: Array<string | null> = [];
  const coordinator = new NativeVaultLockCoordinator(async (uiStateJson) => {
    submitted.push(uiStateJson);
    return { continuity_error: null };
  });

  await coordinator.begin("first session");
  coordinator.reset();
  assert.equal(coordinator.snapshot(), null);
  await coordinator.begin("second session");
  assert.deepEqual(submitted, ["first session", "second session"]);
});

test("unlock waits for the prior native lock before it starts a new session", async () => {
  let finishLock!: (outcome: NativeVaultLockOutcome) => void;
  const coordinator = new NativeVaultLockCoordinator(
    () => new Promise((resolve) => { finishLock = resolve; }),
  );
  coordinator.begin("latest before Ctrl+L");
  let unlockSent = false;
  const unlocking = coordinator.settle().then(() => { unlockSent = true; });

  await Promise.resolve();
  assert.equal(unlockSent, false);
  finishLock({ continuity_error: null });
  await unlocking;
  assert.equal(unlockSent, true);
});

test("a rejected lock is retained for an exact-snapshot retry before unlock", async () => {
  const submitted: Array<string | null> = [];
  let attempt = 0;
  const coordinator = new NativeVaultLockCoordinator(async (snapshot) => {
    submitted.push(snapshot);
    attempt += 1;
    if (attempt === 1) throw new Error("bridge result was lost");
    return { continuity_error: null };
  });

  coordinator.begin("same immutable snapshot");
  await assert.rejects(coordinator.settle());
  assert.deepEqual(await coordinator.settle(), { continuity_error: null });
  assert.deepEqual(submitted, ["same immutable snapshot", "same immutable snapshot"]);
});
