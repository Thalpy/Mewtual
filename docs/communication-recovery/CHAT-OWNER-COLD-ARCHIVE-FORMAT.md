# Private Chat V2 archive format (draft; not implemented)

This is the next production-storage design, not executable acceptance or a supported namespace.
The v6 reader still accepts Empty only. No archive files may be produced until the archive
catalog handler, complete operation invoice and parent-head recovery consumer land together.
The finite owner-fact/v1 checkpoint is not extended into a new production history protocol.
Action/v1 bytes and the already specified owner-evidence/v2 semantics remain unchanged.

## Private records and immutable pages

Use a distinct private Store namespace; never put pages, manifests, journals or provider bytes in
BlobStore or expose them to a network file service. Paths are fixed within the original physical
Store, not supplied by RPC, a peer, decoded manifest or a numeric server alias. Each page is sealed
with the original vault database key. Its file identity is a domain-separated BLAKE3 digest of the
COMPLETE sealed frame, including its original nonce. A retry retains those same bytes/identity;
resealing the same plaintext is not a retry. Private ciphertext IDs are not ActionIDs or public
acceptance proofs. The actual sealed head/catalog authenticates the selected manifest ID.

Maximum page plaintext is65536 bytes. Proposed fixed page prefix is a versioned domain, exact
vault domain32, bounded group<=256, ProfileCID32 and a closed page kind. Every variable vector
has complete borrowed framing/count preflight before copying. The parser never follows a path
embedded in a page. Exact tag/header/AEAD overhead constants must be frozen by format review
before implementation;65536 is a per-buffer limit, not a complete memory invoice.

Three content page kinds suffice for the first vertical:

* Original record chunks: typed record key, complete original-byte digest, total length,
  chunk index/count and <=61440 payload bytes. Action envelopes are <=131072 bytes and therefore
  need at most3 pages. Full framing/group/key/hash/chunk agreement and real signature validation
  are mandatory before importing an assembled action; a partial record has no authority.
* Small-record batch: sorted unique typed record keys and complete original bounded bytes,
  with <=64 rows and one page total. A batch of64 acceptance records fits one page; owner
  header plus its exact body also fits. Public semantic IDs exclude valid signature variants
  where their respective grammars specify that; the private byte digest still names the actual
  retained original representation. This does not let malformed variants occupy a semantic key.
* Complete-node batch: exact tree context and before/after counts; derive the first creation
  ordinal in u128 as2*before-popcount(before), never encode/narrow a caller-supplied ordinal.
  Retain a contiguous run of<=191 newly created hashes in leaf/carry creation order. Check the
  actual checked node count, each leaf's exact body binding and every derived ordinal against
  the prepared append, not caller-supplied total/root labels.
  A64-leaf append needs at most64+127 records under the conservative existing invoice. These
  pages retain O(N) hashes, not a fresh Merkle path for each historical root.

Known record kinds are original Action envelope, signed v2 Profile, signed v2 owner header,
Admission body, AcceptanceRecord, Append body, Removal body and ReviewDecision body. They are
closed distinct keys, not interchangeable32-byte IDs. Unknown kind/version refuses/preserves.
The archive retains one independently validated original envelope per semantic action/header;
every received signature variant is verified before duplicate handling. An equivalent valid
variant need not create another retained row or rewrite old pages. Replicas agree on semantics,
not arrival-dependent ciphertext/signature representatives. Contradictory semantic records and
all actual edits/deletes remain distinct retained originals.

## Bounded manifests, without path-copy history

Each immutable manifest segment links its exact prior manifest ID and commits one operation's
bounded new page descriptors, original source-head identity and resulting owner-header identity
(unchanged for a plain retained-action operation). Descriptor ranges/counts/record kinds are
canonical and nonoverlapping; no arbitrary filenames or external URLs occur. A segment is one
bounded page. The private canonical head contains only the latest manifest ID, total counters
and bounded hot state, not another complete page list or the complete historical action graph.
No current header's own CID may enter its pre-signature state commitment: that would be cyclic.
The separate resulting-public-state grammar still needs its own review before signing v2 heads.
That public commitment must be reproducible from public verified semantics, not a private vault's
randomized ciphertext/manifest IDs that another member cannot inspect or independently replay.

