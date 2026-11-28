# V2 exact review decision body

The ReviewDecision Header kind has a semantic decision data format and a separate exact
held-post format. Governing selection, the owner transaction and native decisions still
require the production journal. Existing v1 action and owner fact encodings are unchanged.

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

### Exact held-post format

`ExactReviewDecisionBody` uses a separate `mewtual/chat/held-review-decision/v1\0` domain.
It encloses the canonical semantic decision, the digest of the complete signed original,
the selected Remove Header ID, and that Remove's MLS epoch. It allows Creates and replies.
The signed Header binds the wrapper's body ID. Two genuine signatures with the same
semantic ActionId name the same case but different exact originals. An exact successor
must refer to the previous wrapper ID and retain the original digest and Remove coordinates.

The matching helpers operate on a decoded hold record or rederived evidence. Approval needs
rederived evidence with verified reply ancestry. The signer must rederive under the actual
physical selection and require the current verifier to recognize the evidence. A decoded
record or matching wrapper supplies no freshness or positive admission.

### Founder review stream

A separate stream keeps review writes independent of MLS Add and Remove. The active
membership selector requires an epoch increment and a new MLS authenticator, and the joined
runtime includes its selected membership Header in ordinary message envelope scope. A review
decision needs neither a new MLS commit nor a change to that scope.

`ReviewStreamEntry` signs its versioned domain, a hash of the original pinned founder
genesis, the stream sequence and predecessor, the actual selected membership Header
and MLS epoch, and the exact decision body ID. The first entry has sequence one and
a zero predecessor. Later entries use their immediate predecessor's signed-data ID.
Every representation undergoes its own full-key signature check. Distinct valid entry
IDs at a stream position supply fork evidence even when the decision bodies are absent.
The original anchor and selected membership lineage come from the independently verified
vault or join pin.

The private journal record frames the versioned domain, the signed stream
entry, the exact decision and the retained original as four bounded,
length-prefixed fields. Its decoder checks the pinned signature, the
decision digest and the original's signature and removal tuple. A cold
writer must rebuild the unsigned entry from the independently selected
membership, journal predecessor and decision, compare it to the covered
Attempt bytes, and only then attach the retained Raw signature. A decoded
record still needs selected membership ancestry and a selected case head.
Rejected and reopened originals remain private even when a complete record
can be authenticated.

Production selection needs a separate critical journal with its own head, intent outcomes
and resource limits. It must cover the exact original, body and resulting head before it
publishes an approval. An observed founder fork pauses further review effects and retains
the signed evidence. Ordinary message admission and selected membership stay independent.
Only approved originals and the proof needed to check them are eligible for ordinary
delivery. Full rejected and reopened case details remain in the founder audit.

The ID11 storage declaration reserves a separate journal in a recovered
founder's ID10 catalog. It preserves the occupied descriptors and uses a new
keyed marker and a domain-specific declaration seed. Cold startup proves
the covered claim, exact original hold and the empty selected directory.
It can recover an admitted declaration, but ordinary founder startup does
not provision the journal before the decision writer is integrated. An
older ID10-only build refuses once ID11 is selected. The original
recovered founder can cover the first signature under the App's exact
Intent, Attempt and Raw files. It rechecks the original hold, current
membership and ordinary archive before the dedicated native key call.
The covered signature is private and has no selected case result.
Selection, later membership recovery, fork persistence and delivery
admission remain open.

### Covered founder audit

After the existing hold writer covers the exact body and its directory, it can
write a fixed authenticated receipt in a separate sibling directory. The receipt
binds the selected ID10 marker, channel key, opaque case label and exact retained
ciphertext. Older writers still use their original catalog and file names. An
unavailable receipt cannot stop the exact body from being held.

The founder audit reads at most eight cases per window. It checks the covering
receipt, exact body and authenticated frozen Remove, and keeps both original
files held until every candidate in the window has been checked again. It can
advance past unavailable cases without repairing them or reporting a complete
page. A legacy hold that later entered ordinary history still needs separate
confirmation to gain a receipt. Native cursor custody and final disclosure are
separate boundaries.

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

## Source evidence

Core regressions cover the semantic format, the separate original and Remove bindings, wrapper
predecessors, terminal approval, verified reply ancestry and the pinned stream signature and
sequence. Thirteen focused Windows Core tests passed. One uses two independently valid
signatures for a single reply and checks that the selected decision matches its original
only. The audited record proof returns the signed original after rechecking the selected
frozen removal and live verifier. The eventual UI still needs its original-source final
publication guard. End-to-end acceptance needs a covered journal, exact intent recovery,
selected approval admission and a native admin view.
