# Durable send adversarial mutation evidence

On 2026-09-26, two isolated frontend mutations verified that the pending-send regression
tests detect removal of their respective safety guards. Both mutants failed the intended
assertion. Restoring the original source after each mutation restored the selected test to
passing; the final unmodified test file passed both tests.

## Scope and isolation

- Base commit: `e67022d9e9b84e34ecb8f5186cfb862dddae2741`.
- Worktree: `target/cr-mutations`, branch `codex/cr-send-mutations`.
- Runtime: Node `v23.7.0`; existing desktop dependencies were accessed through a local
  `node_modules` junction. No dependency installation or Rust build was required.
- Only the isolated worktree's `apps/desktop/src/App.svelte` was temporarily mutated.
  The root checkout's source was not changed.
- Each mutation changed exactly one matched guard. A `finally` block restored the original
  bytes before the next test or mutation. This evidence commit contains only this document.

The tests in `apps/desktop/src/pending-send-recovery.test.ts` extract and execute the actual
production composer and continuity functions with mocked IPC and persistence. These checks
establish frontend guard sensitivity; they do not establish native encryption, disk durability,
network delivery, or the full communication-recovery acceptance matrix.

## Results

All runs below reported zero skipped, cancelled, or todo tests. The numbers are the tests
selected and executed, not the size of the entire frontend suite.

| Source state | Selected tests | Passed | Failed | Exit code |
| --- | ---: | ---: | ---: | ---: |
| Original baseline, both recovery tests | 2 | 2 | 0 | 0 |
| Successful-save barrier bypassed | 1 | 0 | 1 | 1 |
| Original bytes restored after save-barrier mutation | 1 | 1 | 0 | 0 |
| Failed-load readiness forced to true | 1 | 0 | 1 | 1 |
| Original bytes restored after readiness mutation | 1 | 1 | 0 | 0 |
| Final restored file, both recovery tests | 2 | 2 | 0 | 0 |

### Successful-save barrier

Inside `submitPendingSend`, the single expression
`|| !(await saveUiStateImmediately()) || locked || session !== uiStateLoadGeneration`
was replaced with
`|| false || locked || session !== uiStateLoadGeneration`.

The selected test, `retry refuses IPC until the caller identity survives an actual successful
continuity save`, failed at `pending-send-recovery.test.ts:55:10`:

```text
Retry must not bypass the failed first barrier
expected: 0
actual: 1
operator: strictEqual
```

The failing value is the number of native submissions after retrying while continuity saves
still fail. The mutation allowed one submission without a successfully saved caller identity.
With the guard restored, the test also verifies that a later successful save contains the
same token, payload, and authoring context passed to IPC.

### Failed-load readiness

The single `uiStateReady = loaded;` assignment in the continuity hydration completion path
was replaced with `uiStateReady = true;`.

The selected test, `failed continuity hydration cannot replace the sealed pending identities
with empty state`, failed at `pending-send-recovery.test.ts:98:10`:

```text
Expected values to be strictly equal:
true !== false
expected: false
actual: true
operator: strictEqual
```

The failed assertion checks that an authenticated continuity read failure leaves readiness
false. With the original assignment restored, the test verifies that pending identities and
drafts survive, writes remain blocked until a successful load, and the original token remains
in the saved continuity state after recovery.

## Commands and restoration

Commands were run from the isolated worktree's `apps/desktop` directory. Each selected command
was run against its mutant and then again after restoring the original bytes:

```powershell
node --experimental-strip-types --test --test-reporter=tap --test-name-pattern '^retry refuses IPC' src/pending-send-recovery.test.ts
node --experimental-strip-types --test --test-reporter=tap --test-name-pattern '^failed continuity hydration' src/pending-send-recovery.test.ts
```

The baseline selected both tests with `--test-name-pattern 'retry refuses IPC|failed continuity hydration'`.
The final restored run omitted the name filter:

```powershell
node --experimental-strip-types --test --test-reporter=tap src/pending-send-recovery.test.ts
```

The original file, the restoration after each mutation, and the final restored file all had
the same SHA-256 digest:

```text
E24D04375141EC0DA6DE7850B8990BCBF6D4C7F1AB4D679B3C739BBF605655D1
```

`git diff --exit-code -- apps/desktop/src/App.svelte` from the worktree root confirmed no source
diff after the final run. Local TAP logs remain in the worktree's ignored
`target/send-mutation-evidence/01-save-barrier-mutant.log` through
`05-final-restored-both.log`; the original byte backup is in the same directory.
