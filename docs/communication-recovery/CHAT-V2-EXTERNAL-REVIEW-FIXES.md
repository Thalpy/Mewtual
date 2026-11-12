# External reviews 84–88: correction ledger (12 November)

All corrections below are authored on `chat-v2-review-corrections`. They are not
merge approval or Chat V2 completion. Execution is routed to CI; no local Cargo
is run. The five PRs remain draft pending corrected-source validation and any
remaining strict/native failures.

| Review | Finding | Correction and regression |
|---|---|---|
| 84 | Original recovery reconstructs before its work counters | Use the existing nonmutating applied-hash observer and a bounded retained-original scan. Raw target hash, signature, comparison, retention and returned-copy work are charged before execution; phase state survives yields. Real overlapping-origin, pending-edit, orphan, exact-cap, late-signature and unrelated legacy/oversized-history regressions. |
| 84 | Audit attribution uses short prefixes/unverified names | Resolve the complete identity only after the row's signature verifies. Case rows attribute authors; decision rows attribute issuers. Compiled component tests cover collisions, positive controls and invalid/pending signatures. |
| 84 | Studio actor-restart stack abort | The later integration already contains the boxed two-phase driver and frame-size regression. Backport that existing test correction to the lower PR before claiming its own full gate; do not enlarge platform stacks. |
| 85 | Existing namespace skips parent durability debt | Complete the identity-checked parent-directory barrier on every successful open, including an existing directory, before returning a usable owner. Fault regression interrupts creation, refuses a pre-barrier retry, observes the actual barrier and saves/reopens. |
| 85 | Retirement ring forgets canceled tokens | Keep negative Abandon/ForgetLent tokens in a separate non-evicting paid table. Reserve a slot before fresh/adopted work. Positive Saved/SavedHit cache remains rolling and backed by mandatory exact durable lookup. Tests cross 64 real later saves for both negative modes and exercise the last reserved slot. |
| 85 | Full queue masks duplicate ActionId | Action lookup precedes pending pressure. Existing duplicate regression now runs at max_pending=1. |
| 86 | Owned seed handle prevents same-opener Windows retry | Cache successful unlink, close both seed handles before any later fault/barrier, then require strict name absence. Retained Current and governance openers retry cuts250/255 without RNG and preserve exact catalog bytes. |
| 86 | Capacity explanation omits recovery/append overlap | Document the supported 12MiB overlap within the current 17MiB allowance; future publication/checkpoint composition requires a new calculation. No cap increase. |
| 87 | Join facts accept the wrong local MLS own leaf | Require active group and actual local signer. Regression pairs genuine joiner device/routing with inviter's same-group/epoch/authenticator local instance and requires no facts/fence. |
| 87 | Stale join selection can replace newer consumed selection | Exact current tuple stays idempotent. Refuse older or same-epoch mismatching tuples before changing state/evidence. Real Add regression covers E→E+1→E retry→E+1. |
| 88 | Nonblocking combined ordering coverage | Add a cold-fork regression with a retained next candidate and an actual admitted signing reader. Candidate remains unconsumed, new signing refuses, contradiction remains and close waits for the original reader. |

The recovery correction's first static re-review found that unrelated same-key
legacy actors or oversized records could abort a valid target lookup. The fix
now skips oversized candidates and matches raw target identity before applying
target-specific actor/checksum rules. The same review identified 96 fixed hash
input bytes in verification; those are prepaid, and an exact-cap frame must
finish across bounded phases. The original public saved-proof decoder remains
unchanged. Optional vendor counters are enabled only by a dev-dependency to
prove that recovery performs no change reconstruction or value decoding.

Storage and join/retirement actual-diff re-reviews found no remaining blocker,
high or medium in their scopes. Final recovery re-review and CI execution are
pending. Static formatting, raw-filesystem and ambient gates pass locally.

Negative retirement is still lane-incarnation support: a restart cannot infer
cancellation from an absent outbox entry. The full save path reports exhausted
lane capacity through its existing Signing/Busy errors; neither means absence
or durable disk pressure. Normal saved traffic has no 64-message lifetime cap.
Production durable negative records, joined-source/native routing, governing
transport/archive/restart and activation remain separate implementation work.

Existing App strict-Clippy debt, dependency audit findings and native acceptance
failures remain merge blockers until resolved. Focused tests cannot waive them.
The unchanged slow catch-up cost case remains deferred until final acceptance.
