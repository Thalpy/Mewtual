# Completed MLS Add ownership handoff

This is a library ownership boundary, not a production Chat V2 join. Chat V2
remains disabled. No Root, native command or transport consumer invokes this
handoff yet.

`PreparedTextCurrentAdd::prepare_install_into` borrows the actual retained
candidate slot and both exact original receiver halves exclusively. It checks
the completed outcome, original accounting pair, group/key/epoch/authenticator,
active roster, pending state and the new candidate's independent payment and
closed provider. All SDK/provider reads occur during this preparation, outside
the host's final writers. Errors leave all three inputs untouched.

The returned preparation keeps those exclusive loans until cancellation or
installation. Cancellation or scope end leaves the same candidate in its slot.
`install` only takes that candidate, swaps both graph halves, marks it installed
and returns an opaque retired owner. It performs no SDK call, allocation,
callback, hashing, IO or payload/fee disposal. The retired owner exposes no
operation, extraction or adoption method and must be cached before any late
check, then dropped outside host writers/native guards.

The new receiver pair retains the new candidate's full accounting lease in
each half. The retired owner retains the old graph and its original lease,
all output buffers, the closed candidate policy and the new operation's
payment. Destroying it releases the old graph without a parent-lease chain
in the new graph. The new provider remains permanently closed to unrestricted
SDK mutation. Another Add needs another independently paid private candidate.

The existing completed-result module and four tests were tracked but not
registered. This increment registers and verifies them, including their
separate inline metadata invoice. Their snapshot accessor returns secret
provider bytes: the future host must pay for copies and seal them through its
authenticated persistence path. Result views are historical data, not selected
Source, saved-governance or publication evidence.

Root still has to retain one exact request and selected parent, persist the
reservation before SDK work, retain the actual Add/Header outcome and cover
the complete governing save before installation or exposing Welcome. The final
Root admission must independently check its current Source and native session;
an exclusive SDK loan does not supply either. A closed Sync receiver will need
separate access to stable source/scope fields for those checks, without
reborrowing the SDK pair held by the preparation. A combined governance/native
constructor and versioned multi-member archive remain necessary.

Focused17 cases passed, including five new installation regressions and four
newly registered result regressions. The full MLS crate passed156 unit cases
and22 integration cases; two pre-existing ignored cases remain ignored. All
five final compile-fail doctests passed, including exclusive receiver borrowing
and `must_use` retirement custody. Final strict crate Clippy passed (3.06s),
native checking passed (1m47s), and all3,485 frontend tests passed (70.50s).
Formatting, ambient/raw-filesystem gates and both dependency audits passed.
Root strict Clippy still reports the same564 inherited App errors. Final
read-only adversarial re-review is clear after its low-severity `must_use`
guardrail was added. Exact-head complete Root/native CI is queued separately;
the named catch-up cost case remains deferred by the user. This evidence
establishes real SDK Add/Welcome and ownership behavior, not production peer
admission. Startup/browser gates were not repeated for this MLS-only increment;
the carried renderer correction's browser evidence is recorded separately.
