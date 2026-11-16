# Next file milestone: connected member page service

Current implementation, 19 October 2026: the native requester now connects explicit chunk/range/
Keep/private Download reads to source30 and helper29 after direct providers. The original native/Root/caller scope,
strict bound helper, exact encrypted variant, route custody and finite work allowance remain
mandatory. Actual C/B/A tests and four blocked-edge TCP/native file scenarios pass, covering Keep
and private Download, solo revalidation/export, and interrupted C native restart with replacement D.
An explicit observer-only sealed D→A/C route prerequisite passes under the same existing
recovery deadlines. Complete programme, full-suite and physical-network acceptance remain open.
The primitive-stage checkpoints below are historical.
Existing kind5/kind27 reads keep
their released authentication, peer binding, response grammars and compatibility behavior.

## Roles and route

Requester C owns an explicit current file read or saved Keep intent. Helper B and source A are
current members of the same full MLS group; neither needs an open Files view. C reaches B and B
reaches A over already established, currently authenticated member connections. No request creates
a dial, accepts a URL/address, discovers another group or recursively forwards. A only serves its
own already held encrypted chunk. B forwards opaque ciphertext pages and holds no file-wrap key
in its network worker. This is not remote custody evidence or member media relay.

C requests a page through one fixed helper after exhausting direct connected candidates. The
helper may search at most four distinct live, bound current-member sources, excluding C and itself,
under one original deadline. It stops at the first authenticated nonempty page or exhaustion.
No nested bridge request is a permitted inner operation. Direct helper-held content can satisfy
ordinary kind27 before this path. There is no new automatic redundant-copy policy or cache write
on B: forwarding cannot turn a read into durable local ownership.

## Closed new wire domains

Request kinds29 (C to B) and30 (B to A) require explicit interface/budget registration
and fail-closed old-peer refusal. They must not weaken kind27's actual-requester peer binding.
Each framed request is at most2 KiB; page length is at most64 KiB, and the complete signed response
has a separately derived closed ceiling. No legacy whole-blob request is forwarded.

The original C grant names protocol version/domain, full group, current epoch, fixed helper
DeviceId and PeerId, original requester PeerId, ciphertext CID, offset, requested length and its
fresh nonce and timestamp and at most four distinct permitted source DeviceIds. A checks
its own full identity is explicitly named; a malicious helper cannot replay the grant across
an unbounded member set. C signs the exact frame. At B, the declared requester must equal the
actual authenticated Noise endpoint and its current full-member binding. B must equal the fixed
helper. The epoch is exact; wall freshness is strictly less than20 seconds, while locally captured monotonic
deadlines are never extended by retry or clock correction. No forwarded grant creates standing,
peer-record, presence or connection proof for its original requester.

B signs a closed delivery envelope containing that original frame and submits it only to a
captured eligible source connection. A authenticates B against the actual Noise endpoint, checks
B is the original fixed helper, and separately verifies C's original current-member signature
using its declared requester PeerId in the original transcript. This is an explicit forwarded
page authorization, not direct-request authentication. A does not infer a C connection. Both
outer and inner group/epoch/member checks apply before disk reads; no caller can select a secret
key, local path, response signer or another protocol operation.

A's new-domain signed page response binds C's exact original grant hash, requester, group/epoch,
helper, CID, requested offset/length, result marker, total and bytes. B verifies the exact captured
A signer and current scope before wrapping it in a helper-signed response bound to C's original
request. C verifies that exact B, then the current A member and its unmodified response. A cannot
be replaced by B's assertion of success. An all-absent/refused result is signed helper evidence
only; it is not proof that no group member holds the file. The actual provider display remains A.
Ciphertext assembly still checks the complete ciphertext CID before file-layer opening, and the
original actor rechecks its current manifest/run/session/membership before any storage effect.

## Bounded ownership and cancellation

