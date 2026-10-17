# Gate 7 acceptance and evidence ledger

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
| Focused frontend bridge/session/media/export/sound tests | Existing TypeScript parser, retry, navigation, preview, media acknowledgement and export rules remain compatible with the composed native/backend contract. IPC is mocked; no native delivery or audible claim follows. |

Existing focused suites additionally cover malicious/malformed input, membership/scope changes,
unopened/Closing checkpoint joining, retained recovery/Copy, owner succession and lifecycle
failure injection. These are complementary evidence; old pins are not current-head passes.

## Verification

Implementation and focused verification are in progress. Per owner request, complete suites
are deferred until Gate 7's implementation and review are coherent. The final results below
must be filled from completed commands; started, queued or partial checks are not passes.

| Check | Current result |
|---|---|
| Composed actor lifecycle | PASS, 1/1, including rotation discoverability, cold member, stale preview, backup decode and second reopen |
| Real TCP lifecycle | PASS, 1/1 |
| Expanded native restart | PASS, 1/1, including exact initial typed cell/link/recipe/export DTO assertions |
| Focused frontend native/session/media/export/sound | PASS, 65/65 |
| Read-only design / actual-diff adversarial review | No blocker/high. Four medium acceptance weaknesses and two low handoff wording findings corrected; short re-review pending |
| Formatting / ambient / raw-filesystem / diff whitespace | PASS, both workspace formatting checks; Git-Bash ambient/store gates; `git diff --check` |
| Required full suites / format / lint / ambient / dependency gates | Deferred until implementation complete |

Local command logs are under `target/gate7-*.log`; they are generated evidence, not committed
source fixtures. No dependency/version, test policy or production behavior changes are made.

## Explicit limitations and release follow-ups

- Ordinary views remain provisional/local; no new all-history finality or peer custody claim.
- Retained recovery backups are private `p1-recovery-v1` evidence, not playable PIXA or a
  media archive. Copy is not proof that every losing alternative/branch was preserved.
- SFX/export id reuse after a compact seed remains the Gate 6 accepted ordinary-authoring
  limitation. Retained negative recovery evidence still forbids resurrection through Restore/Copy.
- Direct Closing Save and explicit native repair commands remain unavailable; runtime
  `repairing`/`storageRefused` events are real notifications, not command availability.
- Shared-deck PIXA consensus/media-kind, standalone global viewer and shared verified emoji
  catalog/chat clicks remain outside this backend/handoff acceptance implementation.
- Audible native hardware, two desktop UI sessions, macOS old/new stream-8 behavior and
  WebKitGTK WebRTC capability need separate platform release observations. Neither the TCP
  test nor fake IPC/engines establishes them. This ledger does not claim those observations.
- Hosted/Linux CI must be recorded separately for the final committed head. Local Windows
  results cannot be relabelled hosted/Linux evidence.
