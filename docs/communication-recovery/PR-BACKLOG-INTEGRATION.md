# PR backlog integration

Updated 20 November 2026. The remaining integration backlog is merged into
`main` at `0685a6b6d38918b01334ce1ecf58a4509d8bb8a0`. GitHub's open-PR
inventory was empty after #73 merged. The user explicitly chose to merge the
remaining work with hosted checks pending and fix failures on main.
**Chat V2 remains disabled and incomplete.**

## Merged inventory

| PR | Scope | Main merge commit |
| --- | --- | --- |
| [#84](https://github.com/Thalpy/Mewtual/pull/84) | Foundation/security integration | `434232e2` |
| [#89](https://github.com/Thalpy/Mewtual/pull/89) | Consolidated Chat V2/recovery stack, including the native local-message UI and later ownership work | `f5aa1d38` |
| [#83](https://github.com/Thalpy/Mewtual/pull/83) | Studio background save and preservation of current edits | `a937bafe` |
| [#76](https://github.com/Thalpy/Mewtual/pull/76) | Score/voice stack and composed native/Root fixtures | `4dd7de83` |
| [#140](https://github.com/Thalpy/Mewtual/pull/140) | CI cancellation and Linux Cargo target/cache alignment | `7b4f570d` |
| [#73](https://github.com/Thalpy/Mewtual/pull/73) | Unscripted positive three-peer reproduction and bounded diagnostics | `0685a6b6` |

Merge commits preserve the original histories. #77 is included through #76.
[#97](https://github.com/Thalpy/Mewtual/pull/97) and
[#47](https://github.com/Thalpy/Mewtual/pull/47) were closed as superseded:
their complete heads (`81373e7e` and `cacc3763`) are ancestors of main through
#89 and #76 respectively. Their GitHub state is closed, not individually
merged; their code is included. Earlier consolidated PRs remain in that history.

#73's original `c581374b` was composed with current main at `3878c2ee`
before merging. The resulting main tree matches that composed head. The
original three-peer test body and assertions are preserved, along with both
the incoming bounded diagnostics and main's later diagnostic methods.
Its previous draft/merge hold is superseded by the user's explicit merge
instruction, not by a newly passing result.

## Validation and CI repairs

The first main run containing #73 is
[CI at `0685a6b6`](https://github.com/Thalpy/Mewtual/actions/runs/44744721743).
It was queued when this checkpoint was written. Subsequent documentation-only
commits retain this runtime source; inspect their current checks before
claiming a pass. Hosted work is still pending, and there is no current complete
Root/native/frontend acceptance receipt for the whole merged tree.

Earlier #76 checks at `1b1ed84f` continue to provide evidence for that named
source. Their production/test tree predates #73; they cannot validate its
new three-peer test or helpers. Windows runtime-mutation
[shard 1](https://github.com/Thalpy/Mewtual/actions/runs/44740490376/job/136302690148)
and [shard 3](https://github.com/Thalpy/Mewtual/actions/runs/44740490376/job/136302689516)
failed at the first Cargo command's 900-second bound. Their partial output
was lost before artifact creation, so the logs do not establish whether
compilation or execution consumed that time. The shared harness now writes
output directly to each artifact log while Cargo runs; timeout still fails
at 900 seconds and restoration/negative/positive assertions are unchanged.
The timeout's underlying cause remains open pending current hosted evidence.
Cancelled and superseded jobs are not passes.
No build or test ran on the current machine. The #73 composition received
installed rustfmt checks, JavaScript/TypeScript syntax checks, source
comparisons and `git diff --check` only.

Repairs included in the merged source:

- Cached observer source now uses a verified, owned temporary directory outside
  Cargo's target cache, avoiding missing ownership markers after restoration.
- The Chrono advisory was addressed through Jiff and the constrained vendored
  Netwatch dependency configuration. Scoped NoQ deprecation expectations retain
  upstream send/error behavior; strict warnings remain enforced elsewhere.
  Earlier hosted audit passes are source-specific, not a permanent safety claim.
- Five route-custody fixtures now prune after the actual last owner returns.
  Native IPC checks retain the exact decimal generation and both reply fields;
  delayed-Closed accounting retains the real Row and Session allocations.
- The automatic-repair receipt fixture stops and joins its host before actor
  shutdown. The captured earlier failure occurred during teardown after all
  original functional assertions passed. Current hosted execution must validate
  this correction; the assertions and ten-second bounds remain unchanged.
- CI cancels superseded runs in the same PR/main group. The Linux container,
  Cargo target environment and Rust cache now use the same target directory.
  Other workflows retain their own cancellation policy. This is a configuration
  repair, not a measured build-time improvement or a passing test result.

## Remaining implementation and acceptance

Use [the current completion overview](CHAT-FILES-VOICE-PLAN.md) for Chat V2's
implemented boundaries and dependency order. The next functional vertical is
covered invitation issuance and real MLS Add/join, followed by original-message
delivery and restart recovery between admitted members after the founder stops.
Shared projection/audit, scalable governing history, cold operational authoring,
Leave/rejoin and the multi-member/epoch/load matrix remain open.

The merged #73 positive three-peer case last failed in 60.67 seconds on its
original branch. Diagnose current-source results before assigning a cause.
Do not weaken exact peer-identity assertions, clear failed history or renew
old authority to hide the failure. Coordinated admission, authenticated
retry rounds and their capacity/lifecycle rules remain in
[the multi-peer requirements](VOICE-MULTIPEER-ADMISSION-NEXT.md).

The earlier composition review also retains one low coverage follow-up:
force a selected-before-Begin refusal through the observed helper to a real
parent in one deterministic test. Separate late-Index refusal and original
deadline tests do not establish that combined path.

Production voice audio/lifecycle, member media relay, combined load and
[physical acceptance](TWO-COMPUTER-ACCEPTANCE.md) remain separate gates.
Controlled RTC tests establish no audible WebView/ICE result. External-download
and dependency changes are included in the history; verify the exact current
binary and audits rather than describing them as separately owned pending PRs.
The one named 10,000-operation catch-up cost case remains explicitly deferred
until final Chat V2 acceptance.

## Historical evidence

The former backlog narrative is preserved in
[the pre-cleanup source snapshot](https://github.com/Thalpy/Mewtual/blob/0685a6b6d38918b01334ce1ecf58a4509d8bb8a0/docs/communication-recovery/PR-BACKLOG-INTEGRATION.md).
It records #74's full-suite receipts, native observer/clock corrections,
negative/restored mutation controls and the older integration holds. Those
holds describe their original checkpoints; they are not the current PR queue.
Detailed dated evidence remains in
[the communication ledger](CHAT-FILES-VOICE-ACCEPTANCE.md) and feature documents.
