# Scalable owner evidence and retained action archive

Proposed production successor; independent format/accounting review REQUIRED before codec or
persistence implementation. The existing owner-fact/v1 CHECKPOINT-1 remains a nonactivatable
producer test profile. Its 64 lifetime admissions, 4,096-action removal list, 64 MiB archive and
per-action future reservations cannot be the endpoint for long-lived 20/50-person groups.
The first real factory/canonical transaction continues in parallel; it must not activate typed
publication using that checkpoint's availability limits.

## Authority and compatibility

Keep action/v1 signing bytes and semantic ActionIDs unchanged. Introduce a separate closed
owner-evidence/v2 profile/record grammar; never reinterpret v1 signed bytes. The expected immutable
(group, profile, original finalizer full key) pin comes from the actual private founder or the
explicit trusted captured founder handoff BEFORE the real fresh-KP join. A downloaded Profile,
current roster, numeric server ID or local vault scope cannot select its own trusted root.

The scalable successor requires authenticated top-level VAULT5 with its own KDF domain BEFORE
opening any owner-state stage/journal cleaner, Actor or network path. VAULT4 checkpoint readers
must reject it at authenticated vault framing, even if a captured older canonical v1 file exists;
strict inner-tail rejection alone is too late because v4 startup already cleans fixed stages.
Unsupported format is never interpreted as missing/createable. Covering3/4->5 upgrade preserves all
scope Current/Retired/Initializing rows, immutable pins, exact receipts/publication aliases and
uncertain debts. A failed covering upgrade leaves every prior sealed byte/obligation intact or
explicitly latched for exact repair; no compatibility downgrade can launder debt through
stop_obsolete(None). New v2 issuance starts only from actual fresh provenance. Existing legacy
retained originals remain readable as legacy/unproven, without retroactive typed authority.
Regress old-v4 refusal BEFORE any stage cleanup and faulted upgrade with fresh-reopen classification.

The initial policy remains the explicitly trusted non-equivocating owner finalizer and
admin recommendations. MLS alone does not prove unique public finality. The finalizer's
public transition attestations are not independent public replay of private MLS state or a
physical-time proof. Unsupported succession stays pending; fixed-founder staging is not
completion of owner succession. A later selected Reject cannot retract genuine finalized
approval: a separate authorized Delete removes chat content; rejection reopening is explicit.

Every supported public owner transition has a bounded signed header: exact group/profile/full
issuer, original owner-tenure identity, parent header CID, sequence u64, closed kind, body CID,
and resulting public-state commitment. Header plus signature is at most 4 KiB, independently of
historical action count. HeaderCID hashes domain-separated canonical **UNSIGNED semantics**;
verify every signature before use. Genuine alternate signature representations over identical
semantics are equivalent duplicates, never contradictory children.

A fork is two distinct semantic HeaderCIDs, both valid under the SAME externally pinned group,
profile, full issuer and supported tenure, naming the SAME known parent and checked successor
sequence. Closed kind/body commitment grammar still applies; signature-invalid, foreign-profile,
unknown-tenure or malformed headers cannot poison that parent. Once these header checks pass,
missing or invalid body bytes cannot erase signed conflicting commitments: retain the bounded
headers in prepaid contradiction escrow and mark the affected selected suffix breached/pending.
A positive effect requires the exact body, its semantic dependencies and resulting-state commitment
to verify independently. Do not choose by timestamp, CID or first arrival. Unknown parent references
are incomplete, not a free unbounded approval/fork cache. The separate bounded
[successor-header codec](CHAT-OWNER-V2-HEADER-FORMAT.md) is implemented but uncompiled/unexecuted;
Profile/genesis, exact body/state codecs and the actual pinned governing consumer remain gates.

Equal verified evidence unions use the same common nonconflicting prefix on a detected finalizer
fork. No replica-local already-final exception is allowed. Previously exposed plaintext cannot be
made secret again; convergence/finality claims remain conditional on the signed trust assumption.
Ordinary message branches continue to merge automatically and retain history, without user repair.

## Owner-offline ordinary authority

The v2 Profile explicitly delegates ONLY ordinary MEMBER rights (CREATE, SELF_EDIT, SELF_DELETE,
JOIN) to an exact owner-certified Admission incarnation, including member-created realms. This
matches today's text baseline: app Role is Owner/Admin/Member, ordinary create_channel/send do not
have a durable text mute/read-only policy. It does not delegate admin/moderator decision authority
or DELETE_ANY. A future text restriction needs its own verified signed cut/reopen projection policy;
a raw current-role lookup cannot rewrite historical semantic authority.

