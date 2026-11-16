# Covering owner membership operation tail

Implementation contract for the direct-owner Add transaction described in
[CHAT-OWNER-MEMBERSHIP-CANDIDATE](CHAT-OWNER-MEMBERSHIP-CANDIDATE.md). This is an explicit **inactive
checkpoint** contract. It does not replace the scalable v2 public-header/cold-archive contract and
must not be enabled for long-lived production groups. It does not authorize legacy JOIN to run
before the covering save. No source implementation or executed test is implied by this document.

## One transaction, three distinct boundaries

1. Original canonical owner/writer and a bounded direct request admit a private MLS Add candidate.
   The source MLS group, invite ledger, governing roster and user-visible history are unchanged.
   The original node RNG may consume one routing-seal nonce; its state is never cloned or replaced.
2. A covering successor snapshot contains the private MLS state, exact owner facts, consumed invite,
   request result, old document keys and publication debt together with the latest live immutable
   receive union. Actual admission -> head -> Store file and parent barriers install that snapshot.
3. The successful canonical writer installs the private governing state and reconciles negative
   subordinate authorities before it can admit the original response/commit debt to the driver.
   No pre-save public byte getter or callback escapes the private operation.

A rename followed by failed parent sync crosses neither the successful-install nor response
boundary. It retains the entire exact operation as possibly installed. A fresh request cannot mint
a replacement Add while that debt exists. Host retry includes later received immutable history
without changing the candidate's MLS state, signed outcome or original request binding.

## Strict new extension

The new owner-operation tail follows the existing managed-scope and owner-evidence tails in the
Sync snapshot. It has a distinct mandatory domain/version marker and a fully consumed canonical
length-delimited body. Unknown versions/tags and trailing or duplicate records refuse. A plain
decoded tail is data, never a Current admission, Saved result, writer or fresh-join witness.

The top-level owner-evidence vault v5 barrier is already required before this namespace is used.
Older v4 code must refuse the vault before its legacy cleanup paths, rather than treat a missing
understood tail as an empty operation ledger. Legacy snapshot bytes stay unchanged when no owner
operation exists. An owner-profile snapshot whose MLS state requires an operation tail cannot
silently omit it or fall back to a legacy admission path.

The tail must retain these exact records:

| Record | Required contents and validation |
| --- | --- |
| Past document keys | Sorted unique `(DocType, u128 document, u64 epoch, [u8;32] secret)` rows. A bounded final pre-advance view captures every document's old-epoch key from the still-live original group; any failure refuses. Existing retained windows survive. Unknown DocType and keys for impossible future epochs refuse. Keys remain zeroizing and sealed at rest. |
| Direct request outcome | Original full owner/group/profile binding, exact invite and TLS-encoded selected KeyPackage, invite nonce, request digest, subject full key, selected KP reference, exact original Welcome/sealed-routing/signature and original signed public facts. All cross-fields must match verified actual membership observation and the accepted private outcome before save. On restore the public signatures, IDs and request/response cross-bindings are checked again against the independently captured immutable root. |
| Publication debt | Exact signed `CommitRecord` and the **pre-transition** control topic, plus original direct response identity. A Remove must never derive the publication topic from its new routing label. Local queueing and transport ACK are not recipient receipt. A durable terminal clears debt only under its own exact covering successor. |
| Governing revision | Exact predecessor canonical head and source governance revision, scope incarnation and operation kind. The original writer guard serializes every covered MLS/admin/invite/policy mutation. Message receive may add immutable graph union but cannot replace governing state behind this candidate. |

The response should have one retained encoded allocation. A legacy `CachedAdmit` projection may
borrow its validated ranges while materializing a bounded response, rather than retaining another
unaccounted full clone. Maps are keyed by full exact request identity; nonce collision with different
invite or KP bytes refuses. Hash/digest equality alone never substitutes for the retained request
and independently verified public-fact chain.

## Preflight and physical invoice

Before cloning or encoding, check collection counts, original hash-table scan/backing limits,
per-field lengths and aggregate checked sums. BTreeMap iteration is bounded by its count; any
HashMap work bound must come from an independently enforced lifetime maximum or actual backing
observation, never `len`/reported capacity alone. Do not compact the source to manufacture a fee.

The reservation separately owns the retained original/private candidate, provider table backing,
actual Vec payload capacities, strict tail, signed fact frames, response allocation, snapshot,
wrapper, sealing and framed-write buffers. SDK JSON/group heap expansion requires its own bounded
decoder/invoice; a provider-byte cap does not prove that expansion bounded. Required negative/control
capacity is reserved before admitting ordinary work. Checked overflow or quota exhaustion refuses
new work without evicting old facts, responses, receipts, Stops or uncertainty.

