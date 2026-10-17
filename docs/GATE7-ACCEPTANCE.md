# Gate 7 acceptance and evidence ledger

## UI completion candidate — Gate OPEN pending final verification (2026-10-17)

After `f3596ac8` the owner asked to close the remaining UI gaps. This candidate implements
shared-deck PIXA recognition/picker/canvas/focus, a global file viewer from file details, explicit
source/Registry owner fault repair, and separate ticketed Closing local-draft Save. The owner
still explicitly defers two desktops; native audible hardware/platform evidence stays separate.
The current contract and authority limits are in [GATE7-UI-HANDOFF](GATE7-UI-HANDOFF.md).

Focused implementation checks are coherent. Complete suites are now running sequentially,
with `CARGO_BUILD_JOBS=2` and limited test concurrency. Current evidence includes real detached
Closing save/retry, actual rendered repair/Closing buttons and headless Edge global viewer /
shared-deck playback. Final full-suite and build/lint results are not yet claimed.

The read-only actual-diff review found no authority bypass. Its three medium UI findings were
fixed: stale pending fault reads now invalidate, Index/failed-object repair remains reachable,
and separate panels have separate radio names. Bounded failed/held repair details are retained.
The browser boot flow independently caught invalid optional-parameter emission despite passing
Svelte static checks; the declaration was corrected. The focused Edge flow passes with trusted
test clicks and normal audio activation policy. Both the frame labels and independently known
pixels must advance; pause must hold the displayed frame, and fetch caps/permit retirement are
asserted. Interactive browser tooling was unavailable;
headless Edge fixture evidence cannot substitute for a native window or audible hardware test.