A plain original-action retention operation writes <=3 chunk pages plus1 manifest. A public
acceptance append assumes its exact actions were ALREADY retained under a covering source:
<=1 acceptance-record batch,1 complete-node batch,1 header/body batch,1 manifest. Admission,
Removal or Review writes <=1 header/body batch plus1 manifest; actual MLS/provider replacement
and reply/publication custody have their own separate larger fee. An ordinary operation cannot
spend the prepaid governing/control journal, sequence or file reserves.

Old manifest segments and original pages stay immutable. A disposable lookup index may compact
and discard ONLY its reconstructible index runs. It cannot discard original manifest segments,
records, cuts, decisions, contradiction headers or prior-effective Delete custody. A changed
index is never itself new authority or proof that no prior acceptance exists.

## Critical root and exact pending journal

Reserve the coordinated catalog label ARCHIVE ID2/version1 (JOIN is ID1/version1; neither is
supported yet). Its <=4KiB descriptor identifies the original immutable namespace/layout and
fixed archive head/journal destinations. Keep changing manifest/head roots in those separately
authenticated fixed archive records: a message must not rewrite the potentially large unrelated
retained join journal embedded in the shared catalog. Catalog declarations stay rare/monotonic.
Declare
that handler durably BEFORE creating dependent archive files. Unknown ID/version in EITHER
catalog current or stage refuses before old cleaners. Activation validates existing scope/pin/
operation lineage; missing metadata is not an empty archive. Old authentic component rollback
remains the documented residual without an external monotonic anchor.

There is one fixed pending operation journal for the original head writer. It names the exact
parent canonical identity, exact candidate manifest/head identities, operation kind, paid file/
byte/count allowances and the COMPLETE original sealed candidate pages/manifest needed for repair.
Proposed archive journal cap is512KiB, sufficient for four max-size content/manifest frames plus framing;
it is not accepted as the whole physical fee. Before codecs, enumerate every simultaneous buffer,
actual Vec capacity, AEAD frame, index mutation, directory entry, file handle and uncertain copy.
In particular raw+unframed-cipher+plaintext+retained candidate buffers overlap on real Store reads.
No rough multiplier or logical record cap replaces those lifetimes.

This512KiB covers ONLY the archive's exact page candidates. An actual MLS Add/Remove must also
retain the original separately paid sealed private SDK/provider candidate and its reachable
children; a digest of missing private bytes cannot repair that effect. The archive journal may
reference that exact candidate but cannot replace it with an old provider, replay public facts
as an MLS mutation or pretend the whole private candidate fits512KiB. The SDK heap/provider page
format and complete compound-operation memory/disk/control invoice remain prerequisite gates.
Private SDK replacement references belong to the CURRENT canonical head and its pending debt,
not every historical archive manifest. Otherwise ordinary ledger retention would also force
permanent retention of every complete secret provider snapshot. Reclaiming superseded private
SDK candidates needs its own exact current/pending reachability and durability check; it cannot
delete an original action/fact/page or a provider still needed by a pending epoch/publication.

Journal stage/current inspection is mandatory before positive exposure. Never remove an unknown
stage or uncertain operation merely because its canonical parent is present. Known candidates
cross actual file and child-directory barriers before their covering canonical head references
them. Final admission takes original scope -> canonical head custody, with exact parent CAS.
The active writer cannot be substituted by an equal group/server label after an await.

Cold recovery has only these positive outcomes:

1. Parent still exact: independently validate the whole original candidate and its actual MLS/
   public transition obligations, re-establish actual original resource/execution custody, sync pages and
   manifest, and perform the original parent CAS. Do not mint another action, KP, cut or reply.
2. Exact candidate already canonical: validate/resync its complete reachable child closure,
   retain the SAME retry/publication result and complete the original journal debt.