Admission must prepay the complete helper chain before accepting an inbound responder. Proposed
bounds: four helper chains per group, eight per process, one per original full DeviceId per group;
no unbounded pending queue. Budget1 MiB per helper chain, to be checked against exact encoding
and actual allocation. Retain fixed accounting for original/nested request bytes, at most one
source reply, output wrapping and continuation metadata until their actual last owners retire.
A conservative byte ceiling must be derived before code. One chain searches at most four sources
serially, each with at most2 seconds and an original total at most8 seconds, below the requester's
12-second attempt bound. A busy source cannot cause an unbounded scan or a new connection. Application-serial attempts
can overlap below: a two-second upper timeout does not terminate the driver's ten-second request.
Prepay all four possible retained lower attempts and keep the chain/original-device owner until
EVERY actual lower retires. Eight-second effect expiry cannot admit a replacement chain into
still-owned capacity. Queueing, preparation, source reads and completion all spend the original
eight seconds; no stage starts a fresh total deadline.

Per-original-device replay/rate state is bounded without evicting live protective rows to admit
fresh identities. Reserve a chain slot and debit its finite request/byte allowance BEFORE lower
handoff; refused or cancelled local work cannot refund an ambiguous submission. Existing A-side
per-member byte limits also apply to the original C, independently of B's own helper budget. B
needs its own bounded per-original-device byte/work policy even for negative responses; rotating
Noise peers cannot refill full-device limits. Cap misses and accepted fresh nonces as well as hits.
Both B and A use fail-closed bounded ledgers. A charges original C AND immediate helper B.
Original nonce reuse with different complete grant bytes refuses; exact duplicates refuse
without repeating work. Proposed fresh-grant ceilings are eight/device/second,
16/group/second and32/process/second, with at least64 KiB charged even for misses, plus a
separate bound for actual full-chunk source-read work. Replay storage is at most1,024/group
and4,096/process, refusing admission while live rows fill it. Monotonic effective freshness
must advance during a wall rollback so cache expiry cannot reopen a still-accepted old grant.
Cross-restart replay/freshness semantics must be explicit; runtime nonce rows alone do not
establish durable exactly-once service or extend the origin Keep allowance.
These are starting throughput choices, requiring measured saturation evidence.

The mutable Root only authenticates, prepares, revalidates and completes. Detached work owns
immutable bytes, transport, captured connection leases, cancellation and affine accounting, never
MLS/device/store state. Keep the inbound responder and bounded job owned across a cancellable
Root wait, not in a stack frame that can be dropped between polls. Service must progress through
the ordinary Root/ServerActor loop, not a view or a test-only drain. A source wait cannot suspend
inbound handling, chat commands, another group or maintenance.

A helper effect gate captures the Root instance, original epoch, local membership, requester,
fixed helper and selected source identity/descriptor plus actual connection incarnations. Revoke
it synchronously at the existing observed membership/descriptor/proof/connection transition hooks,
before another await. Root replacement/drop and native server closure retire it. Drivers repeat
the original deadline and revocation check at actual admission. Late responses cannot reauthorize
an old chain; the lower request's actual accounting remains held through real retirement. Revoking
an effect is independent of refunding its memory or concurrency permit.

Requester-side Keep retains its existing durable allowance. This allowance bounds its own native
requests/accepted bytes, not total distributed physical bandwidth; helper work has its independent
finite original-device policy. No UI view refresh, route change, helper replacement, unsupported
reply or restart refills Keep's grant. Bridge pages keep their64 KiB ceiling even if a direct path
previously measured a larger page size. Unsupported kind29 ends that bridge candidate; it never
falls through to a forwarded legacy whole-blob request.

## Separate owned source read

Existing kind27 service loads and authenticates the whole at-rest blob before slicing. Reusing
it would not create a64 KiB source memory/disk bound. Introduce a reviewed opaque native
`BlobReadPlan` seam: Root authorizes the exact CID/bound and captures only its current storage
source; an owned blocking worker performs disk read/AEAD/CID checks. It owns no MLS/device/file-wrap
key or store borrow. A sealed source necessarily carries its native at-rest sealing key in a
zeroizing worker owner. Nothing crosses IPC. Explicitly cap encoded allocation, decoded ciphertext,
AEAD output, retained fallback/memory snapshots and page/transcript/wrapper copies. Source scratch
is separately prepaid (initially one/group, two/process, 40 MiB each). The implemented read
primitive requires more than 32 MiB for an 8 MiB + 44-byte chunk; its explicit bound must never
be rounded down. Its permit follows the actual blocking closure
through return after cancellation/Root drop; source work is not included in B's1 MiB chain.

