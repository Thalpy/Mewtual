# PR backlog integration — 27 October 2026

[PR #74](https://github.com/Thalpy/Mewtual/pull/74) is a draft integration candidate,
not a completed communication programme. Runtime source at the initial combined checkpoint:
`2babf97e3162de2bb4212ad044c76c0c6e15f8d3`. Current runtime/configuration checkpoint:
`c4aa9064c078440652e69ddb7aaac4346d2f041c`, after the current connectivity extraction
and native dependency fixes. Earlier results do not validate the changed native graph.

## Landed and prepared work

- PR #26 merged into main at `701da398`. It included all PR #54 commits, and GitHub
  marked #54 merged. The merge rehearsal differed from the tested Studio source only
  in the pending-action document and toolchain comments. Its historical green audit
  predates newer advisories; it does not establish present dependency safety.
- PR #57 merged into `chat-update-implementation` at `0cf203f2`, after the documented
  #26/#54 trigger was satisfied. Its original head `bace1bf3` had 28 passing hosted checks.
- The candidate preserves the original upload repairs `c4005b89` and `985c1f1d` together.
  Main's accepted older foundation contains the upload/lock race until this repair lands.
  The independent actor-event fence prevents a bounded event-sink/publication deadlock.
- The candidate includes the reviewed recovery/V2 stack through #72, both newer Gate7
  changes, and the scoped MLS admission/commit-log fixes #60/#62. Original history is
  preserved. The known-failing three-peer test from #73 is excluded; its reviewed design
  requirements are retained in [the admission handover](VOICE-MULTIPEER-ADMISSION-NEXT.md).
- PR #15 is also included: current connectivity formatting moves into its dedicated
  module, preserving trace correlation, ordering, privacy copy and all compatibility exports.
  Its 15 focused tests and read-only integration review pass.

The send merge preserves both branches' contracts. A native capacity/document-pending
refusal proves this attempt unaccepted after its pending entry is resolved. It cannot
erase possible acceptance of an earlier attempt with the same identity. Tests exercise
both facts through the actual App functions and kept/recovered/cancelled entries.

## Current combined evidence

| Check | Result at the initial combined source |
| --- | --- |
| Actual-App send/retry/recovery focused tests | 28 passed, including the combined ambiguity/refusal cases |
| Complete frontend suite | 3,201 passed; 74.29 seconds |
| Svelte check / production build | Passed; Vite build 37.66 seconds |
| Rendered Edge flows | All 10 scenarios passed with host browser access; the initial sandbox CDP failure ran no scenarios |
| Focused MLS crate | 67 passed; 1 existing ignored subprocess helper |
| Root / native formatting | Both passed |
| Root dependency audit | Passed against the current advisory database |
| Separate native dependency audit | Initially failed; corrected at the native follow-up below |
| Fresh complete Root / native suites, strict Clippy, native check/build/startup, ambient gate | Pending; older checkpoint results do not validate this combination |

On the final frontend inputs, the repeated full suite passes 3,201 tests in 64.90 seconds;
check, build and all ten rendered flows pass. Both formatting checks pass. The new native
dependency graph and policy pass both audits, including all four categories. Fourteen
upload/session regressions passed before the native dependency follow-up. The WMI-only
Windows check passes after its ABI correction; the complete fresh native pipeline is
running serially. Root Rust/Cargo/toolchain inputs are byte-identical to the initial
combined source being tested by hosted Windows/Linux jobs. Runtime manifests record
1,477 file hashes; documentation updates do not change those inputs.

Read-only reviews of the actual merge and neighboring enforcement found no new blocker,
high or medium in the scoped send/upload, MLS/dependency and native custody boundaries.
The stale commit-refusal comment was corrected. These reviews are not a fresh audit of
every inherited file. Full gates and native layout guards still need their new results.

The separate completed `05c97adb` checkpoint passed 897 native library plus 5 ACL tests,
3,691 Root tests with 24 existing ignores, and 3,200 frontend tests. Its native run used a
forced rebuild and checked test inventory; its Root/native input files were byte-verified.
Those results establish the older Rust/dependency checkpoint only.

## Native audit remediation

The initial native audit independently found `h2` 0.4.15 (RUSTSEC-2026-0258; patched in 0.4.16),
`zip` 4.6.1 through `tauri-plugin-updater` (RUSTSEC-2026-0367/-0368/-0369), and six
unmaintained proc-macro-error/UNIC advisories. The follow-up at `c4aa9064` updates H2,
removes unused ZIP extraction, updates the Tauri utility/parser graph to remove UNIC,
and replaces a newly yanked native arithmetic release. Both audits now pass. One new
build-only maintenance decision has exact stable-version/parent fences and tested
positive/negative controls. The actual native authority/parser regressions and full
gates still need execution on this changed graph. See
[NATIVE-DEPENDENCY-AUDIT](NATIVE-DEPENDENCY-AUDIT.md) for the source analysis and limits.

The upgrade's existing crypto-bigint exception remains the reviewed, constrained policy:
native secret P-256 uses 0.14; the old MLS arithmetic is fenced behind the single
X25519/Ed25519 suite and dependency bans. This is not removal of every old crate.
New OpenMLS 0.9-created/joined group records do not safely downgrade to older builds.
Use isolated validation profiles; the compatibility limits are in
[the upgrade design](../design-crypto-bigint-upgrade.md).

## Held PRs and remaining implementation

- #51: current native six-client convergence failure; its old all-green description is
  superseded by the current-head merge hold.
- #55: HIGH stale cache-limit command across lock/account/server replacement or leave.
- #56: inherited #55 HIGH and MEDIUM saved-metadata mutations after their generation
  guard is dropped. Both PR descriptions record the required barrier tests and fixes.
- #47: independent Score work with unresolved integration conflicts.
- #73: deliberately failing automatic three-peer reproduction. Do not merge its test
  as a green checkpoint or weaken the original deadlines to hide the failure.

App V2 remains OFF. The Source already enforces the bounded audio SDP shape before
HIGH's PC constructor and checks local descriptions. Runtime receiver/direction checks,
paid audio cells, capture/playback wiring, App lifecycle/renewal, automatic multi-peer
admission, member media relay and combined/physical acceptance remain unfinished.
Only one physical desktop is available. Native Current is not a separate server-enforced
speaking/listening/moderator-mute grant.
