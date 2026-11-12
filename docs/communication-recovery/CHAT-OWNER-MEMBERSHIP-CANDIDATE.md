# Private membership candidate and covering response

Implementation supplement for the first real owner->fresh-KP member vertical. Design review is
required before this producer is enabled. CHECKPOINT-1 remains nonactivatable: scalable v2 public
headers/cold archive, signed root-distribution invite, native registration and full gates are separate
mandatory successors. Do not change legacy JOIN/action signing bytes or route an owner-profile group
through legacy admit_now while claiming it has a covering barrier.

## Smallest executable path

The actual ready OwnedOwnerServer receives a bounded invite/KP request, authenticates the exact
profile/owner/source and current invite policy, and prepares an isolated MLS+owner-data candidate.
It covers that candidate through original Current -> canonical head -> original Store, installs only
on a successful file+parent barrier, then exposes the exact Welcome/routing/signature and original
signed facts. The member's already captured immutable root and actual selected-KP completion verify
those facts, then the same canonical initializer saves the member. No success boolean is a witness.
A staging network host can retain the existing exact JOIN_READY bytes and transfer original public
facts through its explicit trusted checkpoint handoff; that is not the production signed v2 invite
or an automatic first-contact root acquisition adapter.

The first operation refuses existing unsupported state before allocation: unresolved MLS contests,
replacement uncertainty, pending alternate membership operations, unsupported owner succession, and
unbounded/unsupported legacy inventories. It never drops those records to fit. No source/role/key is
selected from a numeric alias. Ordinary chat/causal conflicts still merge automatically; these guards
serialize the one MLS writer, not an ordinary message conflict-repair UI.

## Candidate object and no premature observers

PreparedOwnerMembership is non-Clone/nonserialized and owns an isolated MlsDevice+ServerGroup produced
by the bounded SAME-guard MLS snapshot/restore path. It owns the complete candidate tenure, invite
ledger, retained commits, original public evidence, exact response cache and new publication debts.
It has no transport, subscribe/run method, public pre-save byte getter or callback to a publisher.
Creating a second ChannelSync with a live transport or swapping the entire actor is unnecessary and
would risk unrelated runtime/voice/registry owners; do neither.

Prepare pure durable data directly. Do not invoke with_observed_mls_transition while preparing a
private copy: that helper also reconciles live lower authorities. Capture OwnerTenure::Position and
apply the tenure transition only to the private state, using the actual add_member_observed/remove
observation. Prepare routing/policy endorsements and signed commit records into private fields.
All SDK/provider and node-RNG work uses the existing crypto/RNG/Clock seams. The candidate NEVER
clones or replaces the node RNG. After original source/response-capacity admission, preparation
performs the routing AEAD seal ONCE synchronously using the ORIGINAL node RNG and retains the exact
encoded sealed routing bytes. This direct Add has no other node-RNG draw: commit/Welcome signatures
are deterministic; SDK provider entropy follows its existing sanctioned MLS implementation. Other
live RNG consumption can continue and is never overwritten at install. A failed operation may consume
its nonce, which is safe; while retaining the same private attempt or after possible replacement,
retry must not reseal, resample or produce a different Add/response.

## Full field invoice and snapshot view

Use a read-only snapshot view/override encoder over latest live history plus private candidate data,
not temporary live-field mutation. The view must inventory every field before its first allocation:

