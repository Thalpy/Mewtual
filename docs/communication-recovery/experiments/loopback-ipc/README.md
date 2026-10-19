# Packaged WebView IPC experiment

Manual, disposable Windows compatibility harness. This is **not production IPC, live V2 or a
member-media relay**. It never opens an account and uses generated fixture key material only.

The first slice tests embedded `http://tauri.localhost` assets, secure-context availability,
an exact numeric-loopback CSP addition made before window creation, actual main/restricted/remote
WebView ACL behavior, CORS/private-network preflight observations, injected accepted-bootstrap
reply loss through stock Tauri fallback, and finite bootstrap allowance across hard refresh.

Authenticated binary framing, port-takeover resistance, action/controller ownership, browser
engine accounting and full native/socket/body custody are **not implemented** by this slice.
Its fixed GET response does not carry SDP or application state. Framework/COM/browser allocation
counts remain unmeasured. Two fixed bootstrap grants bound successful fixture key replies, not
every possible internal framework copy or rejection reply.

From the repository root, with the existing pinned registry cache:

```powershell
cargo test --offline --locked --manifest-path docs/communication-recovery/experiments/loopback-ipc/Cargo.toml --target-dir target/loopback-ipc-probe --features custom-protocol
cargo build --offline --locked --manifest-path docs/communication-recovery/experiments/loopback-ipc/Cargo.toml --target-dir target/loopback-ipc-probe --features custom-protocol
node docs/communication-recovery/experiments/loopback-ipc/run.mjs
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

Recorded on 19 October 2026 with packaged Windows WebView2 `Edg/155.0.4283.62`: the healthy
run passed, including two actual successful native bootstrap entries after injected reply loss,
one later exhausted entry, restricted/remote ACL denials before handler entry, two CORS
preflights, one accepted GET, one wrong-CORS GET and zero requests at the CSP-excluded listener.
Peak active workers were three. `private_network_preflights` was zero, so this run does not
establish Private Network Access permission/preflight behavior on configurations that require it.
Hard refresh refused another bootstrap. Actual native drain, child output EOF and temporary
profile cleanup completed before evidence publication. A second actual run with `--fault=cleanup`
failed at the intended boundary, printed no success and preserved the previous result unchanged.

Three Rust unit tests cover the closed request parser, generated Tauri ACL and completed-worker
permit retention. Two Node tests cover child-exit/output-EOF ordering. Read-only adversarial
re-review has no remaining finding in this slice. Logs live in the isolated probe checkout's
ignored `target/communication-loopback-*` files; `result.json` is intentionally not committed.
