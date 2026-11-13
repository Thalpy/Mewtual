# Native index2 storage

Status: implementation in progress on `chat-v2-native-index-admission`, following
PR96. This is a storage increment. Chat V2 stays disabled. No native create/send,
SDK/key permission, joined source or operational restart is established here.

Namespace7/version2 uses a distinct index2 grammar, descriptor and key domains.
The descriptor commits through the complete canonical version1 descriptor to the
validated full Current3 descriptor. Namespace6 remains reserved for joined pins.
Version1 APIs and grammars remain closed.

`NativeIndexUpgrade` consumes a genuine opaque `NativeRegistryStore`. The only
supported upgrade input is a settled, Legacy-only generation1 index with no JOIN
rows. It preserves every legacy numeric ID, stable identity, name, invite and DM
flag. Preparation changes no files. Failure during preparation drops the private
mount, which can be reopened. Once prepared, retries retain the exact encrypted
frames and actual file identities; they draw no randomness.

The cached intent contains both complete catalogs, both indexes and both markers.
Its fixed seed is `chat-critical-namespaces.native-index2.seed`; the two temporary
outputs are `native-index2.upgrade-index` and `native-index2.upgrade-marker` under
the existing private root. Upgrade first durably claims the seed name and an
**empty old catalog Stage**, then completes the authenticated intent. It writes
and syncs the two new temporary outputs, replaces index/marker, and selects the
new catalog only afterward. Selection bits are cached before fallible covering
barriers, so they do not prove those barriers completed. Every preselection retry,
including cold mixed-tuple recovery, flushes the exact selected index and marker
handles and their held directory again, then re-proves intent and tuple before
writing any nonempty catalog Stage. Final post-selection barriers remain required.
The same held files and complete tuple are re-proved after large reads,
immediately before each replacement and before intent cleanup. Conversion rechecks
the tuple around private-handler installation.

Cold recovery authenticates the whole relation before writing. Once catalog2 is
selected, index2 and marker2 must both exist and be exact; the seed cannot recreate
a missing committed output. Authenticated empty, malformed or future intent is
preserved. An unauthenticated first intent can retire only its empty fence/claim,
with an authenticated unchanged v1 G1 tuple and no new outputs. The v2-only opener
then refuses the retained v1; an explicit fresh upgrade may proceed from it. No
SDK or source grant is recovered from that cleanup.

Index2 retains an immutable birth receipt in Pending, Founder, Retired and
Abandoned: invocation, original canonical-request digest, full signer and funded
network-identity commitment. Pending also retains the exact bounded initial
network ciphertext. Receipt lookup conflicts on edited inputs and still finds a
terminal birth after unrelated index updates. Terminal rows are not recycled.
The monotone last-issued ID cannot decrease. Generation1 accepts only an empty
or migrated Legacy mapping with its exact initial high-water mark.

Structural successors require the actual preceding framed-ciphertext hash,
generation+1, unchanged unrelated rows and a closed Birth/Activate/Retire/Abandon
transition. These checks are DATA validation. The future owning producer must
also supply actual network, journal/source and native evidence; arbitrary valid
rows or historical status can never install a backend or start an SDK.

`NativeIndexStore` exposes only opaque open/count/receipt DATA queries and a
negative whole-vault admission close. It supplies no Store/Current getter,
callback, native command credential or runnable V2 owner. Its single read guard
prevents overlapping consumers from reusing one invoice.

The separate index Stage supports exact authenticated next-generation recovery
and duplicate cleanup. Unauthenticated scratch cleanup retains the same original
phase after unlink, caching removal before a failed directory barrier. A catalog
Stage alone remains ordinary supported JOIN recovery; the private gate validates
its unchanged native descriptor and leaves settlement to the JOIN opener. Dual
index/catalog Stages preserve/refuse before either cleaner.

The actual shared handler has a negative index/JOIN interlock. Both Pending
placements check it under the catalog mutex before ownership moves; the common
physical candidate and existing-result settlement paths check it without taking
that mutex again. An affine serial claim cannot clear a successor or reopen a
closed owner. The actual warm transaction cell/producer that claims this interlock
is **not implemented yet**. It must bind actual mounted.current, selected index
and marker before effects, retain claim/candidate/files/payment across caller loss
and uncertainty, and release only after covering cleanup or proven no-effect
disposal. General index claims must preserve existing JOIN history; the zero-row
restriction belongs solely to the initial upgrade.

Payment covers ten catalog frames, twelve index frames, twelve marker frames,
four complete upgrade intents, explicit row metadata and control/path overlap.
The row allowance has an ABI guard for four maximum-size vectors of each row
schema and eight fixed hash-scratch vectors; the control allowance also has a
layout guard. These are logical quotas, not allocator/RSS or disk-space promises.
Fields retire admission first, then files/buffers, then the mount, then payment.
The interlock and retained claim keep the exact debit alive; copying a pool handle
or claim label cannot replace it.

The next native-founder increment separates that affine admission owner from a
private reference-counted physical allocation containing the reader, Current and
handler. Only the opaque facade owns close; a future admitted worker may retain
physical custody without cloning authority. No public physical loan exists, and
retained cells must not contain an Arc back to their physical owner. Index DATA
completion remains possible after admission closes; central JOIN helpers still
reject CLOSED, so this does not claim every admitted JOIN repair drains after
owner removal. There are no operational source/command issuers here yet; the
native founder consumer must retire those before waiting for worker drainage.

Net now derives bounded public identity DATA from its actual Ed25519 key builder:
the full algorithm-tagged protobuf key, canonical libp2p and Phase-0 IDs, and a
domain-separated full-key commitment. A literal compatibility vector and real
Mesh ownership tests cover it. This helper borrows the original seed; it holds
no private-key/runtime permission and does not reserve or persist a network file.
The follow-up passes all32 index2 cases and3 identity cases locally, including
actual retained Stage/post-rename completion after facade close. Read-only review
has no blocker/high/medium. Full current-source gates are still required.

Local Windows execution passes30 exact cases, including14 real warm cuts,11
complete-intent cold cuts, first-torn recovery, authenticated-empty/future intent
and index Stage preservation, committed-output loss, a JOIN-compatible duplicate
catalog Stage versus dual Stages (not a real v2 JOIN lifecycle),
failed-unlink retention, four late equal-byte destination replacements, and exact
real-owner file/buffer/mount/payment retirement. The earlier surrounding cohort
passed62 cases before the later physical work. Actual-diff review corrected a
drop-order HIGH and a late-target replacement MEDIUM; the regression cases pass.
The actual predecessor-v2 gate passes locally:18 head-generated fixtures,72
opener verdicts in unmodified382a30a0, and18 head-reader round trips. The old
reader's verdict table is compiled into its one added integration-test file;
fixture metadata cannot select the expected outcome. Positive controls require
its genuine Current5/v1 opens and acceptance of a deliberately unfenced complete
intent. Both accepted and refused copies retain their exact file trees. This is
compatibility evidence, not native runtime or crash-custody acceptance.

Raw-fs and ambient gates and exact-marker self-checks pass. Read-only review has
no remaining blocker/high/medium. Complete current-source suites and strict
Clippy remain pending.

Still required: the retained warm index cell and genuine native founder/network
producer; concrete synchronous command and SDK/Profile/Header/member key cuts;
opaque mounted-vault/native backend installation; cold operational owner and
durable Leave; legacy projection/rename; joined trusted pins/Add/archive/transport;
UI/admin and combined20/50-member acceptance. Storage DATA supplies none of those
permissions or outcomes.
