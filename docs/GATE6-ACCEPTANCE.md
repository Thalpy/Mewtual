# Gate 6 acceptance and evidence ledger

## Combined PR #44 candidate — 2026-10-16, post-merge corrections, hosted acceptance pending

PR #45 was normal-merged into PR #44's branch `gate6-sound-export` on 2026-10-16 at
`41fa5b19f9b52a70aa3101f5b69f9cdb8ea45c0a` (parents `0fe17691fa353cee9b37dc85d4ea4149a5aa8ec5`
and PR #45's final documentation head `ba1382b5a1f5cf329946a22602098bd2a96ae510`). PR #44 is
therefore no longer the export-only parent: it carries the whole Gate 6 implementation against
the Gate 5 baseline `7cbe8cbe` (192 files, about 19,000 added lines at `fde014ca`). Its runtime
code at the merge is byte-identical to the locally verified checkpoint `818e7116`; the merge and
`ba1382b5` change only documentation.

Two code corrections follow the merge. Neither is covered by the 2026-10-14 local verification
below, and neither has been built or tested locally; their compile, format, Clippy and test
evidence is the hosted CI run on the final PR #44 head.

| Correction | Commit and scope | Review status |
|---|---|---|
| Combined-review findings 1–4 | `fde014ca815478a885a0dfc4f16398c2c52a4e4a`: a resolved non-budget dispatch refusal stops archival playback without EOF; GIF `.part` staging/sync runs outside the UI-session guard and server registry, with only the rename fenced and reveal after it; take suppression keeps both the outgoing and the adopted deck CID; SFX/export id reuse after checkpoint compaction is specified and pinned as an accepted limitation ([GATE6-FRAME-SOUND](GATE6-FRAME-SOUND.md)) | Re-review at `fde014ca`: three resolved, one accepted limitation |
| Score loop keeps its authored grid | The commit after `fde014ca` (its SHA is in the PR #44 description): `ScorePlayback.gridMs` travels beside the unchanged jam-take:v1 export, and Studio completion is max(grid end, last release end), so trailing rests and empty bars loop. Regressions cover a first-step note then rests, trailing empty bars and a final release crossing the grid. This ledger update is in the same commit | Re-review pending |

Earlier evidence stays attached to its own SHA: the local PASS below is for `818e7116` (equal
code at `41fa5b19`), not for the combined head.

## Locally verified checkpoint — 2026-10-14, hosted acceptance pending

Gate 5 is owner-accepted and merged. Gate 6 implementation, cumulative review and required local
verification are complete. Gate closure still requires the hosted/Linux checks: queued jobs are
neither passes nor code failures. The reviewed integration code checkpoint is
`818e711641bc8193af5cde3be8e56d4817c475f9`; the shared baseline has not been merged or changed.
Gate 7 release/two-real-client acceptance is later.
Older dated checkpoint results keep their own scope.

### Exact stack

| Boundary | Exact commit / relationship |
|---|---|
| Owner-accepted Gate 5 / PR #43 baseline | `7cbe8cbe131e1be8e672d04341c31a1d2b52a98d` on `Create-suite-2` |
| [PR #44](https://github.com/Thalpy/Mewtual/pull/44) as originally reviewed, export foundations + nonce correction | base `7cbe8cbe131e1be8e672d04341c31a1d2b52a98d`; head at that review `0fe17691fa353cee9b37dc85d4ea4149a5aa8ec5` (historical; see the combined candidate above) |
| [PR #45](https://github.com/Thalpy/Mewtual/pull/45), stateful/native/UI integration | stacked base `0fe17691fa353cee9b37dc85d4ea4149a5aa8ec5`; verified code checkpoint `818e711641bc8193af5cde3be8e56d4817c475f9`; merged into PR #44 at `41fa5b19f9b52a70aa3101f5b69f9cdb8ea45c0a` on 2026-10-16 |

The SHA above names the code checkpoint. A subsequent documentation-only evidence commit does
not change that code; the PR description/handoff records its full final documentation head.
The earlier pre-SoundRecord-backend integration checkpoint was
`e83f12e414fccaaf11074cb02b4f384abc38bcd5`, including the reviewed Score loop-checkbox layout
correction. Its scope predates backend recovery; final combined evidence is recorded below.
The implementation-complete integration candidate is
`9fed4588d5f29862f18b38f7f44a14e506fefe4b`, normal-merging reviewed backend SoundRecord recovery
`1d8882875e7178f80140e70c102b8ab6e74ae973`. Its mechanical native-formatting successor is
`8632b7bc69ebc096e5e8a2f8eb19465d902a282c`, with unchanged root-Rust/TypeScript bytes. Full cumulative
code review is clear through that successor. The later normal merge at `818e7116` incorporates
reviewed CI-anchor correction `cbeb65bf7041a946adaa47987868cdd0d24b38b1` and leaves runtime
Rust/TypeScript, manifests and lockfiles byte-identical to `8632b7bc`. Final local checks below
pass on those sources. Hosted acceptance remains pending, not waived.

### Implemented requirements and bounded reviews

| Requirement / review boundary | Evidence pin and scope |
|---|---|
| Deterministic valid-take / PIXA codecs; owned input and nonce validation | PR #44 head above; nonce correction `863fe0fedd51cc28314c64562baa858370fe4130`; bounded design/implementation re-reviews clear |
| Typed Score causal state, reusable remove-wins slots, exact checkpoint/recovery and runtime/native family binding | core `78c79faad327775a3fc162c8437864086d7eb77b`; DTO/session correction `ce7d18e54b9c9ca488bc82ee2f21c44d798fb07a`; later runtime/native integration included in PR #45 |
| Complete Flipnote SFX/recipe/link/export evidence and conservative reference holds | `f4711f4cc63fd7af3e18c9769a04d9a6d4f75d70`; shared fallible native DTO preserves ordinary/provisional/draft/recovery sound evidence |
| Actual-byte PIXA publication/readback and local visual-only GIF | `b9f94aa1a320f9f49f9e4c94a4e92fc8619b05a6`; native validation, safe fixed-name save and custody/acknowledgement boundaries reviewed |
| Shared archival planner/capability | `429e51a42d58f329d197e1b1dc3bb96c8d18f4a3`; actual-diff re-review closed owner fairness, legacy fallback release-tail and property-key snapshot budget findings; live watchdog unchanged |
| Combined vault/Flow-S/drum-deadline corrections | `bc86a44c7bda36b98ecd8c2bd348491078aacf38`; exact held PIXA promotion/S3 recheck, complete Score trailer decoding and delayed drum expiry closed by independent re-review |
| Connected Score/Sound/Music/export UI; orphan cleanup, frame-ABA import fence and truthful Score Copy consent | `9330fc0827812aba997101362d1565c055d8d45e`; bounded actual-diff review clear, including independent 22 compiled UI and earlier 33 media/Score/audio/emoji tests |
| Existing local/shared `.jamtake` consumer uses the same archival capability | `b8969aa3fd77aa9f09b9b4fbfe668193c9e5253b`; long authored holds/seek/gain/EOF and pending-download suppression reviewed; independent 52 focused checks pass, no remaining B/H/M/L in this boundary |
| Native-shaped visual fixture | `5bfbcc9ad11aa4da61f5caeed112dcd35f28c306`; strict read-only fixture, real PIX frames, explicit unsupported-write refusal; bounded review clear |
| Explicit Flipnote SoundRecord recovery | frontend `cefe08256acb6ba1e4e70ebc0f159eaf7cfce50d`: ten new focused tests and independent 24-test review clear; backend `1d8882875e7178f80140e70c102b8ab6e74ae973`: ten focused + eighteen broader Restore + one native-choice tests, scoped strict Clippy/formatting and actual-diff/LOW re-review clear. See [GATE6-SOUND-RECOVERY](GATE6-SOUND-RECOVERY.md); full combined acceptance remains pending |
| CI-anchor compatibility after typed-object/probe changes | `cbeb65bf7041a946adaa47987868cdd0d24b38b1`, normal-merged at `818e7116`; three stale anchors corrected without changing assertions, filters, commands or source-preservation guards. All three affected mutants compile and fail at their intended H5/C3/C4 assertions; all three restored controls pass. Both mutated Rust files are restored byte-for-byte. Final merged-main preflight: 224/224 unique anchors across ten harnesses, zero stale/multiple matches. This is not execution of all 224 mutants |

Each verdict accepts its stated boundary, not the entire gate. Prior findings stay closed unless
new evidence reopens them; neither a small fix review nor ancestor inclusion accepts later code.
The final cumulative implementation/integration review through `818e7116` reports no remaining
BLOCKER/HIGH/MEDIUM/LOW and independently passes 47 selected transport/controller/audio/fixture/
sound-recovery/compiled-loop tests. This verdict covers combined code, not just a correction;
Its supplementary CI review independently verifies all three mutant failures/restored passes,
byte-exact restoration, normal ancestry and the final 224-anchor preflight. It does not substitute
for the primary's complete suites or unexecuted hosted checks.

### Verification ledger

The following execution results are reported by the integration owner at `818e7116`; this
docs-only change does not rerun or substitute for them. They do not cover `fde014ca` or later.

| Check | Checkpoint evidence / remaining obligation |
|---|---|
| `cargo test --all --all-features` | **PASS**, aggregate 2453 passed / 24 existing ignored (app:1116 passed / 22 ignored), including integration/doc tests. Executed after SoundRecord at `9fed4588`; root-workspace sources are identical through `818e7116`. Local log: shared-checkout `target/gate6-final-root-suite-v2.log` |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | **PASS**, 345 library + 5 ACL tests, binary/doc targets; formatted native source `8632b7bc`, unchanged through `818e7116`. Local log: `target/gate6-final-native-suite-v3.log` |
| `npm.cmd --prefix apps/desktop test` | **1555/1555 PASS** at `818e711641bc8193af5cde3be8e56d4817c475f9`; local log `target/gate6-final-frontend-suite-v4.log` |
| Frontend `check` / `build` | **PASS**, zero errors/warnings and production build; Vite's existing large-chunk advisory remains. No assertion or warning policy was weakened |
| `test:flows` | **PASS, all 9 scenarios**, host run on `818e7116`; local log `target/gate6-final-flows-host-v2.log`. Initial sandbox CDP timeout was an environment failure, not a pass |
| `test:startup` | **PASS** against the freshly built formatted native candidate: real app reaches `STARTUP.SETUP.COMPLETE`, no panic. Native `cargo check` and `cargo build` also pass; local startup log `target/gate6-final-startup.log` |
| Visual evidence | Actual 900×640 browser inspection of Score/Sound/linked Score states, no App console errors; PNGs kept under root `target`, not committed. CUA helper failed; isolated host headless Edge/CDP fallback succeeded. Reviewed `e83f12e4` Score loop label right edge is 862<900 and checkbox width 13px; root `target/gate6-studio-score.png` updated |
| Root and native formatting / strict Clippy | **PASS**: root `cargo fmt --all -- --check`, separate-workspace fmt, root `cargo clippy --all-targets --all-features -- -D warnings` and native equivalent. Local Clippy logs: `target/gate6-final-{root,native}-clippy.log` |
| `cargo deny check` | **PASS**: advisories, bans, licenses and sources; existing duplicate-version warnings remain. Initial sandbox advisory-lock refusal was rerun successfully with cache access |
| Ambient / raw-filesystem checks | Local Git-Bash `check-no-ambient.sh` and `check-store-raw-fs.sh` **PASS**; **Linux CI evidence PENDING**. Git-Bash is not Linux CI |
| Relevant mutation gates | Affected H5/C3/C4 mutations + restored controls **PASS**, guard tests 2/2 and merged 224-anchor static preflight pass. Other repair/integration mutation executions remain assigned to their hosted jobs and are **UNEXECUTED on the final head** locally; no all-mutants pass claimed |
| Hosted PR checks | Final-head Linux/Windows/desktop/repair/integration checks remain **PENDING** in the CI queue. Older active candidate heads consume capacity; no superseded run has been cancelled. Record exact final-head run URLs before gate closure |

### Boundaries that remain unchanged

- Gate 4 promoted P5 only for its reviewed bounded lifecycle and ticketed Unconfirmed Save.
  P5 is not globally false. Direct Closing Save and unfinished native repair commands remain
  unavailable; Unconfirmed local drafts remain non-transferable and non-authoritative.
- Score's own historical OBJECT edits do not gain automatic replay. Manual Restore adds only;
  Copy is explicit fingerprinted replacement/removal, with fresh per-item previews/saves.
- SoundRecord may explicitly select a retained historical alternative by operation id. Whole-
  version UI walks choose one effective selected value/removal per slot, not every alternative
  sequentially. Complete evidence retention is not branch preservation or automatic recovery
  of all alternatives. Only archive preservation supplies that preservation proof.
- Header-only Index Copy remains projection-level; this gate does not remove its accepted
  preservation limitation. Recovery/draft backups are not playable PIXA exports.
- Emoji sound support here is an explicit local 4KiB `emoji-sound:v1` import with authored length,
  not a shared verified emoji catalog or chat-click feature. Shared-deck PIXA media-kind/consensus
  and a global standalone viewer are Gate 7/UI follow-ups. Score `.jamtake` playback already
  reuses the existing deck's reviewed archival consumer.
- Audible native hardware validation and two real clients are not claimed by fake-engine tests,
  browser screenshots or this ledger. Final release/compatibility acceptance remains Gate 7.

Local implementation/verification checkpoint: **PASS** at code pin `818e7116`, with no open
review finding at that pin. The combined PR #44 head adds the post-merge corrections above, which
are unverified locally and await hosted CI and re-review of the Score loop correction. Final Gate 6
acceptance: **PENDING hosted/Linux CI** on that head, including the applicable repair/integration
mutation jobs. Do not call a queued check a pass or merge the shared baseline
on the strength of older-head results. Gate 7 remains outside this checkpoint.
