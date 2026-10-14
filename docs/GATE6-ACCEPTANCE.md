# Gate 6 acceptance and evidence ledger

## Current checkpoint — 2026-10-14, NOT final acceptance

Gate 5 is owner-accepted and merged. Gate 6 has an implemented integration candidate, not a
passing final gate. Explicit backend SoundRecord recovery is implemented and bounded-reviewed;
fresh final backend/native suites, hosted checks and startup evidence remain pending. Cumulative
code review is clear through `8632b7bc`; Gate 7 release/two-real-client acceptance is later.
Older dated checkpoint results keep their own scope.

### Exact stack

| Boundary | Exact commit / relationship |
|---|---|
| Owner-accepted Gate 5 / PR #43 baseline | `7cbe8cbe131e1be8e672d04341c31a1d2b52a98d` on `Create-suite-2` |
| [PR #44](https://github.com/Thalpy/Mewtual/pull/44), export foundations + nonce correction | base `7cbe8cbe131e1be8e672d04341c31a1d2b52a98d`; head `0fe17691fa353cee9b37dc85d4ea4149a5aa8ec5` |
| [PR #45](https://github.com/Thalpy/Mewtual/pull/45), stateful/native/UI integration | stacked base `0fe17691fa353cee9b37dc85d4ea4149a5aa8ec5`; checkpoint head `cefe08256acb6ba1e4e70ebc0f159eaf7cfce50d` |

The PR #45 head above is a checkpoint, not the eventual acceptance SHA. The integration owner
must replace/add the final exact code SHA and matching run results after the remaining changes.
The subsequent pre-SoundRecord-backend integration checkpoint is
`e83f12e414fccaaf11074cb02b4f384abc38bcd5`, including the reviewed Score loop-checkbox layout
correction. It does not replace the pending final backend/review evidence.
The implementation-complete integration candidate is
`9fed4588d5f29862f18b38f7f44a14e506fefe4b`, normal-merging reviewed backend SoundRecord recovery
`1d8882875e7178f80140e70c102b8ab6e74ae973`. Its mechanical native-formatting successor is
`8632b7bc69ebc096e5e8a2f8eb19465d902a282c`, with unchanged root-Rust/TypeScript bytes. Full cumulative
code review is clear through that successor; final backend/native runs and hosted/startup evidence
remain pending. Neither candidate is yet a passing acceptance SHA.

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

Each verdict accepts its stated boundary, not the entire gate. Prior findings stay closed unless
new evidence reopens them; neither a small fix review nor ancestor inclusion accepts later code.
The final cumulative implementation/integration review through `8632b7bc` reports no remaining
BLOCKER/HIGH/MEDIUM/LOW and independently passes 47 selected transport/controller/audio/fixture/
sound-recovery/compiled-loop tests. This verdict covers combined code, not just a correction;
it does not substitute for complete final suites, startup or hosted checks.

### Verification ledger

The following execution results are reported by the integration owner; this docs-only change
does not rerun or substitute for them.

| Check | Checkpoint evidence / remaining obligation |
|---|---|
| `cargo test --all --all-features` | **PASS**, aggregate 2443 passed / 24 existing ignored (app:1106 passed / 22 ignored) at `f1151601f3d838ec861b9b0da49db00df5c37601`, before the SoundRecord correction; final post-correction run **PENDING** |
| Separate native full suite | 345 library + 5 ACL tests passed at `f1151601f3d838ec861b9b0da49db00df5c37601`, before the SoundRecord correction; final post-correction suite **PENDING** |
| Complete frontend suite | **1555/1555 PASS** at `9fed4588d5f29862f18b38f7f44a14e506fefe4b`; successor `8632b7bc` leaves TypeScript unchanged |
| Frontend `check` / `build` | **PASS**, zero errors/warnings / production build 29.07s at `e83f12e414fccaaf11074cb02b4f384abc38bcd5`; Vite large-chunk advisory remains |
| `test:flows` | Sandbox CDP attempt timed out; host rerun **PASS, all 9 scenarios** at `cefe08256acb6ba1e4e70ebc0f159eaf7cfce50d`; subsequent App change through `e83f12e4` is CSS only, not a newly claimed flow run |
| `test:startup` | **PENDING** for the final built native candidate; unit tests do not construct a Tauri app |
| Visual evidence | Actual 900×640 browser inspection of Score/Sound/linked Score states, no App console errors; PNGs kept under root `target`, not committed. CUA helper failed; isolated host headless Edge/CDP fallback succeeded. Reviewed `e83f12e4` Score loop label right edge is 862<900 and checkbox width 13px; root `target/gate6-studio-score.png` updated |
| Rust formatting, strict Clippy, dependency audit and relevant mutation gates | Root/native formatting **PASS** through `8632b7bc`; scoped backend/native strict Clippy passes at the SoundRecord pin; `cargo deny` **PASS** (existing duplicate warnings, initial sandbox advisory-lock failure followed by host success). Final complete Clippy/mutation/hosted results **PENDING** here |
| Ambient dependency check | Local Git-Bash check passes; **Linux CI evidence PENDING**. A local result is not a Linux CI pass |
| Hosted PR checks | `e83f12e414fccaaf11074cb02b4f384abc38bcd5` jobs queued with no steps executed / **PENDING**, neither a pass nor a code failure. No CI cancellation performed; record exact final head and run URLs before gate closure |

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

Final acceptance entry: **PENDING** — integration owner to record exact final code head and
complete required suites/checks/startup/CI evidence alongside the clear cumulative verdict.
