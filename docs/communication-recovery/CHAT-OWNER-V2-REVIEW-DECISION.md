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

Fresh intents cover the version 2 entry. It also signs a fixed 97-byte
case step with the case ID, a commitment to the exact original and
Remove, the preceding exact case decision ID and the choice. The same
step verifier can check Reject, Reopen and Approve across interleaved
cases using the selected stream tip and selected case tip. It can read
each choice, sequence and stable case ID without receiving the earlier
decision bodies or post text. A full approval record must match the
exact signed public entry and contain the original it approves.

Version 1 remains readable for local recovery, with the original bytes,
signature and semantic ID. A version 1 first entry cannot establish a
public stream, and a public suffix cannot skip it. These private journals
remain readable locally.

The connected review request uses an independent group-wide exporter domain.
It can read one signed header by sequence, or request the exact approved
entry and channel. Both modes carry a current signed contact proof bound to
the authenticated requester. The joiner uses the pinned original invite
founder and the same authenticated connection generation for its response.
A missing response or empty reply describes only that attempt.

The founder serves a settled journal. It uses the immutable intent root
selected by the current head to confirm the first and requested records,
and refuses reads during a pending selection. The first record must use
the public format. A header carries only its signed transition. An approval
also rechecks the signed original and exact frozen Remove. The original
reply dependencies were checked when selecting the approval. The receiver
checks both exact historical coordinates against its own real joined
prefix, including the steps before its original Join. It must still prove
ordinary reply dependencies and select the case durably before it can add
the original to its review archive. The joined desktop worker runs these
checks before it asks the current conversation to refresh.

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
the covered claim, exact original hold and authenticated empty head.
Recovered founder startup provisions the hold and decision namespaces in
order and reopens each selected catalog. Fresh native Found selects both
under the original warm runtime, retaining the exact Current file and
both original mounts through row selection. It reserves the entire
preparation before writing each declaration. Closing during an admitted
write still allows its exact storage transaction to finish. Fresh founder
moderation and serving still need the same-session authority path. An older ID10-only build refuses once ID11 is
selected. The original
recovered founder can cover the first signature under the App's exact
Intent, Attempt and Raw files. It rechecks the original hold, current
membership and ordinary archive before the dedicated native key call.
The exact covered signature can be recovered after a later membership
change by proving its original Header and epoch in the same selected
lineage. A fresh key attempt still requires the current membership
position.

The Rust-only first-decision path freezes the exact control digests,
encrypted record, intent and case index pages, and replacement head in an authenticated write-ahead
file. A separate scratch name precedes it. Once the file is covered,
the writer can finish only its exact record and head. It covers the
record and both index pages before replacing the head and rechecks the original custody
before each promotion. A retry after the rename proves and covers the
original outputs. The indexed outcome owns its exact record and can be
read after retirement of the signing files and write-ahead file. The
write-ahead file survives until every original input has retired. An
earlier valid unindexed transaction still recovers with its original
files. The missing empty head of an older bare declaration cannot
authorize a new attempt.

Later private decisions read the original nonce first. A genuine miss
checks the exact selected case revision and binds the encrypted head
into Intent before entering the key. The later write-ahead format seals
the record, replacement head and both bounded index edits. A pending
retry rederives those edits from the original roots. A committed retry
reopens the selected request and case paths and retires the original
signing controls and provably unreachable index pages before retiring
the write-ahead file. Case revisions remain visible in immutable records.
Fork persistence on peers, delivery admission and native admin controls
remain open.

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

### Recipient journal

