# Communication recovery review pushes

The feature branches below share the previously pushed `d525054` base. The temporal
bridge branch is stacked on reconnect. The working integration branch remains
`Chat-method-redesign`; it includes both features and the earlier communication work.

| Review branch | Scope | Review base |
| --- | --- | --- |
| `review/communication-ui-retry` | One bounded quiet retry timer for sealed pending sends; preserved token/context and save barrier; permanent refusals pause retry | `d525054` |
| `review/communication-reconnect` | Authenticated reply/member finalization, outbound-only route retention, restart metadata retries, truthful admission warnings and safe close | `d525054` |
| `review/communication-temporal-bridge` | Independent sealed-store regression for an unknown channel crossing a restarted bridge after its origin exits | `review/communication-reconnect` |

Pushed heads:

- [Quiet UI retry: `8d2c954`](https://github.com/Thalpy/Mewtual/compare/d525054...review/communication-ui-retry)
- [Restart-safe reconnect: `eab28d2`](https://github.com/Thalpy/Mewtual/compare/d525054...review/communication-reconnect)
- [Temporal bridge: `7f735a4`](https://github.com/Thalpy/Mewtual/compare/review/communication-reconnect...review/communication-temporal-bridge)

Earlier independent integration pushes remain reviewable by commit:

| Commit | Feature |
| --- | --- |
| `5fc409e` | Truthful persistence and quiet recovery foundation |
| `001cb93` | Save and freeze before clean close |
| `4d19446` | Backup ordering against shutdown persistence |
| `fe747c9` | Bounded catch-up scanning and advancing-page handling |
| `b171292` | Fair admission to a full history-recovery queue |
| `37c6e2f` | Authenticated P2P group policy and legacy distinction |
| `e67022d` | Durable chat acceptance and stable sealed caller retries |
| `dc4fea1` | Mode disclosure and bounded policy publication |
| `d525054` | Acknowledged actor retirement before removing a conversation |

See [quiet retries](PENDING-RETRY.md), [member reconnect](RECONNECT.md),
[admission storage](ADMISSION-STORAGE.md) and [temporal bridging](TEMPORAL-BRIDGE.md)
for the exact contracts and limitations. Native and core backend source on the stacked
bridge branch matches the integration tree. The UI branch excludes native changes;
the reconnect branch excludes the quiet UI retry feature.

Implementation used isolated subagent worktrees with root-owned serial Cargo runs.
Independent adversarial reviews covered candidate-versus-proof authority, removal,
replacement races, storage ordering, close cancellation, same-token retry and test
fixture validity. Corrections include descriptor-free admitted endpoint tracking,
deduplicating targets before their cap, monotonic failed-write reservations and honest
paused-retry wording. No blocker/high remained in the reviewed boundaries.

The supplied CR00–CR11 programme is not complete. Dedicated service enforcement,
opaque multi-hop recovery, complete long-absence/control recovery, authoritative
vacant-leaf owner succession and physical-device/platform acceptance remain outside
this implemented batch. See [STATUS.md](STATUS.md) for the package gap ledger.

## Verification

Final code checkpoint: `45ac997`. Documentation-only commits follow this checkpoint.
Commands use the repository Rust 1.89.0 toolchain, `--locked --offline`, one Cargo job,
no incremental compilation and debug information disabled for the constrained Windows host.

| Check | Result | Local output |
| --- | --- | --- |
| Complete native Rust library suite, one test thread | 285 passed | `logs/cr-final-native-tests.log` |
| Complete sync library suite | 295 passed before the final equivalent lint cleanup | `logs/cr-final-sync-tests.log` |
| App actor tests | 28 passed, 2 existing probes ignored | `logs/cr-final-actor-tests.log` |
| Independent-store temporal bridge | 1 passed | `logs/cr-temporal-bridge-final.log` |
| Network record v5 fixture | 1 passed | `logs/cr-member-net-store-final.log` |
| Complete frontend suite | 1,252 passed before the final warning-only follow-up | `logs/cr-final-frontend-tests.log` |
| Final send/recovery/retry focused frontend tests | 21 passed, including the new warning regression | `logs/cr-final-ui-retry-focused.log` |
| Svelte check | 0 errors, 0 warnings | `logs/cr-final-frontend-check.log` |
| Frontend production build | Passed, existing bundle-size warning | `logs/cr-final-frontend-build.log` |
| Strict core Clippy, library and tests | Passed after the lint corrections | `logs/cr-final-core-clippy.log` |
| Both Rust formatting checks and diff whitespace check | Passed | Executed in the integration worktree |

The final core lint changes are an equivalent optional-value guard and integer ceiling,
plus boxing a private temporary join result. Independent re-review verified all producers
and consumers; the complete native suite subsequently exercised the changed reply path.
The native suite retains its existing unused `PendingApproval` fields warning.

The latest UI retry feature was not rerun through physical WebView or device flows; the
previous eight mocked Edge flows passed before this feature. No supported-platform CI,
physical NAT test, full CR11 acceptance or complete app-core suite pass is claimed. The
historical broad app-core suite discrepancy remains in STATUS.md. Frontend mutation
results are recorded separately in DURABLE-SEND-REVIEW.md.