| Field | Candidate/retention rule |
| --- | --- |
| MLS provider/device/group/authenticator/epoch | Isolated actual operation; no union of distinct MLS heads. Frozen original MLS bytes reused during repair. |
| Owner tenure/archive | Clone by bounded exact codec against original group, apply pure actual before/after observation. No live authority reconciliation until install. |
| Invite consumed/revoked ledger | Bound count AND hash-set scan capacity before copy. Check exact invite before mutation; consume in candidate only. |
| Commit log and new signed record | Preserve existing retained records, append exact commit, obey existing hot catch-up policy without deleting public owner history. |
| Past document keys | Strict pre-advance capture must succeed for every document in the FINAL latest-live view before Add/Remove install; no warn-and-continue. A document opened/received while preparation waits gets its old-epoch key from the still-live original group. Persist the retained bounded key window in a new strict owner-operation tail, since the legacy snapshot does not encode past_keys. |
| Routing label/secrets/topics/subscriptions | Add preserves existing label. Remove derives new secret in candidate; defer all subscribe/eviction/network effects until save. |
| Stable wrap key and pinned group policy | Preserve exact stable key; privately retain current endorsement bytes used by routing/Welcome. |
| Admin/policy generation and exact full-key roster | Preserve or apply exact governing removal change in candidate; no mutable display modlog authority. |
| Peer descriptors/pruning/finalization retry pairs | Preserve latest, prune only in candidate after relevant removal, retain exact pending acknowledgements. |
| Direct/admin/companion admission caches and invite results | First direct-only implementation refuses populated admin/device/companion pending/result inventories BEFORE preparation; legacy snapshot omits some of them. Persist the new direct exact nonce+KP/request-binding result and signed public bundle BEFORE reply in the strict owner-operation tail. Never silently discard or evict an outstanding result. Later adapters require their own explicit retained codecs. |
| Ordinary/managed outboxes and retained receipts | Keep original bytes/observers and signed history. New commit/response debt remains private until barrier. |
| All CRDT/typed original actions, branches and user terminals | Snapshot latest live union; never replace it with preparation-time history. New typed archive uses immutable cold pages. |
| Local scope aliases/reservations and owner runtime | Saved metadata retains exact incarnation; process-local source/runtime owners stay original, not copied from serialized labels. |
| Public owner facts/acceptance/cuts and accounting | Actual opaque observation -> exact signed facts -> retained source. Mandatory controls prepaid before ordinary work; v2 invoice is a separate open codec gate. |
| Dirty/channel/admin/proof revisions | Successful install emits bounded affected-channel/admin revisions; failed save emits no false successful effects. |

The existing generic encoder's framing must remain unchanged for legacy snapshots. Candidate view
fields are explicit references; self-owned runtime/transport/voice tasks are never replaced. The
actual-diff review must compare this table to the implemented view and install list, not assume a
provider-only clone covers the transaction. The strict operation tail is a new critical snapshot
extension after the existing owner tail; old readers reject it. It contains bounded retained old
keys and exact operation/result/publication obligations, with count, aggregate bytes and collection
scan fees checked before cloning/encoding. No decoded runtime/verified flag is authority. Its exact
codec and measured physical invoice receive review before persistence edits. Direct-only refusal is
not a claim that the legacy cache codec covers admin/device/companion admission.

## Serializer, reply loss and automatic repair

Reserve persistent current/replacement, hot buffers, operation rows and required negative/control
capacity BEFORE expensive candidate/signing work. Capture original mount, admission owner/writer,
public profile/head and MLS authenticator. Final Current/head custody rechecks all of them before
synchronous I/O; no source captured after await may adopt a replacement owner.

The actual canonical owner writer owns a pending-governance guard from prepare until install or
conservative debt retirement. ALL writes to overridden governing state must pass that same guard:
MLS commits/removals, admin roster+roster_gen, invite consumed/revoked ledger, group-policy/routing
endorsements and owner tenure. While pending/uncertain they queue bounded original intents or refuse
without mutation; they cannot change live state behind the candidate. Message receive/CRDT union and
non-governing RNG work may continue. Final CAS checks the original governing revision and ALL
captured governing fields as a defensive invariant. Every successful later governing command must
advance that same canonical revision before effect. Synchronous negative admission/mount retirement
always preempts immediately and makes final candidate I/O/export refuse; it never waits for the
candidate guard. No role demotion or invite revocation can be overwritten by installing a prepared
roster/ledger clone. The guard and pending debt are shared across all numeric aliases.

The first executable implementation SERIALIZES other canonical saves while this candidate is
pending/uncertain; it does not require a traffic-sensitive original Meta CAS to rebase forever.
Received immutable graph union may accumulate only under existing bounded receive limits and is
included by the candidate's final view/repair. New local durable Send/mutation acknowledgements and
background history snapshots wait for that covering save, rather than advancing an unrelated head
behind it. Automatic host retry/repair owns progress without a user repair choice. A later design
may allow original-writer/governing-identical rebases, but cannot silently relax this checkpoint CAS.

NotReplaced keeps the entire private attempt for retry. ReplacedNotDurable conservatively latches
that WHOLE exact candidate and fences every numeric alias/new membership advance. No older generic
snapshot may overwrite it. Repair may merge newly received immutable message/history union while
preserving the candidate's exact MLS state, original owner outcome/response and user terminals; it
cannot mint a different Add/Remove or infer installed==durable. Source receive work must remain
bounded and pending projection/controls honest while repair is owed.