A v2 Admission signs these ordinary rights and the closed all-member-created-realms rule along
with full subject and actual Founder/Add/KP incarnation. Derive IDs with catcoms-wire fixed integer
and length-prefixed field grammar; H is BLAKE3 of the complete following frame:

- GrantID: put_bytes("mewtual/chat/ordinary-grant/v2\0"), put_bytes(group),
  put_bytes(ProfileCID32), put_bytes(AdmissionCID32), put_bytes(fullSubject32),
  put_bytes(RealmID32), put_u16(MEMBER.bits()), put_u8(1 /*member-created realms*/).
- BasisID: put_bytes("mewtual/chat/ordinary-basis/v2\0"), the SAME group/profile/admission/
  full subject/realm fields, put_bytes(derivedGrantID32), put_u16(MEMBER.bits()), put_u8(1).

The expected immutable profile must select this exact v2 delegation profile. Never reinterpret a
v1 RealmGrant/Basis as delegated authority. Action/v1 bytes and ActionID stay unchanged: their signed
grant/basis labels commit these exact derived facts. Stable BasisIDs do not contain signed parent
coverage, so two members need no cyclic owner refresh and an offline new channel/reply needs no
per-post owner signature. Same-key A1 and A2 have distinct AdmissionCIDs and derived IDs.

The verifier computes required coverage from the ACTUAL action's exact parent/target records and
their verified same-realm bases, not an arbitrary caller-supplied list of admissions. Missing exact
action/parent/target/chain evidence remains Pending. Supply the canonical minimal unique set of
foreign bases needed by <=64 parents plus an explicit edit/delete/reply target (at most65); reject
extras/duplicates/foreign realm/profile and verify every selected original Admission chain under
the SAME mounted expected profile/full founder. No recursive unbounded basis expansion. Construct
only that per-ACTION ordinary witness.covered set; repeated proof variants normalize identically
and cannot grow it through merge. Existing semantic DAG/dependency checks still run, and historical
target authority never releases a held target body. Cross-author Delete still requires ordinary
SELF_DELETE plus its exact action-specific approval. Neither delegated semantics nor old A1 proof
establishes accepted-before-removal admission for a late action.

## Acceptance log and bounded removal cuts

For each cryptographic Admission incarnation retain an append-only accepted-action log. A member's
same-key rejoin creates a distinct owner-certified Admission from its actual fresh selected KP/Add;
GrantID/BasisID commit that Admission. Old A1 actions cannot borrow A2's current evidence.

A log leaf commits profile, Admission, channel and the exact semantic Acceptance fact CID, whose
body binds ActionID/full signer/Grant/Basis/Create as applicable. Equivalent valid action signature
representations therefore share acceptance; changing nonce/body/realm/target cannot borrow it.
Append bodies commit old tip CID/count/root, at most64 sorted unique exact acceptance leaves,
and new count/root. The signed body does NOT embed the old64-hash frontier: retaining that on every
append would reintroduce O(N log N) retained proof bytes. This sorting is canonical encoding, not an
authority tie-break. An Append body has its own measured bound (at most8KiB proposed, not the4KiB
header/removal bound). The global header chain orders actual accepted transitions under the pinned
non-equivocating-finalizer assumption. Exact canonical request/action acceptance is idempotent across
batches, retries and restart: an already accepted leaf returns its retained original result, never a
second index, and a changed binding cannot borrow that result. The bounded derived lookup index
must prove absence/identity against the committed log before a new append; rebuilding is automatic.


The signed log tip is (profile, Admission, count u64, root32). Define the binary tree canonically:
empty H(domain-empty || context), leaf H(domain-leaf || context || leaf), and internal
H(domain-node || left32 || right32), with distinct domains. For n > 1 split at the largest power
of two STRICTLY LESS than n. Context includes the exact profile and Admission. Inclusion proofs
bind index/count, consume the exact path derived from that pair, permit at most 64 siblings and
reject trailing/short/overlong paths. Append/frontier and historical-prefix proofs need the same
canonical count-aware verifier; a root label alone is never acceptance. Frontier/consistency
witnesses are separately bounded UNSIGNED derived exports authenticated against signed old/new roots,
reconstructible from retained O(N) node pages rather than retained per append. For checked n+k<=u64::MAX,
a batch of k leaves creates exactly popcount(n)+k-popcount(n+k) internal nodes plus k leaves; charge
that count before append. For k<=64 the conservative internal-node cap127 is safe (actual max120 for
k64). Pin power-of-two/u64 boundary golden vectors, not a claim that one append costs only64 nodes.