Any unrelated head, malformed/foreign journal, absent required bytes or uncertain counters remains
closed with all known sources retained. Bounded automatic repair may use exact independently
authenticated original bytes; it cannot invent an empty record or reconstruct signed history
from rendered text. In-memory stale work is cancelled before waiting for storage writers.
No completed closure is reported until required page/file/child-parent barriers really finish.

Decoded journal allowance fields are DATA, never proof of a retained resource lease. Recovery
must obtain an actual hot reservation BEFORE unseal/owned-copy/SDK work and restore persistent
execution/control consumption from the covering authenticated journal. Restart cannot reset an
exhausted allowance or create another retry grant. Fixed pre-decryption frame bounds require an
independent constructor metadata/read allowance; the journal cannot pay for its own first read
by claiming it contains a fee. The actual buffer/phase ledger still needs implementation review.

## Derived lookup and bounded progress

The accepted-action lookup key is exact Profile/Admission/ActionID, with the retained original
record binding, acceptance ordinal and closing tip. An exact duplicate returns its original
result; changed bindings never borrow it and never consume a second leaf. Absence can be used
only against a completely validated exact committed prefix. A missing/corrupt/stale index starts
bounded reconstruction and reports incomplete inventory, never an empty complete set.

Capture a finite manifest/tip prefix before reconstruction so continuous new messages do not
restart the scan. Preserve progress across bounded turns, then reconcile the finite newer tail
under actual single-writer custody before committing a new append. Mandatory Removal/finality/
retirement work preempts ordinary lookup/append work; it never waits for an unbounded archive
scan or another group's bulk transfer. The final on-disk index grammar/work bound is still a gate.

Initial cold startup separates three inventories, rather than treating every missing historical
chunk as a group-wide failure:

* Essential governing/root/key/negative-journal closure: verify the exact selected public
  header/body chain, actual current private key/provider candidate and unresolved publication/
  prior-effective Delete obligations before releasing a positive owner/view. Missing essential
  proof stays held with exact automatic repair; it cannot be bypassed to promise availability.
* Per-Admission accepted-lookup prefix completeness: authenticate the exact acceptance records/
  node sources needed by THAT duplicate/append decision. Unknown lookup pauses only dependent
  acceptance work. An independently proven Remove can freeze the already selected count/root
  without enumerating old action bodies or waiting for that ordinary lookup.
* Audit/body availability: validate requested original chunks against their manifest/digests,
  signature and semantic dependencies before display/import. A missing old action chunk reports
  incomplete admin history and triggers bounded exact repair; it does not stop independent new
  Creates, other channels/groups or a control operation with all its own essential evidence.
  Missing current effect proof still keeps that effect held, and cannot clear prior Delete custody.

This replaces the earlier draft's conservative full-audit startup barrier, which contradicted
control preemption and graceful recovery. All three readers use bounded turns and never load all
historical originals or typed SDK groups into RAM. Control-preemption completion cannot be claimed
when essential governing/key/negative proof itself is unavailable. No missing/corrupt index/body is
reported as an empty complete ledger, and no new authority follows merely from retained bytes.

## Required first actual vertical and adversarial evidence

Do not call this complete with codecs alone. The first integrated producer must retain a real
signed action, append actual acceptance under a v2 header, survive pre/post-rename losses,
reopen through the mandatory catalog and expose the exact retained original through a paid
bounded admin reader. Then test actual Remove/frozen tip, held late action, exact decision and
fresh original-scope restart. No legacy/v1 fact is retroactively promoted to v2 authority.

Regress strict page/chunk/manifest framing, domain/realm/type mixing, unknown stages, missing
children, forged counts/ordinals, malicious huge lengths before allocation, original nonce retry,
duplicate signature variants, matching and unrelated parent CAS, complete child sync failures,
crash journal repair, namespace rollback/refusal, index corruption and bounded finite-prefix rebuild.
Exercise control operations under ordinary storage/count/sequence saturation. Finally exceed the
old lifetime admission/action limits, then run20/50-peer offline bubbles and full combined native
acceptance after the user permits compilation. No new test has executed for this draft.
