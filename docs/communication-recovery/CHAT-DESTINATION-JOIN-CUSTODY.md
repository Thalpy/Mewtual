# Destination join custody (5 November; design, not implemented)

This is the narrow CLIENT companion to the covering owner first-Add checkpoint. It is not
production v2 invite/root distribution, a general helper/grant admission protocol, complete SDK
heap admission, or evidence that a remote owner saved anything. Compilation, tests and builds
remain paused. The existing consuming join still loses its private key-package material on an
error or cancelled future; this document describes the required correction, not shipped recovery.

The separately implemented [EMPTY-only v6 prerequisite](CHAT-CRITICAL-NAMESPACE-CATALOG-V6.md)
supersedes catalog implementation claims here: it supports NO nonempty namespace. Join1 and its
complete journal/client handler below are still design work. No compiler or test has run for v6.

## Existing failure and ownership

`PrivateManagedJoinAttempt::finish_direct` currently consumes its device and calls `Server::join`.
`request_join_tracking` makes a new key package on every invocation. `finish_join` authenticates
the inviter signature, then OpenMLS consumes the selected private key-package/init material before
the group, routing transfer and policy checks have all finished. An authenticated but invalid
Welcome can therefore consume keys without constructing a usable local Server. After a lost reply,
a new key package is a different request; the owner must not redeem the single-use invite again.

The owner-side cache matches complete original invite and canonical TLS key-package bytes. It may
return the original accepted ciphertext before checking new-admission expiry policy. That cache
does not preserve the joiner's private material and is not client recovery. New admissions retain
their existing lifetime, current-membership and signature checks.

This implementation agent owns `chat_admission/fresh.rs`, its client children, a new sealed join
journal and the separate catalog/persistence helpers. The owner-side agent owns Add/operation tail,
successor head/save and reply production. Its `chat_admission/restore.rs` generation-2 selection is
preserved. Owner and destination may use different vault formats; no v4 stage/negative classifier
is broadened merely because a new journal exists.

## Original source, retries and cancellation

Before the first network request, the actual destination preparation captures and durably saves:

* the exact original signed invite encoding, canonical TLS key package and its selected reference;
* the full group and local signer, authenticated trusted Profile ID and full founder key captured
  from the actual trusted source before the await (never a downloaded replacement root);
* the original destination admission-row CAS basis, vault domain and unique local journal identity;
* a bounded secret pre-join provider snapshot containing the actual signer and selected private
  key-package/init keys; and the exact kind-1 request bytes derived from those original frames.

The journal is private, vault-sealed storage. No raw journal decoder, scalar root constructor or
restored snapshot can mint fresh admission. Recovery is an explicit mounted-Store consumer of the
authenticated original journal. It obtains a NEW negative mount capture for that original source;
the old queued task retains its cancelled ORIGINAL capture and cannot adopt the new one.

An immutable master holds the pre-join source. Each finish operation restores one private candidate
from that same source, submits exactly the original request, authenticates/frames the reply, and
then applies the Welcome to the candidate. Invalid replies and cancellation drop only that private
candidate. The master and exact request remain unchanged. The method borrows `&mut self`; only
successful private handoff can move its original capture/pin/creation fee. Completed attempts cannot
construct another Server. There is no generic old-proof/new-capture Server constructor.

The direct wire remains `/catcoms/rr/1`: kind 1 followed by u32-framed original `InviteToken::encode`
and canonical TLS key-package bytes. No extra fields are added to existing peers' requests.
`JOIN_PENDING` keeps the existing authenticated pushed-Welcome behavior and outer join timeout.
The retained request path must never mint a replacement package. Expired historical results can
be processed only against the saved original source and independently verified exact owner result;
the historical metadata parser cannot admit a new package or bypass fresh policy.

## Critical v6 catalog before any cleaner

V6 is a new cryptographic wrapper/KDF domain over the SAME DEK. It defines a mandatory sealed
critical namespace catalog, fixed current and stage names. Constructor ordering is physical session
exclusion, orphan-intent refusal BEFORE any new-DEK creation, authenticated wrapper open, then full
current AND stage catalog authentication/validation BEFORE any existing inventory or stage cleaner.
Missing required catalog, malformed/unknown entry/version, inconsistent domain, nonregular source,
unsupported replacement or unknown stage refuses and preserves bytes.

The catalog is a bounded, sorted monotonic set of `(namespace, version)` entries, with no removal.
The planned first supported namespace is `join=1`; it is NOT supported in the Empty-only slice.
A future archive implementation MUST durably install
its new catalog declaration (including any stage/intent) BEFORE creating its new files. An older
v6 join-only reader sees unknown `archive=2` in either source and refuses before cleaning; v6 does
not silently claim support for an unimplemented archive. Exact current/stage retries authenticate
both complete candidates and cross actual file/root barriers before discarding a covered stage.
Different or regressing candidates are retained/refused, not merged into manufactured authority.

Activation consumes the original pre-exposure Store. It prepares/preserves the existing v4/v5
inventories, initializes the supported mandatory join journal, durably installs catalog intent,
then rewrites the wrapper and confirms the SAME keys/classification. Every failure drops/revokes
the original mount. Constructor repair must complete the same supported order or refuse; a
half-upgraded borrowed Store is never returned. V6 remains sticky through ordinary scope/owner
activation and secret changes. Positive owner-evidence/operation consumers accept authenticated
known v6 through an explicit format capability; exact v4 S1/negative exceptions remain exact v4.
An authentic old wrapper ALONE can still expose the preserved DEK to an old binary; this normal
reader fence is not an external component-rollback anchor.