A Removal body commits the actual observed Remove/epochs, exact Admission/full subject, and the
frozen accepted-log tip. Its encoded size stays <=4 KiB regardless of whether 1 or 1,000,000 actions
were accepted. The body and header are saved with the actual MLS transition before export. An
append beyond a frozen cut cannot silently release late old-grant actions; it needs an exact review
approval, or is conflicting owner-transition evidence under the declared chain policy.

No whole accepted set is materialized for cut creation, import or held-post lookup. Verification
loads bounded pages/proofs. The current/provisional view and semantic DAG remain separate:
approved Edit does not admit a held Create; held Delete is excluded from effective projection;
causal Join/reply knowledge never releases an ancestor body. Prior verified Delete proof loss
suppresses only verifier-produced prior-effect custody, never an attacker's unapproved Delete.

## Cold archive, current state and publication order

Retain original signed actions, all edits/deletes/branches, admissions, grants, basis records,
acceptance batches, cuts, decisions and contradiction headers in immutable sealed pages. Target
page plaintext is at most 64 KiB; large raw actions use bounded exact chunks with a committed
manifest, retaining the original bytes without partial authority. These files are private and
MUST NOT be reachable through network BlobStore paths. Public proofs are separately bounded
exports verified against the selected pinned header; private provider/server bytes never export.

The canonical owner snapshot contains bounded hot state plus committed cold manifest/tip roots,
not another complete copy of the cold archive. New immutable pages are written/file-synced and
their parent synced BEFORE the covering canonical head references them. The head transaction
also retains exact retry/publication obligations; no action/MLS effect/certificate/Welcome is
released before the covering save AND original admission/head custody succeed. Replaced-not-durable
retains the entire candidate and fences every alias until exact repair. Restore re-syncs validated
current canonical bytes and their child directory before publication.

Restore validates the EXACT reachable manifest/page closure before export, not only the canonical
head and its parent directory. Every referenced file's bounded authenticated bytes and required
file/child-directory durability barrier must be satisfied. Missing/corrupt child pages remain
incomplete/unbound; automatic bounded repair may recover exact journal/replica bytes, but may not
invent an empty page, reconstruct signed bodies from current projections, or claim unavailable data
was synced. No actor/publication starts while this closure is unresolved in the initial conservative mode.
This trades restart latency/availability for a complete retained source: even a missing audit-only
body initially blocks chat startup. Large-archive reopen and bounded partial repair must measure that
cost. A later independently reviewed essential-proof vs audit-only availability split may let healthy
chat run while the admin ledger truthfully stays incomplete; it may never bypass unresolved admission,
prior-effective Delete suppression, retained Stops or required authority/publication debt. The final format must
make both manifest reachability and repair-obligation charges mechanically checkable.

One fixed prepaid operation journal per head identifies exact unpublished stage/page candidates.
Crash cleanup never deletes a committed page/current head or uncertain candidate. Stage/page count
and bytes are charged before allocation/creation, including orphan recovery; repeated restart cannot
accumulate unbounded extras. The exact journal/page grammar and max per-operation page count are
format-review gates, not permission for generic recursive deletion.

Use append-only Merkle node pages (O(N) retained hashes) and a disposable bounded lookup index,
not persistent full path-copy trees which silently charge O(N log N) historical versions. Derived
index compaction may discard ONLY reconstructible index data. Lost/corrupt indices rebuild
automatically in bounded turns and report incomplete inventory, never an empty complete ledger.
The final index/page/fanout/work fees must be fixed before production activation.

## Accounting and scale gates

Separate current active slots, retained historical bytes, hot buffers and turn work. A 64 CURRENT
member cap can cover the 20/50-person target with staging margin; removed/rejoined historical
Admissions do not permanently consume those current slots. Their signed facts/cuts remain in cold
storage and consume the explicit configured persistent quota. No historical proof is evicted to
recover capacity. Local authoring-scope lineage limits remain separately accounted and reviewed.

