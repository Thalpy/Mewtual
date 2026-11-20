# Original text V2 invite preparation

Status (20 November 2026): unsigned production preparation now has an initial
immutable intent journal, under hosted validation. The separate Sync key tail
is still unwired. Chat V2 is OFF. Durable KeyAttempt/raw-signature coverage,
SavedInvite issuance, a native invitation command, advertised contacts and
working V2 join remain unfinished.

## Initial durable intent profile

The real `NativeTextMemberJob::prepare_invite_intent` now looks up its exact
token/settings/public plan before asking the retained node for a new unsigned
original. The new closed `text.pages/invite.intent.v1` family stores that
original preimage and plan under a separate AEAD key domain. Its context is
derived from the actual original source encoding, genesis frame, scope nonce
and held pin. Current4, the selected G1/member parent and native command are
checked around IO; no caller path, context digest or signing callback enters.

This initial profile reserves **one object and 8 KiB per context**, separately
from Head1 counters. Its authenticated format fixes both limits. It never
replaces, evicts or deletes the object, and refuses another token/settings.
The original native-only warm Host owns the affine initialization opportunity;
there is no cold/library adopting constructor. Missing after creation,
malformed/unknown/torn quota or format and replacement directories/files are
errors. A cold recovery consumer must require the existing journal and cannot
interpret absence as an empty quota or recreate that initialization opportunity.
Wider history/retention needs a separately versioned producer.

The Host pays a named independent memory invoice before allocating the journal.
Its actual pages/file handles drain before the retained vault physical-work
lease. All reads/writes stay inside the same synchronous RuntimeLoan, so no
detached writer or global text-lane registration is introduced. AEAD candidate
preparation and its RNG run outside Current/Root writers. The candidate is
cached before create/write; the same warm owner can complete only its own
exact partial prefix. Unknown create entry is held. File and directory barriers
and exact identity/byte checks precede covered unsigned success.

Returned hash/expiry remain DATA, not SavedInvite or permission to sign/Add.
No production key edge is wired in this increment. The new regression set
covers strict codec/quota parsing, warm partial writes, changed retries,
missing/corrupt/unknown state, late cancellation, ordinary-member coexistence
and actual predecessor save/physical-retirement preservation of opaque sibling
bytes. Existing Current4/refusal tests remain. Formatting and the static Store
raw-fs gate pass locally; compilation and runtime results await hosted CI.

## Actual owning path

`NativeTextMemberJob::prepare_invite_intent` reuses the original paid native
member-operation capture. It receives only a token, requested lifetime and
cancellation; the renderer cannot provide a pin, route hash, policy, clock,
nonce, native Source observation or signer. The whole RuntimeLoan remains
outside native/Root writers and is restored before outer key metadata drains.

The actual retained transport Reservation derives the route commitment from
the driver's original group, full36-byte transport public key, complete peer
ID, transport-key commitment and fixed TCP port. The native row identity is
independent randomness: it must match the active/pending row, while the
transport commitment must separately match the immutable Birth. Both share
the same original Current vault/root. Configured advertise, relay and
rendezvous strings do not prove external reachability and are excluded. The
versioned plan explicitly contains zero certified contact addresses. The Host
retains the exact bounded plan bytes alongside its commitment, verifies their
domain-separated hash before copying, and compares both on retry. A later
journal must use those bytes instead of reconstructing them from transport
state. Hashing this reservation does not make it a dialable route.

SourceWork forwards through the same fresh driver, Host and warm whole node.
The Host requires NativeRequired origin and the constructor-selected steady
governed-native family. Its existing member lane supplies its ONE verifier;
no second prefix/domain is minted. Before and after the retained draw it
checks the live founder Admission, genesis MLS epoch/header, same selected G1
head, original source and actual mounted4 files under the already-held Root.
Plain native3, GovernanceOpening and cold/repaired owners cannot prepare here.

One independently paid Host slot retains the exact request and Sync plan.
The output is only `(unsigned preimage hash, expiry)` DATA. It cannot be used
as a signed invite, covering receipt, Add admission or save acknowledgement.
Ordinary member preparation continues on the same verifier after this step.