The final actual-diff review has no remaining blocker/high/medium finding. Its last low finding
(pixel results returned but omitted from the flow's expected keys) was fixed and the focused
flow passed again. The previous PR review correction ledger below retains its own scope and
execution checkpoint.

### UI completion focused evidence

| Check / log under `target/` | Result |
|---|---|
| Root `closing_control_` (`gate7-ui-closing-focused.log`) | PASS, 1/1: real detached local Save, actual sealed-journal removal, exact accepted retry and refusal of fresh authority |
| Native `studio::` (`gate7-ui-native-focused.log`) | PASS, 70/70 before the final additional Closing delivery-classification regression; that regression is included in the final suite |
| Frontend reader/transport/fault/rendered controls/security/fixture (`gate7-ui-final-focused.log`) | PASS, 59/59 |
| Session/native/surface/fault/reader (`gate7-ui-session-focused.log`) | PASS, 70/70 at the earlier implementation checkpoint; final relevant changes also covered by the row above and final suite |
| `FLOW_SCENARIO=gate7 npm --prefix apps/desktop run test:flows` (`gate7-ui-browser-flow.log`) | PASS: viewer pixels, shared deck/focus pixels, pause, 9 MiB native caps and retired permits; screenshot `gate7-pixa-focus.png` |
| Frontend `check` / `build` (`gate7-ui-{check,build}.log`) | PASS: 0 static errors/warnings; production build with existing large-chunk advisory |

## Gate remains OPEN — PR #46 review corrections (2026-10-17)

The external review of `5a15ba1e` identifies three medium acceptance weaknesses and one low
frontend event mismatch. The earlier statement that implementation acceptance was complete
was too broad. Backend progress does not close the complete Gate 7 delivery.

Corrections in this worktree:

1. One native cross-language scenario reopens real art/Score/SFX from the sealed vault, runs
   production `StudioSession` against actual native invoke handlers over a bounded private
   Node pipe, waits for publication AND reference Save, then has another actor fetch and read
   the generated PIXA through the production session. Independent expectations cover actual
   group/exporter, distinct ordered frames, 17 FPS, exact Score note timing/recipe and SFX hold.
   The original root/TCP/native golden-package references prove custody only, not composed export.
2. The TCP receive-only restart loads the server snapshot from the reopened vault. It carries
   no live snapshot through memory and adds no pre-shutdown save to conceal a receive barrier.
3. Rotation keeps a second live SFX/export alongside the deliberately deleted records. Known
   frame/CID/order, Score link, recipe/descriptor, live SFX note/hold and export CID/length are
   asserted independently at the owner, cold member and final restart; peer agreement is additional.
4. The frontend union/parser accepts `repairing` and `storageRefused`. An all-nine-state real
   session regression pins media invalidation, document/recovery rereads, scope rejection and
   unknown-state rejection without relying on a companion refresh notification.

The Node runner is test-only and starts no nested build. Native CI already provisions Node 22
and frontend dependencies. Actual media command bodies are shared through internal `&AppState`
helpers; their registered signatures, cancellation, generation/incarnation fences and custody
semantics remain unchanged. The parser correction is the only intended production behavior change.
Complete suites remain outstanding; they are not restarted while Gate 7 work is incomplete.

### Review-correction verification

All Rust checks use `CARGO_BUILD_JOBS=2`; focused test binaries use `--test-threads=1` and run
sequentially. Frontend test files use `--test-concurrency=1`. No complete suite is rerun here.

| Check / generated log under `target/` | Result for these corrections |
|---|---|
| `cargo test --offline -p catcoms-app --lib gate7_ -- --test-threads=1` (`gate7-review-preservation.log`) | PASS, 1/1, independent live-content assertions at owner/cold member/restart |
| `cargo test --offline -p catcoms-app --test studio_tcp_acceptance -- --test-threads=1 --nocapture` (`gate7-review-tcp-disk.log`) | PASS, 1/1, receive-only server snapshot loaded from reopened vault, 20.15 s |
| `cargo test --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --lib studio::tests::export_acceptance:: -- --test-threads=1 --nocapture` (`gate7-review-composed-export.log`) | PASS, 2/2, including the final valid-input locked-publication regression, 5.84 s |
| Native `creative_blobs::tests` (`gate7-review-native-media.log`) | PASS, 3/3, input bounds, disk-attachment failure and session/incarnation suppression |
| `studio-native.test.ts`, `studio-session.test.ts`, `studio-media-session.test.ts` (`gate7-review-frontend.log`) | PASS, 57/57; final added foreign-scope assertions separately pass in the all-nine-state regression |
| All-nine-state regression (`gate7-review-events-before.log`, `gate7-review-events-after.log`) | Reproduced failure before parser fix; final regression PASS including foreign-scope and unknown-state rejection |
| Frontend `check` / `build` (`gate7-review-frontend-{check,build}.log`) | PASS, zero errors/warnings in static check; production build succeeds with existing large-chunk advisory |
| Native `cargo check` (`gate7-review-native-check.log`) | PASS |
| Focused root `cargo clippy --offline -p catcoms-app --lib --tests -- -D warnings` (`gate7-review-root-clippy.log`) | PASS |
| `cargo clippy --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --lib --tests -- -D warnings` (`gate7-review-native-clippy.log`) | PASS |
| Both workspace formatting / ambient / raw-store-FS / diff checks | PASS; final diff/format check repeated after any remaining correction |
| Read-only adversarial correction review | PASS; all four original findings and both test-quality follow-ups resolved. Final actual-worktree re-review finds no remaining blocker/high/medium/low |

Frontend focused command: from `apps/desktop`,
`node --experimental-strip-types --import ./scripts/test-dom.mjs --test --test-concurrency=1 src/studio-native.test.ts src/studio-session.test.ts src/studio-media-session.test.ts`.
The final event-only rerun adds `--test-name-pattern="all nine native settlement"` and selects
`src/studio-session.test.ts`. Both retain the required DOM preload before `--test`.

The new native test requires Node 22 and installed desktop npm dependencies, already provisioned
by native CI. It starts only the fixed repository runner with private pipes, at most 96 requests
and 16 MiB per child output line. Each session has a 60 s work deadline, a 5 s exit wait and at
most a further 5 s forced reap. `kill_on_drop` is a final best-effort OS cleanup safeguard.
Event listener registration is simulated in this request/response harness; nine-state event
behavior is covered separately. No Tauri app/ACL construction, desktop window or audible claim
is implied. Recovery export remains a separate backup format.

## Scope and baseline — 2026-10-17

The owner confirms Gate 6 completed and accepted. PR #44 merged into `Create-suite-2` at
`82631a81`; the initial Gate 7 worktree was clean. No hosted CI outcome is inferred from that
acceptance. Earlier results remain attached to their original heads in [GATE6-ACCEPTANCE](GATE6-ACCEPTANCE.md).

Gate 7's fixed definition in [BACKEND-IMPLEMENTATION](BACKEND-IMPLEMENTATION.md) is the complete
create/save/restart/share/join/rotate/recover/export flow through production adapters, including
failure paths, plus the actual command/event/limit/recovery handoff. It does not authorize
reopening accepted earlier gates or adding a second store, transport, synth or receipt owner.
The [UI handoff](GATE7-UI-HANDOFF.md) maps each canonical dependency to its implementation,
frontend-owned rendering or explicitly unavailable control.

## New composed acceptance

| Evidence | What it actually establishes |
|---|---|
| `studio_exchange::tests::acceptance::gate7_create_sound_score_restart_rotate_recover_export_through_actor_custody` | Real actor Ready/vault lease creation, Score note/recipe, PIX/PIXA custody, frame/SFX/link/export Save, cold actor+vault restart, actual idle-owner rotation with published Registry pointer/owner journal, cold member installation of the runtime-issued checkpoint and linked Score, peer media fetch, actual recovery-before-source-replacement, deleted SFX/export refusal, recipe Copy, stale-preview rejection, exact successful retry, decoded scoped backup/media readback and a second cold reopen. No fabricated receipt or staged recovery snapshot. Only quota eligibility uses the existing valid signed title-workload fixture. Transport is simulated. |
| `studio_tcp_acceptance::gate7_tcp_cold_join_fetches_art_score_and_export_then_reopens_sealed_vault` | Two actual libp2p TCP clients, real MLS admission after content creation, cold Index/art/Score catch-up, references separately followed by exact peer-fetched PIX/PIXA bytes, caller size-bound rejection, both clients stopped, cold actor/vault reopen and offline readback/pin enumeration. Injected SystemClock, actual events, real idle cadence and an enclosing deadline avoid accelerating network/parser expiry. This is one process with two actors, not two desktop windows. |
| `studio::tests::native_studio_save_restart_reopen_real_blob_and_complete_projection` | Expanded native production custody/conversion regression now carries art, Score, authored SFX/recipe, linked Music and held export reference through one sealed-vault/actor restart. Exact retry and wrong-family rejection remain covered. Uses the native invoke seam, not a constructed Tauri app. |
| `studio::tests::export_acceptance::gate7_reopened_native_composition_exports_and_other_peer_reads_actual_pixa` | Reopens authored art/Score/SFX from disk, then drives production StudioSession through actual native responses, generated PIXA publication, landed reference Save and another actor's bounded peer fetch/readback. Checks independently known group/exporter, ordered distinct frames, 17 FPS, recipe, Score 125–374 ms note and 375 ms SFX on the second frame. This is the composition-export evidence; the older golden packages establish custody only. |
| Focused frontend bridge/session/media/export/sound tests | Existing TypeScript parser, retry, navigation, preview, media acknowledgement and export rules remain compatible with the composed native/backend contract. IPC is mocked; no native delivery or audible claim follows. |

Existing focused suites additionally cover malicious/malformed input, membership/scope changes,
unopened/Closing checkpoint joining, retained recovery/Copy, owner succession and lifecycle
failure injection. These are complementary evidence; old pins are not current-head passes.

## Historical verification before the PR #46 external review

Historical pre-external-review checkpoint: focused verification and adversarial re-review passed.
That review missed the acceptance gaps listed above; it does not close Gate 7. As requested,
complete suites were started only afterward. The machine subsequently restarted; full backend
verification is incomplete. The native run had already reported a six-client test failure.
Concurrent backend runs were too resource-intensive for this session; subsequent verification
must run sequentially with limited test concurrency. No cause is inferred for the native failure
without a focused rerun. Started or partial checks are not passes.

| Check | Current result |
|---|---|
| Composed actor lifecycle | PASS, 1/1, including rotation discoverability, cold member, stale preview, backup decode and second reopen |
| Real TCP lifecycle | PASS, 1/1 on the final SystemClock/Index/custody assertions (20.26 s) |
| Expanded native restart | PASS, 1/1, including exact initial typed cell/link/recipe/export DTO assertions |
| Focused frontend native/session/media/export/sound | PASS, 65/65 |
| Read-only design / actual-diff adversarial review | PASS. All four medium and two low findings resolved; short actual-worktree re-review finds no remaining blocker/high/medium/low |
| Formatting / ambient / raw-filesystem / diff whitespace | PASS, both workspace formatting checks; Git-Bash ambient/store gates; `git diff --check` |
| `cargo test --all --all-features` | INCOMPLETE: app test binary exited abnormally with code 1; no complete workspace pass |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | INCOMPLETE/FAILURE observed: `six_client_native_reversed_reopen_gossip_loss_and_command_load` failed; run interrupted, no final summary |
| `npm.cmd --prefix apps/desktop test` | PASS, 1563/1563 |
| `cargo deny check` | PASS: advisories, bans, licenses, sources; existing duplicate-version warnings |
| Root strict Clippy / separate native `cargo check` | NOT RUN before restart; final verification remains outstanding |
| Startup / UI flow / frontend check/build | Not rerun: changes are backend/native tests and documentation only; production setup, runtime, Svelte and build inputs are unchanged |

Local command logs are under `target/gate7-*.log`; they are generated evidence, not committed
source fixtures. The historical results above predate the review corrections and must not be
represented as final-head verification. Current changes add only test-side Tokio process/IO
features, internal native seams, acceptance coverage and the documented event-parser correction.

## Explicit limitations and release follow-ups

- Ordinary views remain provisional/local; no new all-history finality or peer custody claim.
- Retained recovery backups are private `p1-recovery-v1` evidence, not playable PIXA or a
  media archive. Copy is not proof that every losing alternative/branch was preserved.
- SFX/export id reuse after a compact seed remains the Gate 6 accepted ordinary-authoring
  limitation. Retained negative recovery evidence still forbids resurrection through Restore/Copy.
- Direct Closing Save, explicit source/Registry repair and shared-deck/global PIXA viewing
  are implemented by the current UI completion candidate above, with the authority and
  availability limits in the UI handoff. This supersedes their absence at earlier checkpoints.
- A shared verified emoji catalog/chat-click feature remains outside this UI completion
  scope; bounded local emoji JSON import is not that feature.
- Audible native hardware, two desktop UI sessions, macOS old/new stream-8 behavior and
  WebKitGTK WebRTC capability need separate platform release observations. Neither the TCP
  test nor fake IPC/engines establishes them. This ledger does not claim those observations.
- The owner identifies the two-desktop test as the outstanding Gate 6 item and explicitly
  defers it on 2026-10-17 because two desktops are unavailable. It does not block this
  implementation handoff; it is not recorded as passed.
- Hosted/Linux CI must be recorded separately for the final committed head. Local Windows
  results cannot be relabelled hosted/Linux evidence.
