# Native discovery fixture recovery

## Integrated evidence update (29 October)

All four listed focused regressions now pass on Windows. The capsule inventory
measures2,176/4,096B. The real forced-attachment scenario passes62.91s, and the
original alternate-peer discovery scenario passes61.24s, retaining their original
deadlines/candidates and independent sibling assertions. These local passes do
not establish the older hosted pressure cause or replace the full Linux gate.

Hosted checkpoint0b5c6739 is still red: Linux native passed1,066/failed44. Its
first failure moved to the new forced-attachment scenario's later automatic
WebCrypto pump, which timed out while the parent had no key and one confirmation
row had action1/receipt0/settled1/ACK0/drained0. The remaining recovery fixtures
then hit the closed isolation gate. A separate independent delayed-metadata test
still expected the former8KiB Row refund after the intentional10KiB correction;
that exact expectation is corrected and its focused test passes. The pump stall
remains unresolved. Both Root OS jobs also failed formatting at0b5; the module
ordering/test wrapping are corrected and current Root formatting passes.

The earlier unrun plan and original failure analysis below are retained as
checkpoint history. Full native/Linux acceptance and strict checks remain open;
no timeout has been raised and no passing isolated replay is called a CI fix.

29 October 2026. Test-harness-only leaf from pushed `fbe53dab`; no production protocol,
factory, deadline, attempt, resource ceiling or V2 activation change. Compilation, tests,
compiled layout measurements and lint are **unrun**. Narrow formatting/static review are
separate evidence and do not establish execution or resolve the underlying Linux pressure.

## Evidence and corrected oracle

Hosted Linux job123274490838 completed1,020 passes and39 failures. Its origin was
`native_browser_local_discovery_refusal_selects_other_peer_and_preserves_indexed_sibling`:
an unforced Discover returned Protocol(Capacity), cursor3/stage2 advanced normally to4/3,
and the original observer had8,142ms remaining. Global diagnostics reported publication
stage5 and a missed update. Their `[0,8,0,0,0,0]` values are outcome counters, not pool
counts. They cannot identify an exact allocation or refusal source.

The exact `NativeCompletion::discovery_refusal` marker is set only when `discover()`
itself returns Capacity. It is absent after successful selection, including a later local
refusal. The helper previously required every failed Discover to carry that marker, then
asserted no parent existed. A retained original Pending needs a different continuation;
it cannot be reclassified as unstarted or used to restart discovery authority.

On a fresh Pending, native publication's first possible Capacity is the actual Root pair
call, followed by Root Derive and native attachment/`PreparedPhase::try_bind`. The
registration's earlier blocking `has_publication` and `mark_handoff` return Unauthorized,
not Capacity. A prior stored publication can instead fail native attachment without a new
Root call. None of these observations proves the older64KiB SDP reservation failure's cause.

## Exact bounded recovery

The observed test producer uses the same `spawn_inner`, native execution and shared
publication path. A cfg(test) worker-local capsule records the actual selector, RowId
including serial, allocation address, and immutable Action pointer/serial/version/input
tag/key references/handshake/generation/deadline. It retains no Row/Action/Grant Arc.
Global diagnostics never authorize recovery.

Only an unforced post-selection Capacity with an actual exact retained Pending enters
recovery. One explicitly harness-owned ResolvedRow retains that original allocation. Pure
snapshots repeatedly check the original selected grant/executor/lease/full purpose and
original prepared/publication state, with Action/Clock observations outside Index/State.
The actual Action remains engine-pending; no execution permit or receipt is consumed.

Recovery drives the SAME Cursor and original10s observer deadline. It stops parity-based
Servers pressure and does not call the pre-selection or parent-free oracle again. Stage2
may normally refuse because that Pending occupies the parent; stage0 resumes the exact
original purpose. Every selected row/action must remain identical. Missing/replaced/closed
state, a different selector or a non-Capacity error fails with diagnostics. `Ok(RowId)` is
not sufficient: success requires that exact row to be Live/NotPending with the original
attached Action. No row, key, generation, deadline or attempt is reset/refunded.

## Deterministic real attachment pressure

The new six-client variant holds the actual Root-published Derive action's existing
handoff loan immediately before ordinary `Registration::attach`. The real `try_bind`
must return Capacity and retain its original Action/preparation. A pressure-setup failure
is a named harness assertion, never a fabricated native Capacity. The completed worker
records actual pressure and attachment stage; the helper validates identity, releases
that same loan outside capsule guards, drops actual native completion custody, then lets
ordinary Cursor continuation attach. The existing alternate-peer choice1 and indexed
sibling/Session/key preservation assertions remain unchanged.

## Accounting and outstanding validation

Only thin optional capsule handles enter the actual native worker/completion graph;
the existing8KiB guard still includes them and may refuse. It has not yet been rerun.
The separately harness-owned capsule graph includes loan/binding/snapshot/guard copies
under an explicit4KiB guard. The whole recovery future plus overlapping capsules has a
separate16KiB harness guard. These are candidate **unrun** measurements, not application
funding or permission to enlarge a production allowance. Original Root/row/loan payments
continue to pay their actual owned objects.

New focused tests (all unrun):

```text
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml exact_worker_capsule_is_a_separate_bounded_harness_graph -j2 -- --nocapture
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml recovery_witness_requires_exact_owner_purpose_and_actual_attachment -j2 -- --nocapture
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml recovery_action_observation_cannot_forget_or_replace_a_known_publication -j2 -- --nocapture
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml native_browser_discovery_attachment_capacity_recovers_original_and_indexed_sibling -j2 -- --test-threads=1 --nocapture
```

Rerun both original alternate-discovery peer/media cases, the native producer and complete
Control inventory guards, strict native Clippy/check, then complete integration suites.
Primary owns these runs. This leaf has not run Cargo, rustc, any full suite, a native build,
startup or frontend command. A local passing replay will not establish the remote pressure
cause or replace the next complete Linux/native gate.
