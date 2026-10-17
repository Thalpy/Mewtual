# PR #29 review corrections

Review baseline: `f77c82c6c14ad9b974c46df6b0059b6b5852c68b`.
The supplied review identified four defects. Each correction is implemented in an isolated
worktree and integrated as a separate feature commit. This is bounded review remediation,
not acceptance of the complete communication-recovery programme or Gate 4.

## Review contracts

| Finding | Required correction | Preserved boundary |
| --- | --- | --- |
| R1: finalization starvation | Filter completed peers before a separate per-pass budget; rotate unfinished work; retain all outstanding candidates | Connected-only authenticated member exchange and exact registered actor |
| R2: loss of another member's restart route | Separate peer, transport and byte limits; preserve peer diversity through save, restore and dialing | Sealed local observations, current member/descriptor validation and shared endpoint budgets |
| R3: unresolved pending sends exhaust capacity | Vault-wide recovery/cancellation with sealed retirement and explicit ambiguous-acceptance warnings | Original retry identity, commit-before-publication and no automatic replacement send |
| R4: archive-sized retransmission | Retain bounded, locally derived ancestry certificates; count transferred operations as well as convergence | Original signed operations, conservative omission and bounded request work |

Root performed design review during implementation, including runtime route caps beyond the
reported storage cap, fair dialing with a separate work budget, continuity-save ordering,
late IPC completions, and fixed ancestry-index memory/work limits. Final source review and
executed evidence are recorded below as each correction completes.

## Feature commits

| Finding | Integrated commit | Main regression |
| --- | --- | --- |
| R1 | `c352eb7` | Actual capture worker reaches the third member after the first two finalize or fail |
| R2 | `687ad3d` | Cold store restore retains C's private route after B's TCP/QUIC refresh; B is then unavailable |
| R3 | `a66090e` | Vault-level sealed recovery/cancellation, ambiguous acceptance, failed saves and late completions |
| R4 | `2daad64` | Cold provider with 10,000 operations; requester holds 9,999; measure convergence and total payload |
| R3 browser follow-up | `1af0af9` | Actual App, zero servers, recovery/stop confirmations and lock/reopen without publication |

## Verification

The checks below completed on 27 September 2026. Unrun acceptance remains explicit below.
Windows Cargo work runs serially with `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=1`,
and `CARGO_TARGET_DIR=C:\GitHub\Mewtual\target`.

| Check | Result | Local output |
| --- | --- | --- |
| `cargo test --locked --offline -p catcoms-replication --test catchup_cost -j 1 -- --nocapture` against the original replication implementation | Expected failure: convergence and one admission succeeded, but 7,952 operations / 4,691,680 payload bytes crossed 45 pages; transfer assertion expected one operation | `logs/pr29-r4-baseline.log` |
| `cargo test --locked --offline -p catcoms-replication -j 1 -- --test-threads=2 --nocapture` | 274 passed: 226 unit and 48 integration tests; zero doctests selected. The identical cold-provider cost fixture transfers 1 operation / 590 payload bytes / 1 page, admits one operation and converges | `logs/pr29-replication-tests.log` |
| `cargo test --locked --offline -p catcoms-sync --lib -j 1 -- --test-threads=2` | 296 passed, no failures or skips; includes signed 4,098-operation requester/provider cost and cold-restore coverage, anti-stall abuse cases and rotating mesh dials | `logs/pr29-sync-tests.log` |
| `npm.cmd test` in `apps/desktop` | 1,267 passed, no failures or skips; frontend source matches `a66090e` | `logs/pr29-ui-tests.log` |
| `npm.cmd run check` | No errors or warnings | `logs/pr29-ui-check.log` |
| `npm.cmd run build` | Passed; existing bundle-size warning remains | `logs/pr29-ui-build.log` |
| `node scripts/flow-check.mjs` with `FLOW_PORT=5187`, `FLOW_CDP_PORT=9357`, from the isolated pending-send worktree's `apps/desktop` | All 9 headless Edge checks passed, including actual-App zero-server pending recovery and reopen | Captured terminal session 80276, exit 0; no log file written |
| Both workspace `cargo fmt -- --check` commands and `git diff --check` | Passed on the integrated source | Executed locally |
| `cargo test --locked --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --lib -j 1 -- --test-threads=1` | 288 passed; includes both actual capture-worker fairness cases and the B-offline/C-private-route restart with signed history | `logs/pr29-native-tests.log` |
| `cargo test --locked --offline -p catcoms-discovery --lib -j 1 reconnect_retention` | 1 passed | `logs/pr29-discovery-tests.log` |
| `cargo test --locked --offline -p catcoms-app --lib -j 1 -- server_net member_mesh_net_v6 --test-threads=1` | 6 passed; includes old-version migration and exact 8,192-byte acceptance / 8,193-byte refusal | `logs/pr29-store-tests.log` |
| `cargo test --locked --offline -p catcoms-app --lib -j 1 actor::tests -- --test-threads=2` | 28 passed; 2 existing probes ignored | `logs/pr29-actor-tests.log` |
| `cargo clippy --locked --offline -p catcoms-discovery -p catcoms-replication -p catcoms-sync -p catcoms-app --lib --tests -j 1 -- -D warnings` | Passed after an equivalent test assertion was named explicitly | `logs/pr29-core-clippy-final.log` |

