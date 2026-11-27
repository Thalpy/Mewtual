# Production Chat V2 join: next vertical

Production design, not an implemented owning join or activation approval. The
initial isolated [DATA codecs](CHAT-V2-JOIN-BINDINGS.md) are under validation;
their design/diff reviews are clear after corrections. Chat V2 remains OFF.
PR #128 connects warm local saves to the renderer; it does not
communicate with a second member. The existing joined-source coordination
decisions remain binding.

## Required outcome

Founder A admits B and C using real MLS and a covered governing Add. A then
stops. B saves an ordinary message once and C receives the same signed original
through a permitted real transport path. They recover history after restart,
including edits/deletions, without per-message founder certification. Missing
parents cause bounded catch-up, never invented authority or an automatic fork
winner. The ledger keeps original actions; projection hides replaced/deleted
values while authorized admin audit retains them.

This requires a live founder source as well as a joined source. The current
`UnboundV2GenesisNode`, generation-1 member lane and one-slot/epoch-zero archive
remain unchanged until their explicitly versioned successors exist.

## Signed protocol bindings

Use independent, domain-separated V2 codecs, not a wrapper containing a valid
Legacy `InviteToken`. Legacy invite/request decoders must refuse the new bytes,
and a Native V2 endpoint must refuse every Legacy admission request before SDK
Add. Communication policy (peer-to-peer/dedicated) is not protocol mode.

The proposed small signed **invite binding** contains:

- the complete genesis anchor: group, founder key, Profile, founder Admission,
  tenure, genesis Header and state commitments;
- inviter key, one-time invite nonce and expiry;
- immutable communication-policy commitment and versioned routing-plan commitment.