For this checkpoint the existing eight-fact owner tail limits the first founder/member vertical.
Do not raise a constant and call that scalable. The production successor needs immutable cold
pages and bounded hot indexes, semantic signed public headers, count-aware proof/append invoices,
control reserve and crash-safe manifest/head publication. It must permit ordinary members to author
offline under their admission incarnation without asking the owner to certify every post or channel.

## Recovery and response custody

There are two different pre-replacement cases. A FIRST initialization with no exposed canonical
source may be negatively retired only after original Initializing-row classification and actual
absence/barriers establish that no `.bin`, stage or replacement uncertainty exists. Absence never
constructs a fresh source or promotes it to Current.

A later membership successor already has an ORIGINAL Current canonical `.bin`. Before any possible
successor replacement, process loss may discard its never-exposed private Add only after validating
that the durable canonical file is still that exact original captured head and no successor stage
or uncertainty remains. This preserves the original Current owner, invite ledger and history; it
must not require the original `.bin` to be absent or negatively retire that owner. Any retained
successor stage must be preserved and classified before choosing this path.

After a possible replacement, the exact canonical snapshot and tail are the recovery source. The
opener validates and re-syncs that source before reconstructing any runtime debt. It never rolls
back to a numeric alias, derives an outcome from the current roster, or recreates the Add from a
request whose original result is uncertain. Rejoin A2 preserves A1 signed history, terminals and
incarnation bindings; it cannot replace the canonical file with an empty new-member snapshot.

Lookup of an already durable exact request precedes fresh invite/role checks, so reply loss cannot
consume the invite twice or add another leaf. Returning that original ciphertext does not confer a
new current membership. A genuinely new request still passes current policy. Old outcomes remain
in the retained ledger; at-cap admission cannot evict an outstanding result to fit a new one.

The durable outcome retains immutable operation, request, recipient and result bytes, NEVER the
first request's ephemeral ReplyWait as permanent positive authority. Each genuinely new authenticated
exact retry captures its OWN original current mount/admission/writer/head and Responder before any
await. It verifies the original durable outcome is retained in that independently validated canonical
closure. Cold restore may therefore answer an exact accepted request without reviving the old
process's writer. A previously queued task cannot switch to that later writer or request observer.
An uncommitted private Add remains tied to its original preparation's writer throughout.

Only the attempt's consuming response handoff reaches `Responder::try_respond_with_custody`.
The driver repeats that attempt's ORIGINAL effect authority at actual response admission and keeps
its charged service/bytes alive until lower work retires. Reply-wait expiry is a cancellation source
for that reply attempt, not an irreversible destruction of the durable outcome. Retirement before
admission refuses. Already admitted network bytes cannot be recalled; local cancellation must not
be described as remote erasure.

## Required regressions before any activation

- Actual owner and actual fresh selected-KP member, with failures before rename and after rename
  before parent sync: no early response/facts, exact retry result, one Add and one invite use.
- Lost reply, duplicate request, changed valid KP on the same nonce and old A1 request after A2
  rejoin: preserve the exact original outcome and incarnation; no authorization retargeting.
- Cancel the first reply wait, then issue a genuinely new authenticated exact retry: return the
  same bytes with one Add, while the old queued callback and same-nonce/different-KP request refuse.
  Repeat after actual cold restore with a fresh original writer capture.
- New channel/history arriving during a held candidate: the final view contains its signed union
  and old-epoch key; repair preserves both without reminting MLS or dropping a user terminal.
- Actual concurrent original owner retirement/replacement and response held at driver admission:
  old candidate and old response cannot adopt a new owner, even if a numeric alias is reused.
- Post-save installation reconciles removed subordinate scopes before any response or commit drain;
  pre-save/uncertain paths emit no false successful user/admin event.
- Corrupt/truncated/oversized tail, unknown version/DocType, duplicate keys/outcomes, integer
  overflow, inconsistent request/public-fact/response binding and exhausted control capacity refuse
  before allocation or mutation, preserving existing exact durable state.
- Actual cold reopen validates the stored proof chain and exact tail, crosses the real barrier and
  resumes one original debt. A visible but unsynced file alone never counts as successful save.

The primary owns all compilation and complete backend/native/frontend gates. Current user steering
pauses compiling tests; all new selectors remain unexecuted until validation resumes.