## Retained draw and key boundary

Sync freezes the original seed/native observations and every request field
before its Clock/RNG seams. Root's request token differs from the random wire
nonce. The same node derives the complete original genesis anchor and hashes
the exact supported `GroupPolicy::encode()` body. The initial lifetime bound
is seven days. Clock and RNG entry markers precede invocation; expiry/nonce
are cached once, and interrupted unknown seams cannot redraw on retry.

`InviteBindingSpecData::signing_bytes_data` gives the exact canonical unsigned
preimage. Full SDK/source preflight runs outside final writers. The separate
key tail performs only exact-seed, stable original-life/scope and retained-key
checks before the distinct `JoinInvite` native purpose (new local slot8).
Slots0 through7 keep their previous meaning. Root must cover the intent and
establish actual current/native authority before calling that tail; the
current production preparation never calls it.

Only an actual returned pre-effect Busy/Unavailable may retry with a fresh
same-source cut. The owner must dispose any old partial UI End outside writers;
attempts stay spent. Native admission panic or entered key with no raw result
remains terminal. Raw64 is cached before late checks. Materialization uses
only the retained raw, so cancellation, lock or a lost return cannot re-sign.
Historical getters/materialization remain DATA after revocation.

## Remaining signing and publication producer

The initial unsigned journal above establishes only the first barrier. The
paired signing producer must preserve its separate family, independent quotas
and memory invoice, and ordinary Head1 inventories/accounting. The immutable
v1 intent cannot be silently reinterpreted as a key attempt or signed result.

For the current private producer it can use an isolated affine IO/access gate
inside the SAME synchronous RuntimeLoan. Native collection, Entry and Host
creation are unique, and whole-Source retirement waits for that loan. All
journal reads, writes, recovery and admitted work must stay inside it; no
worker, mutation capability or library/cold adopter may escape. The journal
does not mutate shared Head/O/H/WAL/I/S objects and requires no global text-lane
registration. Actual Current4/physical, key, covering-save and publication
proofs remain separate. Future independent cold/detached writers need genuine
per-context serialization before enabling that broader ownership profile.

It must look up the exact token before beginning new work, cover its intent
and durable KeyAttempt before key entry, retain exact raw/output across write
failures, and construct opaque SavedInvite only after real covering barriers.
Cold KeyAttempt without Raw is held, never permission to sign again. Missing,
unknown, malformed or torn quota/selector evidence must not become empty
usage. Safe reclamation/paged retention is needed before claiming indefinite
invite history. The signing journal's later key-attempt/raw stages and their
actual physical/IO admission are not implemented by the initial unsigned profile.

Add must independently recheck current inviter/policy/public plan, expiry,
nonce use and exact signed JoinRequest/KeyPackage. Saved issuance cannot stand
in for present admission. Joined pin/source custody, governing archive,
multi-member installation, shared transport/projection, restart authoring and
multi-peer acceptance remain open.

## Historical verification before durable intent coverage

The11 focused Sync cases,14 Core join-binding cases and36 native-counter cases
pass. They cover exact original fields/draws, changed retries, wrong
seed/source/purpose, interrupted Clock/RNG/native/key entry, expiry overflow,
partial source-Busy End custody, cancellation and lost raw return. Synthetic
native atoms establish lifecycle properties, not Tauri caller authorization.

All five actual Root preparation/transport-context regressions pass, including
the final early-family check. The156-case Sync owner cohort, strict RT/Core/Sync
lint, full3,485-case frontend, all10 browser flows and ambient/raw-fs gates
also pass. Both read-only reviews have no remaining findings. The17 existing
Root member regressions, native compile (2m10s), strict native lint (3m35s),
formatting and both dependency audits pass. Root strict lint retains556
inherited diagnostics. Complete paired Root/native validation at6f822243 is
queued in [run43733133820](https://github.com/Thalpy/Mewtual/actions/runs/43733133820),
deferring only the agreed10k cost case. No acceptance claim follows from
pending checks. Startup was not rerun: setup, spawning and runtime placement
are unchanged; browser flows passed after the sandbox initially blocked CDP.
