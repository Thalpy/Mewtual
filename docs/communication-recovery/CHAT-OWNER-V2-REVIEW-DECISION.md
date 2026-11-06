# V2 exact review decision body

Concrete implemented prerequisite with clear design and actual-diff reviews. This closes the fourth supported header body's data
grammar, not the actual governing verifier, UI decision journal, owner transaction or live command.
Compilation/tests/builds remain paused. Existing v1 action/owner facts are unchanged.

## Case and body grammar

A ReviewCaseId is BLAKE3(raw `mewtual/chat/review-case/v2\0` || exact canonical v2 AcceptanceRecord
frame). The full target binding is group/Profile/Admission/channel/ActionID/full subject/Grant/Basis/
Create as specified by that record. Case IDs are distinct typed labels from Action IDs and body IDs.
The unsigned record is target DATA here, not a claim that its action was accepted. Action matching
still needs real signature and the later historical ordinary/dependency verifier.

Body framing is catcoms-wire: bytes `mewtual/chat/review-decision/v2\0`; ONE byte field containing
that canonical target record (at most567); intent nonce bytes16; decision u8 (1 Approve,2 Reject,
3 Reopen); previous-decision presence u8(0/1), followed by previous owner-body CID32 only when present.
All byte fields are u32-length-prefixed. Extra/short/unknown fields refuse; Reopen without a previous
decision refuses. Maximum body665bytes. Body ID uses the shared owner-body-id/v2 domain.
The case ID excludes decision/nonce/previous fields so the same exact action has a stable case;
the decision body ID commits them all, so a changed intent cannot borrow an exact saved result.

An explicit selected Review Approve supplies a separate EXACT REVIEWED admission route after a
removal. It does not append to/change the frozen accepted-log cut or prove accepted-before-removal.
The Profile's inclusion rule authenticates LOG acceptance; its closed v2 review-body grammar
governs this additional reviewed permission. Raw target records, Reject and Reopen are never
positive admission. The active governing verifier, not body binding alone, must issue that token.

Decode first borrows/checks ALL outer fields, option semantics, lengths and trailing bytes. Only
then decode the bounded target record, whose own framing precedes its sole group copy. No full
action body is duplicated inside a decision; it remains in the retained action archive.

## Successor decision rules

The pure new-transition checker requires the exact same case and previous body CID:

- Initial None -> Approve or Reject.
- Reject -> Reopen only.
- Reopen -> Approve or Reject only.
- Approve has no decision successor. A later authorized Delete is a distinct chat action.

An exact retry looks up its retained ORIGINAL result before attempting a new transition; this
checker is not an idempotency cache. Missing/wrong previous evidence stays incomplete/refused.
The actual governing consumer must prove that the supplied previous decision is the selected
CURRENT case head. An older Reject cannot reopen an already Approved case. A known finalized
Acceptance likewise cannot be retracted by an initial Reject just because this codec sees no prior
review decision. These checks require the selected state and are not claimed by the data helper.

A header-binding helper verifies actual header signature, exact ReviewDecision kind/group/Profile/
bodyCID. It does not choose owner/tenure/root, validate current state or mint ordinary authority,
projection admission or exact cross-author Delete approval. Missing/invalid bodies still cannot
erase a separately authenticated pair of contradictory successor headers.

## Actual producer and retained custody remain required

Only the original finalizer under the mounted v2 Profile can issue a final decision after verifying
the exact retained action, ordinary historical rights/dependencies and the selected held case.
Current admins may recommend; a current-role lookup alone is not the finalizer's signing capability.
Approval of an Edit/Delete never admits a held Create. Rejection cannot retroactively retract genuine
finalized approval. Every original action, recommendation and decision remains in the admin ledger.

The native request captures original UI/registry/owner scope and exact case revision before awaits.
It retains one stable intent nonce and original request/outcome through covering save, reply loss,
lock/restart and exact retry; no old scope adopts a replacement. A separately sealed critical review
journal (or new supported critical UI envelope) must preserve positive and negative abandon custody;
existing UI continuityv3 and the closed chat Stop grammar cannot carry it in ignored fields.

Reserve exact decision/header, terminal/receipt, negative abandon, cold pages, replacement and
mandatory control sequence capacity BEFORE dispatch/signing. The665-byte body cap is not that
physical/runtime invoice. Actual save-before-install/publication, archive/index/state validation,
opaque projection/approval issuance, admin workflow and crash/compatibility tests remain open.

Required authored tests: exact maximum and independent frame/CID/case identity; real action/header
binding and signature variants; every valid/invalid decision pair; wrong/foreign/stale case and
previous IDs; nonce/decision/previous changes alter body ID but not case; short/extra/wrong option/
fixed-field/count/legacy frames; exact retry distinction; missing governing/current-head authority
is not fabricated by a successful data check. No execution is claimed during the pause.

## Source evidence

`owner_bodies::ReviewDecisionBody` now implements strict outer/target framing, case/body identities,
exact new-transition checks and action/header bindings. Nine regressions are authored/unrun, using
real action/header signatures and the existing genuine action-signature variants. Source formatting
and whitespace checks pass. The governor, selected-current case lookup, covering producer, durable
intent/abandon journal, opaque approval/admission issuance and admin UI remain unimplemented.