Reserve constant mandatory control capacity BEFORE each new current Admission: removal body+
header, one contradictory pair, terminal/index/receipt and negative-resolution work. Exact sizes,
rows, buffer fees and control slots follow the final <=4 KiB header/body codecs; the proposed
former rough suggestion of40KiB/10records is WITHDRAWN as an insufficient physical invoice.
Before v2 codec constants, specify measured header/body/node/page/manifest/journal fields and exact
public-state commitment/append consistency grammar. Charge k leaves + the exact internal-node formula,
up to127 new internal nodes, both conflicting signed children, page/manifest fanout updates, fixed
journal and receipt records, AEAD nonce/tag/frame overhead, directory entries, one bounded replacement/
orphan allowance, control sequence slots, and separately bounded transient proof export/scratch.
No unaccounted "small metadata" row or one64-node carry can spend removal/contradiction reserves.
This full invoice remains an explicit format implementation gate, not an accepted reserve constant.
Ordinary action/recommendation saturation must never refuse a governing cut/retirement/finality
proof and leave provisional effects active. When retirement/required publication is durably complete,
release only UNUSED future reservation; consumed signed records remain charged and retained.

Sequence/count exhaustion is prepaid too. Before any ordinary signed owner transition, require
that checked remaining u64 owner-header sequence space still covers every already-issued mandatory
removal/retirement/review control obligation after consuming this transition. Do not wrap or saturate
an exhausted successor. A full acceptance-log count rejects new ordinary leaves; Removal needs no
new acceptance leaf. Admission/control fees must reserve their required sequence slots in addition
to bytes/rows/work before issuance. Counter exhaustion may not strand an existing prepaid cut.

Acceptance batches pay actual stored bytes and bounded append/proof work. Do not reserve 8 KiB
forever per accepted action. A durable review intent separately prepays its exact signed successor,
negative abandon terminal, retry result and publication debt before first dispatch. UI continuity v3
is now recovery format; critical review custody requires v4 or a separately sealed old-writer-safe
journal. It cannot be placed in an ignored field or coerced into the closed chat Stop grammar.

Configured cold storage is finite: exhaustion refuses NEW ordinary work before signing, with
existing history intact and mandatory prepaid controls usable. It is not a 4,096-action or 64-lifetime
admission cliff. Hot reader cache, crypto work, page reads and outstanding operations have separate
fixed caps; page count/accepted history may exceed those hot caps. The bounded CHECKPOINT-1 whole
snapshot has a different 144 MiB limit and must remain explicitly nonactivatable.

## Required executable progression and tests

The first consumed transaction uses the actual private factory, original scope reservation,
immutable pin and canonical saved-head CAS. It produces real founder facts, then the SAME path
handles actual fresh-KP Add, member signature, exact acceptance, actual Remove, held unproven
second action, privileged admin detail, signed review/abandon and fresh restore. It is not complete
when only the founder checkpoint works. The full retained ledger reader includes ordinary actions,
not only cases; typed ActionRef and CaseRef stay distinct and legacy originals stay unproven.

Before activation exercise owner-disconnected20/50-member new-channel and later-member reply/
edit/delete/<=64-parent causal-join bubbles, including targets from >64 historical admissions,
missing/invalid/foreign target evidence and same-key A1/A2 IDs. No per-post owner refresh is allowed.
Execute >4,096 accepted actions, >64 lifetime Remove/rejoin incarnations and
20/50 current members under small hot budgets; cut/proof frames stay bounded. Cover opposite
bubbles/orders, same-key A1/A2, exact/foreign proofs, invalid-first and missing-body forks, proof
loss/recovery, saturated ordinary bytes/rows with prepaid removal+contradiction, interrupted page
writes/rename/parent-sync, reply loss, two aliases/CAS, stale frozen candidate, automatic index
rebuild and fresh restart. No ordinary conflict requires a user repair choice.

Canonical storage prevents stale API/numeric-alias writes within its original mount. Without an
independent latest-head freshness anchor it cannot detect replacement of the canonical file ALONE
with an older authenticated same-current-admission copy after restart; this is component rollback,
not just a whole-vault backup rollback. Pin/scope files need not be replaced for that residual.
Do not claim a monotone external witness or hide it behind a locally saved authority boolean.