Policy/routing bodies travel with the pasteable invite but are separately capped
and must match these commitments before use. They contain route descriptions,
not an embedded Legacy invite. No connection hint is permission to scan/dial.
The [initial invitation packet](CHAT-V2-INVITE-ISSUER.md#invitation-packet)
implements the bounded binary envelope for the warm founder. This profile
carries a TCP reservation with zero certified contacts.
The initial accepted out-of-band invite is the trust decision; signature
verification or a downloaded self-signed Profile alone is not one. A nonfounder
inviter carries the already pinned genesis anchor. An existing same-group pin
conflict preserves both inputs and refuses repinning.

The signed **join-request binding** names the exact V2 invite commitment,
genesis-anchor commitment, joiner key, retained request ID, domain-separated
commitment to the exact KeyPackage wire bytes, and the distinct OpenMLS
KeyPackage hash reference. The latter is the existing Add/`V2JoinFacts` field;
it is not `blake3(raw KeyPackage)`. The actual SDK boundary must independently
compute/check BOTH against the same package. Full KeyPackage validation must bind that same key, group and invite
credential. Do not mint another package when a reply is lost. The request's
separate domain/version prevents reinterpreting its signed fields as Legacy.
Also retain the distinct MLS credential digest in the request and compare it to
the Add origin and actual package at the SDK boundary.

The signed **Welcome binding** names the same invite/request/KeyPackage/anchor,
actual governing Add Header and new Admission, exact Welcome bytes, both
KeyPackage bindings, original public routing-plan/policy commitments and a
SEPARATE commitment to the exact post-Add sealed routing-transfer bytes. The
sealed transfer cannot be frozen when minting the invite. Distinct field/hash
domains prevent its substitution or confusion with the public routing plan.
Its signer is the actual authorized admitting
device, which may differ from the inviter or forwarding endpoint. Verify its
authority through the independently pinned complete governing prefix and match
the genuine MLS outcome; a valid device signature alone is insufficient.
The DATA linkage check requires `AdmissionOrigin::Add`, matching subject,
KeyPackage reference and credential, and the exact Admission Header body/kind/
group/Profile/tenure. Welcome signer must equal that Header issuer. Current
Core requires the pinned Profile founder as governing issuer; an ordinary
member Admission does not grant it. Founder-Admission substitution is refused.

Codec framing is length-prefixed byte fields and big-endian integers from
`catcoms-wire`. Semantic domains are `mewtual/chat/join-invite/v2\0`,
`mewtual/chat/join-request/v2\0` and `mewtual/chat/join-welcome/v2\0`, each followed
by u16 version2. The common envelope contains its own distinct
`mewtual/chat/join-binding-envelope/v2\0` tag, semantic field and64-byte signature,
with exact EOF. Each complete envelope is capped at1024 bytes before parsing.
Semantic IDs hash `mewtual/chat/join-binding-id/v2\0` followed by the complete
semantic frame, so signature variants share an ID but never signature validity.

The complete anchor has its own tagged/versioned canonical encoding. Payload
commitments use distinct kind-specific `mewtual/chat/join-<kind>/v2\0` tags,
then u64 big-endian byte length and the EXACT bytes, under BLAKE3. Each body
commitment refuses empty or greater-than512KiB input before hashing. This is a
format/work cap, not a paid network/SDK resource grant. Public routing plan and
sealed routing transfer are different kinds. The OpenMLS KP reference and MLS
credential digest are supplied/checked in their existing SDK domains, never
computed with this wire-commitment helper. Expiry is checked when admitting;
an exact successful Welcome may legitimately arrive after it. Diagnostics must
not disclose bearer nonces, raw request/key material or routing bodies.

First implement these bindings as bounded DATA codecs/signature checks. Their
constructors/decoders expose no Source, Current, native credential or trusted-pin
constructor. The existing Legacy bytes remain unchanged. Do not register or
activate the V2 transport until the actual owning join vertical exists.

## Retained join request and private Welcome candidate

Sync now builds a JoinRequest from an explicitly accepted founder anchor and
one actual MLS KeyPackage. The request contains the exact wire digest,
OpenMLS reference and typed credential digest. The private provider snapshot
and original packet remain together for exact retry. The API leaves the
selected contact, mounted accepted pin and durable pending join to the host.

The wire frames use distinct length-prefixed
`mewtual/chat/join-request-packet/v2\0` and
`mewtual/chat/join-response-packet/v2\0` tags followed by u16 version 2.
The request carries the signed Invite, signed JoinRequest and exact
KeyPackage. The response carries the signed Welcome binding, the MLS Welcome
and the sealed routing transfer. Each field is length prefixed, separately
bounded and decoded with exact end of input. Signed binding verification
precedes use of request identifiers or payloads. The joiner charges the exact
response frame, including tags, length fields and the encoded binding, against
its original exchange limit before entering the Welcome SDK.

On the native founder, `check_received_join_request` verifies both request
signatures and the package wire commitment. It compares the complete
incoming Invite with the linked, verified signature recovered from the
original mounted journal. It does not accept a caller-supplied journal token.
The original Current and file checks bracket authentication. The paid read
gate then validates the exact MLS package, including its credential, MLS
reference, and lifetime against the founder's original clock. This keeps a
signed request with a malformed or expired package from reaching a future
durable reservation. SDK admission repeats validation at entry. The owning
transaction spends the nonce only through durable Add coverage.

The joiner checks the complete verified genesis anchor and the current
replay tip at its own Add. It validates the signed response and both
payloads before spending its one SDK attempt on a private provider restore.
The Welcome must select the original package and produce the same group,
subject, credential and epoch. The strict V2 routing decoder then compares
the original commitments, actual Add parent and epochs, exact policy and
fixed L0 slot. It verifies the policy against the actual joined MLS owner.
Exact completed retries reuse the private candidate after fresh prefix
checks. The candidate exposes public facts and has no Legacy conversion.
The native founder job now drives the retained Add through covered Head2 and
installation. It returns the original response only after a fresh check of
the selected head and archive. A later delivery of the exact same request
reuses the original response. Rejecting a different request still requires
the original nonce fence. Network delivery and the joined native owner
remain required for production.

The response carries a bounded public proof of the original genesis and
first Add. The joiner compares it with the accepted anchor and replays the
exact Admission and state before opening Welcome. A truncated or unrelated
proof leaves the original private KeyPackage available for a valid retry.

The private request owner can seal its exact KeyPackage provider and request
against the original pending join and joined pin. Reopening matches the
complete signed invite from that pin and checks that the retained provider
owns the same package.
An owning journal must still prove whether Welcome entry was possible before
resuming SDK work.
A successful Welcome's private graph, policy, routing secrets and verified
binding can be sealed to that same original request for later native
installation. The completed outcome reuses its original ciphertext on retry.
Its independent reader authenticates the original pending ciphertext, replays
the signed delivery, and checks the restored closed MLS graph and routing
transfer against it. Mounting and installing that graph require separate
native ownership.

The warm founder continues to use its existing ordinary-member lane after
installation. Before signing a new action it checks the selected Head2,
the exact archive and the same verifier's governing Header and epoch.

## Namespace6 and original pin custody

The [combined governance/native constructor](CHAT-V2-GOVERNED-NATIVE-STORAGE.md)
implements the startup-only Selected3 handoff before group or Source creation.
The combined handler requires namespaces 4, 5 and 7. The ordinary native3
handler still refuses governance4. Namespace6 now has a separate joined-pin
record and declaration grammar. The record binds the first signed invite and
pending join to the vault and root. Exact ciphertext identity keeps the
original pin stable across rejoin. The combined catalog parser preserves
namespace6 bytes in existing JOIN rewrites.

Namespace6 needs its own critical handler and versioned authenticated format.
Founder `GroupPinData` and pending Legacy `CapturedOwnerProfilePin` are different
provenance and cannot be relabelled. Store the accepted genesis anchor and its
accepted-invite provenance, bound to the actual vault/group/Profile and covered
original pending join. Invite-specific request/KP facts are separate from the
stable group pin. On same-Profile rejoin/reopen, reuse the SAME original pin
ciphertext and `V2GroupPinId`; resealing identical anchor plaintext changes the
stable outbox context and is forbidden.

The startup declaration prepares a seed over the exact predecessor and
candidate catalogs, their governing, outbox and native files, and the first
empty pin index. Its staging path holds those original files and preserves
them on refusal.
The explicit joined storage opener classifies this declaration before the
older inspectors and carries the same selected file through their checks.
It can finish an authenticated interrupted declaration or open an already
selected joined catalog. The warm governed owner can also promote the original
held declaration and move directly into joined-aware storage on the same vault
session. Its parser switch happens inside that exclusive owner. An equal-byte
replacement of the held Stage refuses, and retry takes the original file.
Fresh gated desktop startup moves that same original owner through the
joined declaration before publishing the native vault and its source collection.
The factory rechecks the governed selection around joined declaration preparation.
Warm promotion uses the exact native leaves already held by that declaration,
after retiring the predecessor's covered declaration and its read reservation.
An equal-byte replacement of a native leaf refuses promotion until the original
is returned.
On a process restart, gated desktop startup opens the original joined-aware vault
only through the existing-file cold path. It authenticates the critical wrapper
under the original lifetime and wrapper locks, and requires the selected native
registry and joined-pin index to be empty before installing empty Sources. It
can also reopen exactly one selected native joined row. Startup preserves and
refuses other nonempty states that lack a complete runtime factory.
The mounted index preserves the first pin ciphertext
for each group and checks the held index when reading pin context. The first
pending-JOIN record covers the private request and the one exact index
insertion. The original warm owner can select the insert and release the
exact signed request bytes while keeping the private package. Cold opening
can complete the same insertion and verify the original request. A selected
nonempty index refuses cold opening when its original Pending file is missing.
Restoring the original file preserves the selected pin and request. The warm
store can create and cover a first request directly from the caller's accepted
signed invite. It checks the anchor and mounted pin before creating the MLS
device, then retains the same request on an exact retry. The mounted native
Source collection can perform that transaction under its original UI command
and single metadata slot. It keeps the warm Pending when publication fails,
including when the index was selected on disk before a late file check refused
the result. This App operation returns the selected request commitment and pin
context. A second capture carries the covered request to the signed peer on an
already authenticated connection or through one explicitly approved direct IP.
The mounted request selects a revision-two Pending that encrypts a fresh local
transport seed and its derived TCP port. The Pending identity binds the public
transport commitment and port in addition to the exact request and predecessor
pin index. The warm and cold read paths return the same public reservation.
After a restart with an empty registry and neither Welcome file, an
authenticated desktop Join can resume the exact previously accepted invite
and request token. A separate native key purpose restores the original
private package from the held Pending. The desktop reuses the original
request bytes, TCP identity and port. An expired invite can only reach this
path as an exact retry of the already covered request. An unresolved join
prevents native Found on that vault. A selected native founder prevents the
vault from covering a first join.
An expired desktop preview authenticates the original Pending or registered
row in the unlocked vault. Preview leaves the transport closed until the user
approves a host. A fresh join still requires an unexpired invitation.
Revision-one requests keep their current read path. The mounted owner starts
one actual paid TCP actor from the covered seed and loans it to the original
request invocation. Cancelling before installation returns that actor for the
same request retry. The closed node takes that same actor at attachment, and
whole retirement waits for its tasks and physical custody to drain.
The connection capture releases the metadata slot before contacting the peer.
The actor checks the original invocation and UI generation, and the mounted
worker checks the files again before covering Welcome. The transport used to receive that Welcome can
move with its original closed graph into the same mounted collection. This
handoff preserves the original runtime inputs on early refusal and keeps an
already moved graph for an exact retry. Source retirement waits for admitted
installation work before disposing of that closed node. A new authenticated
invocation can recover the parked node's exact selection after losing a reply.
It reproves the original request token and the held Welcome files without
renewing or requiring the founder connection. Warm inspection and cold replay
derive the same registration facts from the selected private Pending, its
transport commitment, signed request and selected Add. These include the
actual joiner signer, pinned Profile and root, own Admission, governing Header
and state, and epoch. The native registry uses a distinct completed-joined row
tag. It appends only from the original parked node with the held Current,
joined-pin index, Pending, Entry and Outcome. The CAS rechecks those files
before selection. A cold opener proves the same registered facts before
recovering a staged row or consuming an already selected row. The gated
desktop Join command publishes that row from the same mounted Source. A retry
after registration observes that original row without dialing or entering
Welcome again. The warm joined row can take the ordinary member save path.
The explicit cold native storage owner can reopen that selected row with a TCP
actor built from its same covered seed and port. It attaches the independently
restored private graph after matching its receipt and governing replay. The
external transport constructor accepts the earlier external profile only.
The cold runtime can move the same physical vault, actor and closed graph into
a fresh native UI and Sources pair. Fresh authenticated inspection checks the
original registered row without another Welcome. Desktop startup installs that
pair and its prepared row together. The row keeps its registered group name,
numeric ID, request context and local display label. The older joined row tag
remains readable and uses the default local display label on cold reopen.

The warm owner verifies the delivery before covering possible Welcome entry, then
seals and covers the exact successful outcome. New outcome records also retain
the node page for the original Core candidate under the same Entry commitment.
Core selection follows coverage. Cold opening reconstructs that candidate from
the exact original signed delivery and verifies every retained node hash before
selecting. Version-one outcomes retain their original read path. Both the warm
successful graph and the independent cold graph prepare Core's initial selected-history candidate from the same
signed delivery. Each advances only after the mounted storage rechecks the
original pin and outcome, and retains the retired parent for fork evidence.
A mounted join uses separate native key cuts for first Device/KeyPackage work
and for the joiner's Welcome SDK entry. The prepared request remembers the
original Source allocation and requires that source and the actual UI counter
at the Welcome entry edge, after the signed response and public proof verify.
The existing founder response-signing purpose and the external trusted Rust
join path cannot enter that native SDK operation. Whole retirement closes the
original joined key counter before waiting for its admitted work.
A possible entry with no outcome stays
historical. An owning warm join can move the actual private Welcome graph and
its original transport into one closed joined node after selected history and
the physical files agree. Storage reserves the warm graph's own payment before
covering Welcome entry. The successful SDK operation attaches that payment to
the exact new group and device. It takes the original node's join facts and sets the
joined fence against the actual own Add. The mounted runtime retains the Core
selection and retired parent. A separate cold attachment can move the
independently restored graph into a closed node, using its own original
verified prefix and held catalog, pin and outcome files. Equal-byte replacements
of those files refuse, including after the closed node is attached. A prefix
refusal before attachment preserves the original transport, RNG and clock
inputs so that the same owner can retry. The cold attachment for a completed
Welcome uses the selected Outcome and restores the native row, transport and
ordinary-member save source.
Warm and recovered nodes can read the same original signed first governing
proof by its genesis and selected Add Header IDs. This bounded archive read
returns the original public proof from the covered delivery under a separate
buffer credit. The addressed member catch-up transport is still pending.
The joined storage owner can exchange the exact selected JoinRequest on an
already authenticated connection to the peer named by the signed invite.
For an explicitly approved direct IP, the trusted App caller can connect first
using the signed TCP port and complete peer identity from the covered invite.
One scheduler permit follows the exact dial into the transport, where stale
caller, vault or invocation ownership refuses submission. The caller waits for
the expected peer's authenticated handshake before exchanging the request.
Pasting the exact original invitation again with an approved IP can also
reconnect an already registered desktop row after restart. The selected row
and stored invitation are checked again before using the existing joined TCP
actor. The local row reply is independent of the dial outcome, and reconnect
only establishes the transport handshake for the existing membership.

The Sync reader for a later Welcome can verify a bounded sequence of signed
governing records from the original genesis through the Add named by that
Welcome. Each transition replays through Core, including the exact resulting
state. The separate later-delivery codec pairs the original signed response
with that proof under the current Join transport byte limit. A gap, reorder,
or different final Admission refuses before MLS entry.
The existing joined node can check a later Add against its own selected
Core prefix, the signed commit digest and the exact MLS group and epoch. Its
prepared successor leaves the installed member at the original epoch.
The original Core verifier records a valid signed successor before it checks
the Commit bytes, so an incomplete delivery still surfaces a signed fork.
The Sync founder can install the real later Add through its exact Core and
MLS handoff, retaining its original Admission and closing the old transport
observation. It can also prepare and sign the next invitation under the
original founder at the selected epoch. The pending intent records that
parent so a later selection cannot resume signing it. Its live Add owner records the consumed invitation nonce at
that handoff and refuses to spend it on a different request. The host still
owns the durable selection proof and must restore consumed nonces from the
covered journals before enabling later cold Adds.
The warm native writer covers a later Add under stage names bound to the exact
selected predecessor. It writes the newer archive and selector after checking
the original predecessor's physical files and the signed invitation journal.
The writer releases the complete signed Welcome after the new selector is
covered and the original Core and SDK are installed. An interrupted later
transaction blocks cold activation at its old Head. Existing members still
need native delivery and durable installation of the successor Commit.

The closed founder SDK can be reopened from a later Add snapshot using the
complete governing prefix and the original bootstrap genesis. It checks the
selected Header and Admission, the resulting state commitment and epoch, the
original founder key, the SDK authenticator, and the full active roster.
The native store checks the original physical archive selection for cold activation.
Cold recovery can answer an exact repeat from a predecessor archive. It matches
the original request bytes, checks the retained file identity and ciphertext,
then returns the Welcome and governing proof through that request's own Add.

A new member can verify a later Welcome against the complete prefix from its
original pin. It prepares the final Add for host selection and retains its
own Admission from that Add. The native joined journal writes a distinct
later-entry version and requires the exact new selected-history page on cold
open. The original first-entry version and legacy first outcome remain
readable. The three-vault regression exercises the connected later Welcome
and native registration of its new Admission.

An existing joined member can stage and merge the exact later Add on a closed
candidate graph while its live epoch remains unchanged. The SDK checks the
authenticated committer, one inline Add, the joining device and signed
KeyPackage and credential bindings, and the successor roster. It retains the
new graph and snapshot with the corresponding Core candidate. After the host
selects the exact signed Header, state and SDK authenticator, Sync installs
the pair under the original joined fence. The member keeps its original
Admission and the old transport observation retires. The warm and restored
graphs can stage the same Add with independent payment. The governing frame
keeps the signed Header readable ahead of the Admission and encrypted Commit
so damaged bodies still allow fork detection. Native TCP assigns this
exchange its own bounded request queue. Existing members derive the request
proof from their selected MLS epoch. The later Add journal stores a one-way
verifier bound to the signed parent and epoch. A recovered founder can use
that verifier to serve only the immediate successor from its authenticated
archive over the same native connection. Cold Core replay releases each
predecessor's retired model before staging the next authenticated step.
Earlier request records remain readable for exact Welcome retries. The
desktop founder dispatcher serves the governing lane with bounded background
work for both warm and recovered native rows. Joined Welcome recovery keeps
the original signed proof and registration while releasing its retired Core
model, so the existing member can stage the successor on that same verifier.
A reopened joined runtime can cover that exact received frame, the candidate's
selected-history page and its MLS epoch authenticator in an encrypted record.
Each record names the original pin and commits to the previous covered record.
The original Welcome remains the registration receipt. Startup replays the
contiguous chain on the original Core and MLS pair and refuses a missing
intermediate record or a mismatched authenticator. A desktop joined row can
fetch the next step on the original connected founder contact, then cover and
select it once existing local save work has reconciled. The joined vault first
covers the signed header under the exact encrypted predecessor, even when the
body is unavailable. A second authenticated sibling closes the live Core view,
and the covered pair refuses cold activation before either branch can advance.
Each new selected membership archive also has a separate encrypted selection
record that identifies its exact ciphertext and predecessor. A cold vault
refuses to activate an older epoch when that selection record remains and its
covered membership archive is missing or replaced. The old ordinary
subscription retires with the old epoch. On restart the original invite must
be supplied for an approved reconnect before the worker can fetch again.
The three-vault regression covers the third native vault's later Welcome,
the existing member's catch-up and direct TCP publication after the founder
actor drains. Joined members can share the contact code for their original TCP
listener and separately approve the destination IP in the desktop roster. The
receiver checks the signature against the exact active admission in its own
selected Profile, head and epoch before asking the existing dial scheduler for
one authenticated connection. It can use that route for governing catch-up if
the founder goes offline. Before sending an epoch proof it rechecks that the
approved route still belongs to an active admission at that epoch. A surviving
peer only serves a covered transition to
a connection that supplies the exact preceding epoch proof. Warm selection
and cold replay derive the verifier from the original preceding MLS graph.
The regression also exercises an old epoch, a changed signature, the local
member's own code, a cancelled request and an invalid governing proof.
Automatic discovery and the physical-device acceptance matrix still need work.

An independently reopened Pending with neither Welcome file can resend the
exact original request over the same connected protocol. It verifies the
reply against that request and the original held files before covering Entry
and checking the separate Welcome SDK permission. Cold startup with only a
covered Entry holds that state for inspection and refuses to re-enter Welcome.
The warm exchange keeps the original connection and vault cancellation through response
receipt and rechecks that authority before covering or entering the Welcome.
Revocation after covering Entry and before SDK entry leaves no Outcome.
A fresh authorization can retry the same covered Entry. The connected
fixture uses loopback TCP and the actual native founder responder. The request
kind selects separate transport size
limits before the decoder allocates either body. The founder stops upon
loss of the original response waiter. The queued reply retains the original
native session and Source checks and pays for its transport copies through
the lower send. Native TCP has a separate single-subscriber JoinRequest queue
for the original native session. Queue and reply ownership still follow the
ordinary transport budgets and authenticated connection. The native receiver
can retire without consuming the mesh's other events. The founder's selected
row carries a weak observation of its original mesh queue. App rechecks the
covered row and its actual Source when attaching a receiver. The gated
desktop path attaches after native founder publication and authenticated
unlock. Each connected request recaptures the original row and generation
before any founder work. Lock revokes the old receiver and any retained reply
authority. A later authenticated unlock can attach a fresh receiver for exact
retries. Legacy invitation commands are refused in the gated native mode
before any Legacy handler or transport work. Gated native preview verifies the
signed invite metadata, and the founder can mint its first covered invitation
through the native desktop command. The joiner reviews the invite and explicitly
approves a numeric IP address for the founder. The native scheduler uses the
signed port and peer identity, and the desktop publishes the closed joined node
only after covering Welcome and selecting the registered row. The first invite
profile carries no certified address, rendezvous, or switchboard routes.
The first history page can hold the native row table during admission or take
the mounted metadata slot as soon as Join replies. Joined worker startup
retries those conflicts. Both startup and dependency provider replacement
retry temporary contention when capturing the original TCP transport or
attaching to its dependency ingress. One worker group belongs to each original
row and authenticated generation. A membership transition revokes the old
dependency subscription and its pending replies. The worker then captures the
current row again and attaches a fresh subscription for the selected
membership. The three-vault regression holds the TCP capture mutex across the
first replacement attempt and recovers a missing original after release.
Native Found also retries a busy metadata slot at its current setup step.
Each retry rechecks the original vault and request. A storage refusal is
reported to the caller with the same request token available for a later retry.

Unknown/ambiguous remnants are preserved and refused, never cleared as debris.
The `joined-predecessor` CI scope builds the pinned governed reader from
`e0ff1033`. It must open the real governed control, refuse the staged and
selected joined-pin cases without changing their files, and leave both cases
available to the current joined-aware reader.
The governed Add fence gate runs the current fixture producer and the pinned
predecessor reader in separate Cargo targets with their own committed locks.
The gate records both lock hashes and checks that the predecessor preserves the
exact current fixture bytes.

## Founder operation and whole-node handoff

The MLS exact-receiver pair handoff and registered completed-result view are
implemented under final validation. They provide no Root save/Source/native
admission. [Ownership boundary and remaining integration](CHAT-V2-ADD-OWNERSHIP.md).

Retain one actual founder Add operation under its original request, selected
parent and native/Source custody. The existing independently paid
`PreparedTextCurrentAdd` candidate is an input, not covering save or Header
authority. Cache the genuine SDK outcome once. Retain separate actual governing
Header signing custody and cover the candidate SDK snapshot, invite ledger,
governing records and selected archive before exposing Welcome. A late refusal
or lost observer resumes the retained operation without another Add/signature.
Crash policy must distinguish unexposed work from any possibly published result.

Move the SAME original founder device/provider/transport/event ownership into a
closed governed runtime. No raw mutable SDK getter, runnable Legacy Server
conversion, second node or replacement transport is allowed. Preserve existing
native key admission, Source closure, SDK/physical drainage and payment order.
The live founder ordinary-member source selects the real governing prefix after
Add; it must not pretend that head is empty genesis. Legacy dispatch must not
write into this runtime.

## Joined source and durable selected history

Keep the source beside its one actual joined node in canonical Native Sources.
Use `take_v2_joined_join_facts` once on a genuine fresh Welcome. Replay the full
pinned prefix before the own Add, then match own Add to that exact consumed
KeyPackage/credential/Welcome. Postjoin authoring additionally follows supported
finalized local transitions, not numeric epoch equality alone. Restored nodes
need a separately reviewed cold reconstruction path; do not fabricate fresh facts.

Use the real `V2GeneralPrefixVerifier` selected-history append data and original
scope proofs. Retain selected records, contradictions and old-parent evidence in
durable paged storage before hot-cache eviction. Never reconstruct an opaque
known-parent witness from decoded IDs. A cache cap is not a lifetime-message cap;
missing/corrupt/busy backing is not authenticated absence. Neutral or multiple
MLS changes need an explicit reviewed rule; current refusal is not completion.

The closed node retains its actual Core Admission from the selected prefix.
Its final save fence admits a plan only when that plan belongs to the original
Admission and its original Core group and channel remain live. Warm and cold
plans can have equal action and Admission IDs and still belong to different
incarnations. The native joined writer uses this fence when issuing the
save-source end for its exact prepared outbox action.

The mounted joined row derives the existing member outbox context from its
exact original pin, selected Profile, initial tenure and full device key.
The warm receipt and independently verified cold receipt derive the same
context. The current epoch and Admission are separate from that identity.
The approved joined row can look up a saved request through that context.
An absent namespace stays absent. An ambiguous or damaged namespace refuses
and preserves the original files.
The desktop history route pages the original locally saved actions through
the same context, including after a cold reopen. For a founder the desktop
context comes from the original creation invocation in the covered row. The
vault also retains the request digest for verifying the original settings.
After a cold reopen, a lost founding reply can be recovered through the
original invocation and settings on that authenticated row. A conflicting
retry cannot create a new group or start the original network again.
A separate desktop view pages saved received messages through the held history
namespace. Each body replays the exact selected actions under the current
Core guard. A mounted history writer retains the exact partially written WAL
and its resource reservation. The next archive read or write completes the
original file before using that archive. The renderer serializes live reads
and retains their invalidations through transient admission contention.
Joined dependency repair is bounded to the original authenticated
carrier. The installed founder can recover its own saved direct dependencies
and show its saved received messages after the live receiver retires. The
desktop briefly retains an incoming founder frame or dependency reply through
temporary row capture and metadata contention. Each retry rechecks the same
mounted row and carrier. Cold founder startup restores ordinary member history
and its original listener from a covered Add. If no governing Add has entered,
startup authenticates the original promoted genesis and exposes its singleton
roster and saved local history. A founder restored at a covered Add or Remove
can save an invitation and recover that exact packet after restart. Its
native listener can also select a new Add for a fresh request. It checks the
same original invitation files and predecessor graph, covers the SDK and
both signing stages, and syncs the immutable archive and child Head before
installing the matching MLS and Core successors. The connected reply is
read from the authenticated selected archive. An exact retry in the same
runtime can continue its original covered work. Cold recovery before a
complete Head has been selected, same-vault rejoin, authoring before the
first governing Add, and automatic peer discovery remain open.

The warm joined node can draw and sign an ordinary action through the original
native Joiner Source. Its signing cut requires the ordinary member purpose and
retains the one result in the original Core plan. An independently reopened
graph binds a new native Source only after the original registered row and the
exact independently replayed Core Admission have been proved under a fresh
authenticated desktop session. The same save driver retains the action in the
shared outbox and issues its save-source end under the joined fence. A saved
request can be recovered before the restored Source draws or signs anything.

The actual joined fence orders every transition/removal against final issuance.
An admission that wins retains its original end through IO; removal that wins
prevents issuance. A1 saved/uncertain originals stay historical across A2 rejoin;
they cannot become new tokens or be silently discarded. Reuse the existing
two-phase native member save rather than adding a parallel writer.

The closed surviving-member MLS path can stage a private Commit containing
one inline Remove. It matches the exact named committer and target leaf and
checks that the remaining roster and local signer survive the merge. Core
derives the unsigned frozen cut from the selected admission slot. Sync pairs
the signed Core step and the closed SDK candidate under the existing joined
save fence and returns the original frozen admission with retirement custody.
The MLS committer can also build the exact private Remove once on a paid
closed candidate while the original graph stays selected. Surviving members
accept a bounded Remove frame through the same joined-row handoff as Add.
They observe the signed Header before reading its body or Commit, then cover
the exact Core history page and the verified MLS epoch in an encrypted
continuation of the original Welcome. Existing Add continuations retain their
encoding. A Remove uses its own record version, which older readers refuse.
The live founder binds its paid Remove and governing signature to distinct
native key cuts, the selected Core slot and the exact MLS parent. It can
swap the checked Core and MLS successors together after host selection.
The cold archive reader also follows mixed Add and Remove predecessors.
It checks the same named encrypted archive files, authenticates each signed
step and restores the founder SDK only when its roster and fingerprint agree
with the selected Core history. An existing Add keeps its original Welcome
reply and each Remove archive can serve its exact Commit to holders of the
preceding epoch's proof. The evicted member can authenticate its own Remove
without merging or deriving successor keys. Sync can select the original
frozen Core admission and close the same fence that admits final saves.
The joined vault covers that terminal cut on its original Welcome file
chain. The record retains the original Commit digest and frozen history
page. Recovery authenticates that cut before refusing to activate that same
old registration, and live application retires the original row and SDK.
The desktop reports removal on that original row and retires its composer.
The selected transition reports its terminal outcome directly. Local worker
shutdown leaves the membership notification live until it can reach the
original mounted row, subject to the desktop session's lifetime.
The founder refreshes its open roster after answering a covered Join.
An exact draft or saved retry survives eviction. The renderer stops dispatching
new requests for that exact context, even when removal arrives during the local
retry journal write.
The warm founder covers the exact Remove intent and SDK entry before SDK
work, then covers the original SDK outcome and Header attempt before signing.
Add and Remove share the exact-file Head replacement under the original
Current writer. A lost reply after Head replacement can finish the same
cached Core and MLS selection. A local refusal before SDK or header signature
entry can retry the same covered work with a live cut on the original source.
The desktop founder chooses the exact admission shown by the selected roster.
Invitation, roster, removal and contact commands require the renderer's
original context.
The native row checks permission again before issuing the operation. A surviving
joined row and a reopened founder read the roster from their authenticated
selected history.
A cold restart during an incomplete governing transaction still needs recovery
work.
Add and Remove check the same membership epoch bound as cold replay before
covering a successor intent. Both writers can select epoch 64.

## Acceptance before enabling

The Chat V2 integration workflow runs focused desktop membership checks on
Windows and Ubuntu and a separate renderer job on relevant pull requests.
Select `all` and `run_deferred_catchup=true` when dispatching it for full
acceptance. This includes the deferred long-running catch-up regression.

Require deterministic tests for stripped/downgraded/cross-root bindings, exact
lost-Welcome/KP retries, every Add/pin/archive crash cut, both removal/admission
orderings, stale A1/A2/A3 tokens, delayed old-parent siblings beyond4096 records,
missing/corrupt archive pages, persisted contradictions and actual cold reopen.
Use at least three real peers for founder-offline delivery, then the planned
20–50-member bubble/epoch/restart/load matrix. Physical multi-device evidence is
separate. No newly decoded DATA or in-memory mock can stand in for these results.
Archived replies across a removal still need the exact historical admission
and its frozen cut before production enablement.
See [selected history proofs](CHAT-V2-SELECTED-HISTORY-PROOFS.md).
## Remembered joined contacts

Members can opt to remember a signed contact for the original joined row.
The encrypted local record keeps the exact approved contact and its covered
governing Header and epoch. An address is reusable when the native actor
proves that exact numeric outbound route authenticated the expected peer.
An incoming connection can support a passive approval without supplying a
future dial address.

On cold restart the original joined row verifies the saved signature and
live Admission at the actual selected head before attempting a route. Both
peers need an authenticated live connection before history recovery starts.
**Forget saved contacts** clears the disk record and revokes pending restored
work. A later governing head needs a fresh contact approval for cold startup.
