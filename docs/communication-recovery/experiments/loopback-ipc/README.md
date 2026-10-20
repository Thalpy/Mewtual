# Packaged WebView IPC experiment

Manual, disposable Windows compatibility harness. This is **not production IPC, live V2 or a
member-media relay**. It never opens an account and uses generated fixture key material only.

The harness tests embedded `http://tauri.localhost` assets, secure-context availability,
an exact numeric-loopback CSP addition made before window creation, actual main/restricted/remote
WebView ACL behavior, CORS/private-network preflight observations, injected accepted-bootstrap
reply loss through stock Tauri fallback, and finite bootstrap allowance across hard refresh.

The framing slice adds versioned HKDF-separated directional HMAC/AES-GCM keys, a fixed
authenticated header before bounded body allocation and an8192-byte fixture plaintext maximum.
Each of the two finite bootstrap grants has a distinct seed. A grant slot and its counters are
never replaced/rearmed. Request replay floors and one shared64-admission allowance are spent
atomically after capacity, before body intake; EOF, bad AEAD and timeout never refund them.
Responses authenticate the entire original request header as well as their own, so reusing a
fixture ID cannot adopt an older reply. This protocol intentionally refuses reordered requests.

Four shared header positions and two authenticated body positions follow actual workers through
completed-task collection. Browser readers are separately capped at two through actual cancellation;
close/abort actively wake a stalled reader. Request plaintext is privately copied before the first
await and cleared after actual encryption settles. Clearing is best effort, not a guarantee about
browser/crypto-library memory. All payloads are generated fixture patterns, with no SDP or account.

The runner also closes both native listeners and binds an independent **keyless** responder at
the exact old permitted port. It sees ciphertext and sends a syntactically fresh forged reply;
the browser must actually receive HTTP200 and fail HMAC verification. This is one concrete port
replacement scenario, not a general confidentiality or hostile-process security certification.

Production action/controller authorization, reservation/phase transfer, framework/COM/browser
allocation accounting, RTC and application integration remain **unimplemented**. Two bootstrap
grants bound successful fixture seed replies, not every internal copy or stock rejection reply.

From the repository root, with the existing pinned registry cache:

```powershell
cargo test --offline --locked --manifest-path docs/communication-recovery/experiments/loopback-ipc/Cargo.toml --target-dir target/loopback-ipc-probe --features custom-protocol
cargo build --offline --locked --manifest-path docs/communication-recovery/experiments/loopback-ipc/Cargo.toml --target-dir target/loopback-ipc-probe --features custom-protocol
node docs/communication-recovery/experiments/loopback-ipc/run.mjs
node --test docs/communication-recovery/experiments/loopback-ipc/framing.test.mjs docs/communication-recovery/experiments/loopback-ipc/child-output.test.mjs docs/communication-recovery/experiments/loopback-ipc/takeover.test.mjs
```

`run.mjs --binary=ABSOLUTE_PATH` can select an already built fixture executable. If using a
shared native target cache, wait until all other native Cargo commands have finished; do not
clean it. The standalone lockfile was resolved offline from the native lock, preserving every
remaining registry package version; only the experiment root package is new. Its direct edges
and pruned graph differ. No production manifest, command, CSP source or asset is changed.

The runner opens temporary actual WebViews under a fresh owned profile. It enables only the
debugging port, with no CSP, origin, certificate or other browser-security bypass flag. Closed
metadata goes to ignored `result.json`; fixture keys, payloads and arbitrary page/native output
are never printed. The exact readiness, acceptance and cleanup assertions must pass before a
result can be recorded. A private-network permission denial is a compatibility result to diagnose,
not something to disable through browser flags.

Recorded on 19 October 2026 with packaged Windows WebView2 `Edg/155.0.4283.62`: the platform-only
run passed, including two actual successful native bootstrap entries after injected reply loss,
one later exhausted entry, restricted/remote ACL denials before handler entry, two CORS
preflights, one accepted GET, one wrong-CORS GET and zero requests at the CSP-excluded listener.
Peak active workers were three. `private_network_preflights` was zero, so this run does not
establish Private Network Access permission/preflight behavior on configurations that require it.
Hard refresh refused another bootstrap. Actual native drain, child output EOF and temporary
profile cleanup completed before evidence publication. A second actual run with `--fault=cleanup`
failed at the intended boundary, printed no success and preserved the previous result unchanged.

The final framing/takeover run also passes: two encrypted8192-byte round trips, bad header/body
and replay refusal, three fresh authenticated admissions, two executed bodies/replies, and one
real keyless replacement receiving8208 ciphertext bytes without the8192-byte plaintext pattern.
The forged reply's freshu64-maximum sequence reaches and fails HMAC verification. Native and
keyless listener cleanup, output EOF and profile cleanup precede success publication. The final
cleanup-fault run again refuses success without replacing the previous result.

Nine Rust unit tests and17 Node tests pass. They cover maximum real crypto, cross-grant/direction
and request binding, finite replay allowance/reordering, coalesced HTTP/body boundaries, completed
worker permits, partial-body deadline/shutdown, stalled reader cancellation/custody, deferred-input
mutation, dribbling header/body deadlines, occupied/pending bind cleanup and child output EOF.
Successful reader cancellation has no second asynchronous cleanup after the final plaintext
check. Its new regression fails with the isolated prior condition and passes with the guard;
the final rebuilt packaged run also passes. Strict fixture Clippy and formatting pass.
Read-only re-review has no remaining code finding.
Logs live in the isolated probe checkout's ignored `target/communication-loopback-*` files;
`result.json` is intentionally not committed.

The **additional standalone dependency audit fails** under the unchanged root deny policy,
including with the Windows target filter. It reports existing pinned Tauri `unic-*` maintenance
advisories and quick-xml0.39.4 via plist1.9.0 (RUSTSEC-2026-0194/0195); the all-target check also
reports proc-macro-error maintenance. Local source tracing finds plist's XML reader uses plain
`Reader` and never iterates attributes or uses `NsReader`, which does not reach these two reported
quick-xml paths. This reachability observation does not make the audit green. All435 registry
versions match the native lock; no new exemption is added. Dependency-policy follow-up remains
open, independently of runtime probe success. Logs: `communication-loopback-framing-deny.log`
and `communication-loopback-framing-deny-windows.log`.