## Bounded journal and state transitions

The first draft proposed ONE pending destination join per physical vault. That is NOT cleared for
activation: an unreachable inviter must not indefinitely block unrelated joins. The connected
implementation must instead finalize explicitly bounded active/paused slots and persistent/hot
allowances, retaining an abandoned/paused original secret source without reusing its ambiguous
invite. Refusal never evicts an unresolved attempt. This is a local concurrency/resource policy,
not a group-size or lifetime-admission cap. The journal is intended to be embedded directly in its
sealed catalog namespace payload, with no separate mandatory-file/bootstrap gap. No unbounded
directory scan or per-peer filename is used. Exact slot count and fee are still design work.

Record grammar is closed: `Empty`, `Pending(original source)`,
`ValidatedReply(same source, exact authenticated fully processed reply)`, and
`Settled(same source identity, actual saved canonical identity)`. Every transition checks the exact
previous record and checked next revision under the journal serializer. Overflow refuses. Pending
source is immutable; no reply may substitute its invite, key package, Profile or admission basis.
Only a completely validated reply may be retained; a merely signed but invalid first response must
not poison the journal. Current/stage both validate before selection. A different source, malformed
stage or unsupported transition preserves/refuses rather than resetting to Empty.

The budget must pay the fixed catalog/current/stage plus journal current/replacement, retained
original plaintext, serialization/sealing/framing overlap, and original/candidate provider table
backing and retained Vec capacities. Codec lengths/counts and provider-map backing are preflighted
before allocating their corresponding outputs/maps. The existing canonical head fee does not pay
for a third durable private source. The separate SDK typed JSON/Welcome/group heap/work gate remains
open: neither encoded-byte limits nor a generic multiplier establish that bound.

Before candidate network submission, the original journal file and parent/root barriers must have
completed. A possibly transmitted original request is never treated as an unused invite merely
because no local reply was saved. Local loss/negative retirement cannot undo a remote Add or license
minting a new package from that invite. A malformed durable source is an unavailable original attempt,
not evidence that it did not exist.

`ValidatedReply` is saved before local scope reservation. On crash, it is processed AGAIN on the
original private source and must produce the same actual selected package/Welcome binding. The
ordinary fresh proof is created only by that real verified join. If exact Fresh+Initializing/Current
already exists, its same provenance can resume; foreign/later/Retired lineage refuses positive work.
An already committed canonical member source is restored/validated using its actual saved barriers,
not replaced by rerunning a first initializer. Exact journal/Welcome/member-bootstrap comparison
must finish before settling that source; decoded digests alone issue no fresh or Saved witness.

## Publication and settlement ordering

Ordering is original admission writer -> canonical head -> join journal serializer. Initial journal
prepare does not hold the journal lock while acquiring admission/head. Recovery reads/captures the
journal, releases its lock, then enters the ordinary original-source admission transaction; every
positive commit rechecks the exact journal identity under the final journal lock. There is no inverse
journal->admission lock acquisition or writer held across network work.

Successful local member initialization first crosses the actual canonical file/parent, durable
Current and exact Saved/head/writer barriers. Under those SAME original guards it saves the exact
`Settled` terminal, dropping private journal secrets only after the real terminal file/root barrier.
Only then may `take_ready` move the runnable Server. A lost terminal reply/restart must recover the
same canonical source and settle idempotently, with no second Add. Partial rename/barrier failure
retains the original source/candidate and cannot be reported as accepted from Current alone.

Replacing a settled slot revokes its old in-process attempt before exposing the replacement. Lock,
mount replacement or journal replacement therefore stops queued old work at final admission. New
mount recovery has its own opaque original source; it cannot revive an old queued future. Explicit
UI generation publication remains a separate native integration requirement.

## Required deterministic acceptance (not yet authored/executed)

1. Exact request bytes/KP and signer survive rejection, timeout, lost reply, future cancellation and
   repeated malformed/forged/signed-but-invalid Welcome; private master bytes stay unchanged.
2. Real owner covering Add occurs once; reply-loss/reopen returns original result, including after
   fresh invite/KP expiry, while changed KP/nonce/profile/request cannot replay the cache.
3. Crash at journal stage sync, rename and parent barrier, accepted-reply save, member canonical
   stage/promotion, Current, Saved and terminal journal save; both known-source retry orderings.
4. Lock/retire/replacement wins before network/final admission, and admitted final publication wins
   before lock completion; old queued task after lock/unlock never obtains new custody.
5. Current/stage unknown namespace/version, missing catalog/journal, orphan intent without vault,
   malformed counts/truncation/duplicates, domain/root/key substitution, revision overflow and quota.
6. Both valid legacy formats remain unchanged; v6 sticky activation/secret rewrap, old v4/v5 readers
   refuse v6; known v6 positive restore works while exact v4 negative/stage fallback stays closed.
7. Genuine selected-KP member App stage recovery using the actual owner Add producer, not fabricated
   signed facts. Source-cache tests alone do not establish full client recovery.

All new runtime boundaries require read-only design and actual-diff review. Tests are to be authored
alongside each connected slice and remain UNRUN until the explicit compilation pause is lifted.