Each prepared source/result retains an exact store-lifetime/current committed-winner fence.
Forget, poisoned publication, winner/store replacement, scope loss and Root drop suppress stale
signing. No partial retained candidate or staging source can be captured. Revalidate source and
original current membership/epoch/grant on the actor before signing; a timed-out blocking task
cannot mutate state or deliver bytes, and its still-live scratch is never refunded.

Deferred responder closure needs a narrow runtime wait seam for receiver closure or original
inbound-authority revocation. It stops another helper attempt and suppresses wrapping without
dropping the real lower/read charges. File-service reconciliation must run at the ordinary
observed networking/MLS transition hooks before optional voice-runtime early returns; no voice
attachment or UI view can be required for revocation.

## Required regressions and remaining boundaries

Use the same enforced topology in deterministic Root/app tests and native real-TCP fixtures:
A-B-C with A-C forbidden, then B loss and replacement D. Record that only permitted edges were
used and verify exact source-signed ciphertext/plaintext, with no hidden direct dial or test copy.
Exercise B serving without UI, missing/removed/cross-group sources, wrong helper/requester/signer,
nonce replay, altered window/grant, expired original deadline, copied outer envelopes, full tables,
miss floods, cancellation while a lower is held, lock/server close, provider replacement and late
results. Preserve actual lower charges; another group and chat/control must make measured progress
under sustained bounded bridge load. Old peers refuse new kinds without changing direct behavior.

Run actual app Keep over this path, pause/reopen under its saved allowance, exhaust/restart with a
healthy source and require explicit Retry, then complete whole-variant fallback without splicing.
External destination resume, physical two-network/native audio, live voice V2 and member media
relay remain separate unfinished boundaries. Only one desktop is currently available.


The bounded read primitive, affine source task/completion, process-stable source pool, concrete
native blocking executor and reply-lifetime cancellation seam now exist with focused checks.
They do not activate this proposal. Signed frames and an inert process-stable admission policy
now exist too; current Root authorization, revocation hooks, response custody, service dispatch
and requester integration remain unimplemented.


## Historical transport allocation and freshness checkpoints

The released request/response codec accepts 16 MiB frames. A 72 KiB protocol response promise is
insufficient to bound a malicious peer's raw response allocation, and four timed-out upper waits
can still retain four actual lower requests. The implementation must enforce the new frame bounds
before body allocation. The pinned libp2p-request-response 0.29.0 handler uses one mutable codec
clone per substream: outbound `write_request` then `read_response`, inbound `read_request` then
`write_response`. A value-only per-clone response limit can capture the request class without
changing released protocol negotiation or request IDs. Reset that limit for EVERY request,
including empty and legacy requests. New-kind inbound request admission reads only length and one
kind byte before rejecting a request above 2 KiB; response admission rejects its length above
72 KiB before allocation. Outbound oversized writes fail before emitting a header. Legacy 5/27
and other released kinds retain their 16 MiB behavior. These codec limits now pass focused unit and malicious-peer TCP regressions. Application service
dispatch and authorization remain unimplemented; registering kinds alone would not establish them.

Original-grant freshness will be strictly less than 20,000 ms in either direction. Replay rows
remain protective for 40,000 monotonic ms; the strict boundary prevents a future-dated grant from
being accepted again exactly when its row expires. Effective wall time advances from its latest
high-water mark using monotonic elapsed time, including after a wall forward step followed by a
rollback. Arithmetic overflow fails closed. Serving-side runtime replay rows are not durable
exactly-once evidence across process restart: an otherwise still-fresh grant may be accepted again.
Its explicit maximum four named sources, scoped service limits and original requester allowance
remain required; no such restart refills the durable origin Keep grant.


## Historical closed frame implementation