Only after covering save and install can a DurableOwnerAdmission admit a response through its
consuming `respond(responder)` boundary. There is NO public reusable raw-byte getter. The opaque
object retains exact original mount/admission/writer/head/request identity and charged response
buffers. Pending work follows Responder::cancellation_authority plus original native owners; actual
reply transfer uses Responder::try_respond_with_custody with the original native writer/admission
conjunction as effect authority (NOT the reply-wait lifetime, which ends before send). The lower
driver repeats that original authority at actual response admission and retains byte/service fees.
Retirement/replacement before that edge refuses; already admitted network bytes cannot be recalled.
A trusted in-process CHECKPOINT member finisher may receive original facts only within an equivalent
same-original-head consuming callback, never retain an unscoped pre-save/cached result getter. Exact
nonce+full KP/request-binding lookup precedes new invite/role checks, so lost reply returns original
response without adding another leaf or consuming the invite again. Foreign binding on the same
nonce refuses without mutation. Current policy is rechecked for any genuinely uncommitted request.
Retaining owner response bytes alone does not recover a crashed joiner that lost its original
private KP/init material; production join attempts need their own durable exact private custody.

Crash custody is explicit: before the FIRST canonical replacement, this direct checkpoint exports
nothing and changes no source MLS/ledger; process loss may safely discard that unexposed private
candidate/fixed stage and retry the still-unspent exact request against the original durable head.
The in-memory seed/Welcome is not claimed to survive. Once replacement may exist, the complete strict
canonical snapshot is the recovery source; restart validates it and crosses actual file+parent/root
barriers before response/install. No old snapshot or cleanup may overwrite/erase that candidate.
Production join-attempt and host admission journals must separately preserve exact input/negative
custody and bound automatic restart work; this paragraph is not a durable operation journal claim.

Rejoin uses actual A2 fresh provenance PLUS old canonical head/archive custody. Never replace an
A1 canonical file with A2's empty history. Preserve old signed originals, cuts, receipts/Stops and
references; old observers stay revoked. Missing unexposed initialization after a crash needs bounded
native negative retirement or an exact retained journal, not manual user reconstruction.

## Direct CHECKPOINT operation-tail codec proposal

This measured first producer codec is NOT the scalable v2 production archive. It makes the actual
Add/save/Welcome path executable under finite limits while that successor is built; no native/typed
activation may use these lifetime cache limits. Review this grammar before writing it.

Append one length-framed strict tail after initial owner evidence, tagged
`catcoms/owner-direct-operations/v1`. It is present only for an owner-evidence group and requires the
original owner tail/profile. Unknown/trailing fields refuse. Encoded grammar: tag, profile32,
`past_count:u32`, sorted unique `(DocType:u16,doc_id:u128,epoch:u64,key:bytes32)` rows,
`result_count:u32`, then nonce-sorted direct operation records. Every variable bytes field uses
existing put_bytes/u32 framing; fixed integer widths are big endian. Encoded upper bound is checked
with arithmetic before every allocation, not after cloning a complete snapshot.

A direct result record holds exact nonce16, canonical request-binding32, KP-frame-hash32, actual
KP-reference32, full subject32, original InviteToken frame (<=8KiB), original KP frame (<=32KiB),
exact JOIN_READY response (<=4MiB INCLUDING its legacy framing), and <=8 original signed small fact
frames (each<=4KiB; unique sorted semantic IDs). The root Profile original may duplicate the existing
small owner tail so the complete response's original proof bundle is self-contained, but supplies
no independent trust. Request-binding hashes a new local domain plus exact signed invite/KP frames;
it is not a wire signing/CID change. Saved results remain immutable and lookup precedes current
role/expiry/target checks; same nonce/different complete binding refuses. Result count<=8 is a named
CHECKPOINT lifetime limit, with NO eviction. The scalable cold result index must replace it before
activation. The tail retains exact response bytes and public proof frames, not a saved-authority bit.

Record framing/order is EXACTLY the listed fields: put_bytes nonce, binding, KP hash, KP reference,
subject, InviteToken bytes, KP bytes, JOIN_READY bytes, then fact_count:u32 and put_bytes fact frames.
Decoder checks 1:1 equality with the unchanged legacy direct_admit_results prefix: identical nonce
set/count; legacy KP hash equals tail hash and recomputed exact KP frame hash; parsed JOIN_READY's
Welcome/routing/signature tuple is byte-identical to legacy cache. Recompute request binding from
the original exact frames plus group/profile/local full owner scope. Revalidate Invite signature and
all original Admission/Grant/Basis/Bootstrap chains under the external immutable pin; KP reference,
full subject and Welcome hash must match the original operation's actual opaque observation before
save and the retained signed Add/Bootstrap facts on restore. Historical result replay does NOT
rerun current invite expiry or admission lifetime policy. A malformed/foreign/inconsistent duplicate
refuses BEFORE writer/runtime/response admission; never select one representation as convenient.
Require max-boundary and each independently inconsistent prefix/tail field regressions.