The R4 baseline executable was compiled before integrating these fixes, using replication
source from `f77c82c`. The independent fixture creates 10,000 genuine signed operations,
gives the requester the first 9,999, restores the provider from a snapshot, then measures
every exported operation and encoded payload byte. Its assertions distinguish transfer
cost from successful convergence. The unoptimized baseline run took 833.73 seconds;
the corrected run took 764.91 seconds, mostly building the fixture. These timings are
fixture evidence, not a production throughput benchmark. The measured payload reduction is
7,952 operations to one, and 4,691,680 encoded payload bytes to 590.

R3 is independently pushed on `fix/pr29-pending-sends` at `b615bc7`; its frontend source
matches the integrated tested tree. The new UI tests exercise the compiled manager and
actual App functions with deferred storage/IPC. The additional full-App browser regression
(`1af0af9`) hydrates a vault with zero servers and three paused orphaned requests, confirms
recovery/cancellation, then locks and reopens. It asserts sealed retirement, full 40,000-character
text and original retry/reply provenance, and zero publication/context/new-token calls. Native
responses are mocked: this is not a physical filesystem or different-account-switch test.

The frontend suite and native suite originally ran at the unpublished integration checkpoint
`d1f9f43`. Before the remaining feature pushes, equivalent visibility/import/test-assertion
lint fixes were folded into their owning commits. The frontend source is unchanged. The
replication/sync runs and strict core lint check use the final integrated runtime source.

R1 (`c352eb7`), R2 (`687ad3d`), R4 (`2daad64`) and R3 (`a66090e`) were each pushed to
`Chat-method-redesign` separately. The browser regression follows in its own test commit.
The native suite was built before the subsequent equivalent visibility/import
lint cleanup; its new TCP/QUIC fixtures all passed. No runtime algorithm changed in that
cleanup. The store run includes the final exact-byte-boundary assertions.

## Independent review and limits

Read-only cross-agent reviews inspected each implementation and its surrounding consumers;
none found a remaining blocker/high in the declared scope. Root reviewed the designs during
implementation and the final source. The following limits are explicit:

- R1's new worker tests cover completed and failing first pairs. Direct queued/in-flight
  cancellation and registry replacement during that specific worker remain static review plus
  existing general incarnation tests. Canceling the caller stops waiting; it cannot retract an
  already-issued transport request.
- R2 retains at most eight peers and two routes per peer, within 8 KiB of serialized route
  state. Actual recent successful observations have priority when peer capacity is exceeded.
  The new codec writes v6 and reads v1-v5 at their original bounds; older binaries cannot read v6.
- R3 preserves ambiguous acceptance explicitly. Stopping retries cannot retract a committed
  message; manually sending recovered text can duplicate it. Storage failure leaves the
  original request and capacity intact rather than claiming successful retirement.
- R4's transfer-cost claim concerns a causally ordered linear history. A large reverse-causal
  arrival order, evicted old head or highly fragmented graph can still retransmit retained
  operations through the bounded conservative fallback. Large reverse-order transfer cost is
  not covered by the new integration fixture. Legacy cursorless service remains unchanged.

Physical devices, Internet NAT behavior, Linux/macOS and external CI are not exercised by
this local review. The complete app-core suite was not rerun; its changed actor and store
boundaries have focused coverage. The historical broader scheduling discrepancy and other
programme gaps in [STATUS.md](STATUS.md) remain applicable.