The native joined source covers signed public headers in critical namespace
ID12. The first native Join can declare it after durable registration, before
publishing the original joined row. It reserves the replacement metadata
proofs and receiver mount before writing. The same parked node, signer and
registration remain live. A physical cut that the warm owner cannot adopt
requires reopening the original vault. Startup can also declare it before
the first same-vault rejoin declaration. The declaration checks the registered
row against the original joined pin, retains the native registry and shared
history files, and covers the authenticated empty selector before replacing
Current. A2 binds the complete encrypted catalog. A vault with an existing
ID9 keeps that catalog, while a future ID9 declaration preserves all ID12
descriptor bytes.
When a rejoin Stage or other rejoin evidence already exists, startup
defers the new receiver. The retired owner can reopen and select its
original rejoin Stage against the unchanged catalog.

The selector uses the original genesis pin and vault for each group's stable
identity. It keeps an exact ordinal chain and each case's latest covered
decision. A gap reports the next required sequence. An exact duplicate
preserves the existing selection. Two distinct valid headers at the same
sequence cover both originals and freeze that group's review lane. The
ordinary message archive continues independently.

Both the original native row and selected membership ancestry are checked
before admitting a new header. An admitted write-ahead file retains the
exact original pin and selector change and can finish after a restart or
membership replacement. Recovery verifies the signatures, predecessor,
immutable index outputs and exact old or new selector. This recovery can
cover header metadata alone. Message admission requires its own proof and
receipt. Missing selected files leave review unavailable.

Cold recovery requires the separate keyed admission tag. It preserves any
incomplete scratch and permits a fresh attempt in one fixed spare for
that scope. Two incomplete scratch files leave the lane held. A previously
authenticated selector can be restored from outside the process if its
matching old files survive.

The selector charges retained files against 128 MiB per group and 512 MiB
per vault, with room reserved to record one contradiction in each lane.
The original header, fork evidence, receipt and index/head frames remain
encrypted locally. This implementation retains older index frames and
charges for them.

An approved original can be selected after its exact public header
appears in the ordinal and case indexes and the original joined source
rechecks the decision anchor, frozen Admission, Remove and local reply
ancestry. A missing parent returns the exact ActionId required for retry.
On the founder, the same ordinary history lookup can open the existing
outbox after restart to verify an original parent that the founder signed.
The lookup does not create an outbox. It applies the ordinary founder,
channel and group checks and replays the exact signed parent in the
selected membership prefix.
The receipt covers the complete signed record in a separate encrypted
file. Its action index is selected by a versioned lane Head that preserves
the original public stream position and both header roots. Recovery can
finish a previously admitted write. The receipt does not select or replace
an ordinary message. An exact joined history read can use it after a
verified ordinary miss. The detached display preserves the original
signature and rechecks ordinary reply ancestry. The read checks both
selectors again before publishing and cannot repair a pending receipt.
The joined desktop polls signed headers over an already established
original founder connection. It reads the durable ordinal from ID12, gives
ordinary inventory a turn between review polls when needed, and checks
the exact current membership card on every request. An empty response
ends that attempt without advancing the journal. A cold A1 vault needs
its original founder route restored before polling begins. A newly registered
A1 can retain the signed zero-address reservation in an independent optional
encrypted file and recover only its pinned founder PeerId after restart.
This does not retain a host/IP, establish a connection or authorize dialing;
polling still requires the original founder's current authenticated TCP lease.
Missing or damaged optional data supplies no route and preserves both original
Join registration and ordinary startup. Optional failure never suppresses
the independent Pending/Welcome or caller-current checks. Complete final and
Stage together can supply the inert hint only when byte-identical and fully
authenticated; cold reading modifies neither. ID9/A2 does not inherit the A1
hint. The joined
worker scans up to four selected header ordinals per turn and requests
an exact Approve for its attached channel. A missing body or ancestry
stays eligible on later passes. The worker refreshes an active view
from its guarded receipt. Normal history reads the selected receipt index
in pages of at most two scanned entries, and can rediscover these originals
after navigation or restart. The encrypted continuation binds the original
row, channel and selected approval root. A changed root requires a fresh
review page. Each body still needs a separate exact guarded read whose
ordinary result takes precedence.

## Exact case transitions

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