Current private checkpoint has <=64 open documents, max_past_epochs exactly8, and <=576 retained
key rows (64*(8+1), each62B plus count), with future/current-epoch and unsupported document-type
checks. Its retained key row invoice is <=35,716B. Bound document HashMap scan/allocation separately;
reported capacity is not a physical bucket bound after tombstone-heavy deletion. Likewise bound
all graph/log bytes before snapshot, refuse every pending AutoCommit operation before get_heads/save,
and refuse durable-chat preparation/uncertainty debt rather than implicitly committing it.
First direct producer refuses all populated unsupported admin/device/companion pending/result maps,
preexisting MLS contests, and unsupported routing/policy succession. It preserves <=8 real member
finalization retry pairs. Existing direct result and signed commit history is never discarded to fit.

Conservative direct record invoice is 4MiB+80KiB (six fixed/framing labels + <=40,968B input frames +
<=32,804B proof frames + response framing; the remaining margin is explicit, not an unbounded field).
Eight records cost <=34MiB. The unchanged legacy direct result cache ALSO retains the same Welcome/
routing/signature: invoice its duplicate <=33MiB separately (exact max32MiB+444B including
its u32 count: each legacy prefix record is READY+55B); pin that actual maximum framing in a test.
It cannot be silently omitted from the
shared legacy prefix or used as authority without the new bound original record. Together with
<=36KiB keys, <=8MiB provider, <=32MiB committed graph/log source, <=8MiB retained commit records,
<=64KiB initial metadata and <=1MiB other fixed owner metadata,
the explicit source/candidate output invoice fits below144MiB. Each partition is independently
preflighted; the total cap alone does not prove those checks. One new consumed ledger row and possible
HashSet growth are reserved BEFORE mutation, separately from duplicate_bounded's existing-copy fee.
Existing current/replacement file and four-buffer fee remain held through exact repair/transfer;
response Bytes and lower serialization custody keep their own charged allowance through handoff.
All constants and boundary arithmetic are pinned by encoder max-size tests before publication.

Canonical successor wrapper keeps its existing domain/framing, but checked generation>1 requires an
exact32-byte original parent head AND generation == checked(current.generation+1) under the SAME
original head CAS; generation1 still requires empty parent. A restored single current file validates
framing/retained parent reference without inventing an external freshness anchor. The retry identity32
is the full original direct request binding. Current/head validation never selects a different
logical writer. The covering snapshot encodes latest committed live graph union through explicit
candidate references for MLS, tenure, ledger, commit log, old keys, immutable results and owner facts.
Only covering-save success installs those governing fields and admits the exact response. Failed
save leaves source governing fields unchanged and retains the entire candidate under original guard.

## Required focused execution

- Actual two-member join: owner pre-rename and post-rename/parent failures expose NO Welcome/facts;
  same request repairs and replies once; fresh owner/member reopen verify original exact evidence.
- Hold old source capture, advance/replace/retire original owner, then final save refuses before
  signing/export; same-number alias cannot fork or replace the known head.
- Request lost reply/reordered retry/foreign KP on same nonce: exact result unchanged, no second Add.
- Hold prepared candidate, consume live RNG in unrelated receive work, then fail/save/repair it;
  source RNG matches the uninterrupted advanced stream and candidate response bytes stay identical.
- Hold candidate then request unrelated invite revocation/admin demotion; governing intent cannot
  mutate behind it or be overwritten, and negative local retirement immediately closes both.
- Open/receive a new channel while candidate waits; final covering snapshot includes its strict
  old-epoch key; after install AND fresh reopen the actual old-epoch message still decrypts.
- Hold real saved response before lower driver admission, retire/replace original mount/writer,
  then release it; no response/facts escape and charged buffers remain owned until lower completion.
- Receive ordinary action in another channel during held preparation; final save/repair preserves the
  actual latest union and emits that channel's real event only after covering success.
- Actual Remove with original accepted Create plus unproven later action: archive retains both,
  projection holds only the latter, privileged log detail links exact evidence, signed decision and
  restart obey independently verified proof/finality policy.
- A1 actual removal, same full key A2 fresh KP/rejoin, old data/terminal history retained; old scalar
  context/observer/record cannot authorize new actor, and A2 cannot relabel an A1 action.

No code/tests are implied executed by this design supplement. Primary owns Cargo/full suites.