Pure signed frame parsing/issuance exists in `file_service::wire`; all eight focused checks pass
and read-only review has no blocker/high/medium. Strict sync Clippy passes. These types do not authorize a current member, process an inbound request or start a
read. Original grant body includes full group, exact epoch, C peer, fixed B device/peer, exact
ciphertext window/bound, sorted distinct source IDs, timestamp and nonce. Four separate signature
domains cover original grant, one-hop delivery, source page and helper wrapper. The source/helper
responses use the normative v1 hash: BLAKE3 of the **complete raw signed kind 29 frame**, which binds all its fields,
key and signature. Response bodies need not repeat those fields. Parsing is borrowed and rejects
noncanonical role/window/negative-marker shapes before crypto. Signature verification is explicit
and is not current-membership or replay admission.

| Complete frame | Maximum bytes |
| --- | ---: |
| Original kind 29 grant | 707 |
| One-hop kind 30 delivery | 893 |
| Source signed 64 KiB page | 65,691 |
| Helper wrapper including source frame | 65,842 |

Application preparation must derive the maximum ciphertext size from its exact authorized file
reference. Helpers verify their captured source identity/connection as well as the nested signature;
requesters verify both current helper and original source keys. A negative helper result is only
that helper's observation. The process-stable admission policy below adds replay/rate/crypto checks. Source caching, final
effect custody and Root/actor dispatch remain unfinished; these frames are not a working bridge claim.


## Historical process-stable admission policy checkpoint

`file_service::admission` now supplies the inert policy stage. Its only production constructor
shares one process ledger and sanctioned clock across Root/executor replacement. Native ingress
must supply current full-group, epoch, local identity and authenticated Noise sender evidence. An
affine ticket binds those observations, the exact complete input frame and an eight-second local
ceiling. It grants no membership, connection or native-session authority.

Before crypto, actual senders spend bounded one-second windows: eight per full device/group,
16/group and 32/process. Verified work charges helper chains at 368 KiB; uncached sources pay
the declared whole ciphertext bound plus 72 KiB. A charges both original C and immediate B
against independent 64 MiB/device/group windows; aggregate work counts once against 128 MiB/group
and 256 MiB/process. There is no cheap cache-hit admission. At maximum chunk size this permits
seven source reads per device per second; throughput and requester backoff still need measurement.

Preflight reserves nothing. Future integration must acquire actual capacity, recheck current
Root/native/member/connection scope, then commit. Commit repeats all time, replay and capacity
checks under one lock and publishes all debits plus nonce or none. Precrypto ingress remains spent
on refusals; admitted work has no ambiguous-handoff refund. Live protective rows are never evicted
for new identities: at most 1,024 device windows, 64 group windows and replay limits above. Replay
keys include local service device and phase so a one-process A/B fixture can legitimately admit the
same original grant once at B and once at A. Epoch changes do not reset protective rows.

Clock sampling and pruning occur under that same lock. Effective wall time advances from its
latest high-water mark using monotonic elapsed time; backward monotonic time, overflow or mutex
poison fails closed. A large wall correction can leave the process ahead of other devices and
refuse service; local request timestamps use the same effective timeline. Runtime replay is not
durable exactly-once service across process restart and never refills origin Keep allowance.

Eleven focused tests and strict sync Clippy pass; read-only review finds no blocker/high/medium
in this policy slice. Source two-second hop deadlines, current native authority before commit and
effects, actual capacity custody and ordinary Root dispatch remain activation requirements.


## Historical activation review checkpoints

The ordinary file worker now has independent actor and Root/member/policy observers. Native
background Keep also captures exact entry/account ownership. These do not supply the stricter
bridge route gates, native handler attachment or live dispatch. Before activation:

- Preserve service custody through the driver's actual response copy. An application Bytes owner
  alone is insufficient: the driver copies into a Vec, then drops those Bytes. The new explicit
  response-custody API composes the service owner with driver response capacity; both byte clones
  and the actual pending response retain that fixed composite.
- Source Present replies need a storage-current observer separate from work cancellation. Reply
  receive ends the work wait before actual send; querying completion.result at the final driver
  gate would self-cancel every success. Retain completion as custody and separately check its
  weak storage fence plus current native/Root/deadline scope.
- A successful helper Found response must retain its selected A child scope through final send.
  The parent excludes unused candidates so A loss can rotate to D, but a wrapped success must
  still reject later A membership/descriptor/proof/connection replacement before admission.
- Repeated healthy native attachment observations must preserve healthy jobs. Old captures remain
  bound to their original native/actor generation; failed freeze resumes with a new generation.
- Typed Unavailable distinguishes transient capacity/rate/timeouts from Absent and unsupported.
  C retries within finite deadlines using fresh grants and unchanged durable origin allowance;
  it must not turn capacity refusal into permanent absence or legacy whole-blob forwarding.

The eight-second total is B's locally captured ingress ceiling, composed with two-second child
hops. A enforces its own local admission/read ceiling. C close or lock cannot remotely revoke an
already-signed delegation grant: malicious B can present it while it remains fresh at A, but only
to the finite named sources and under A's current C/B membership, replay and work limits. Accepted
clock skew/future timestamps also prevent claiming a universal physical20-second lifetime.
C still rejects late results against its original local job before any storage effect. This is
bounded delegated service, not an instantaneous distributed cancellation guarantee.


The explicit response-custody seam, positive storage-only completion observer and inert helper
pool now pass focused regressions and clear slice reviews. Helper memory reserves1MiB for a
conditional944KiB explicit-buffer envelope. The outer affine chain mints four monotonic lower
preparation tokens and is consumed into final response custody; a future pending-request builder
must consume each attempt token once and pin the actual serialized allocation trace. Independent
lower-only and response-only retirement cases prevent one owner from masking the other. These
primitives alone provide no dispatcher or requester path.

### Historical source30 activation checkpoint

The current implementation now dispatches source30 through an explicit `NativeFileService`
attachment. Its native account/exact entry and actor observers compose current Root membership/
policy with the strict immediate B descriptor/proof/connection capture. Original C is checked
against its current MLS key, without requiring or inventing a C connection. A retained Root
queue owns deferred metadata and rotates completions with ordinary network recovery. Native
read tasks and deferred metadata share the same opaque source reservation independently, so
panic or outer cancellation cannot refund another actual owner. Present retains the original
two-second read deadline through final driver admission, plus its current storage fence;
reply-wait retirement is excluded. An eight-second parent permits a bounded timeout Unavailable
observation. Actual dispatch and lifecycle/deadline regressions pass; the revision ledger records
the positive evidence, deadline-only red control and review. Requester/Keep was unfinished at this
historical source-only checkpoint; see the current status above.

Helper integration must prepay five bounded route records per admitted chain, rather than
discovering a registry-capacity refusal after three attempts. The private builder consumes
one parent/four child slots with the same retained credit. A successful Found keeps its selected
A scope and exact two-second hop cutoff at final admission. Once queued, expiry can suppress it;
the lower driver cannot replace it with an Unavailable response. Root may choose Unavailable
before transferring the responder. None of these limits renews the original eight-second scope.

### Historical helper29 activation checkpoint

The helper is now dispatched with its original native/actor/Root/current C scope. It reserves
all five route records before irreversible admission, then consumes each affine lower token
once. Scope positions never reset on source/proof changes. Actual lower replies must match
the selected A DeviceId and current MLS public key before the helper wraps the original signed
source frame. The final Found gate retains C and that selected A, including its original
two-second cutoff; unused-route loss permits rotation. Any skipped, invalid, unsupported or
temporary result leaves Unavailable as the exhausted-chain outcome, unless a later source
succeeds. Only every actually checked signed Absent observation yields helper Absent.

The allocation trace keeps the original944KiB explicit-buffer derivation inside1MiB: four lower
responses, two copies of each of four2KiB requests, eight additional72KiB response windows and
64KiB metadata. The metadata layout regression covers five scope/registration pairs, four
continuation layouts and16KiB watch/Arc/group overhead. Signing/wrapping borrows the original
source bytes; it does not accumulate whole chunks or full historical source replies. Original
received Bytes and clones independently retain the same chain across final signing/admission
refusal. Responder attaches service custody before its earliest refusal and retains a separate
driver composite for actual serialization. Source/helper/metadata/cancellation tests pass;
requester integration and blocked-topology TCP Keep were still open at that historical helper-only
checkpoint. The current requester/Keep evidence above supersedes it; complete programme acceptance
remains open.
