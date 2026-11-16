# File fetching and explicit kept copies

The shared file index is metadata. Publishing confirms the uploader's local copy and the index
write, not another device's possession. The desktop says **Cached here**, **Partial**, **Remote
copy unconfirmed**, or **No connected provider**. A live member connection alone never becomes a
claim that this particular file is downloadable. Preview errors likewise do not assert that every
peer lacks the file.

## Fetch scheduling

Explicit native reads can now fall back to one consented connected member as a file helper after
direct providers. The helper forwards at most four named current MLS sources per signed grant;
it receives encrypted pages and no file key, and does not silently Keep them. Each page is at most
64KiB. Helpers are discovered through bounded MLS catalog pages, not a truncated descriptor or
connected-peer roster. At most four helpers by four exact encrypted variants retain independent
positions. Local Busy leaves unstarted source coverage intact. A signed Unavailable advances
fresh coverage first, then permits at most two bounded retry sweeps under the same read authority
and remaining allowance. Local150ms helper pacing does not guarantee remote quota availability.
An unsupported helper never falls back to forwarding a legacy whole-file request.

Root reserves route custody before the actor spends durable Keep work, then commits the exact
selection. A proven local preparation refusal spends no coverage. A post-debit refusal does not
refund durable work. Ambiguous transport retry uses the same source batch with a fresh grant
and debit; old route tickets and connection incarnations cannot become current again. Both
successful and failed AEAD outcomes repeat original read/effect/work authority before durable
storage or bad-candidate decisions. These paths do not yet establish the full native restart,
bridge-replacement, load or physical-network acceptance matrix.

A lost helper connection is a local availability observation, distinct from cancellation of the
original download. Its response is discarded without importing bytes or rejecting a wrapped key;
fresh usable helpers receive turns before a bounded later sweep of the old helper's sources.
Waiting for an offline cached helper preserves coverage and does not spend retry passes. This
selection still admits at most four helper identities per read; it does not rotate a fifth helper
into a full cache. Native/session, Root, membership/epoch and original deadline retirement remain
terminal. The typed effect observation is checked again after AEAD; a discarded complete page
does not refresh the quiet deadline or reset retry accounting. A post-debit route failure keeps
that debit spent even when no transport request was submitted.

`FetchFileChunk` and `ReadFileRange` prepare authenticated requests on the server actor, suspend
only network I/O in tasks, and authenticate/open/store results back on the actor. Workers have no
MLS owner, file keys, blob store or transport event consumer. A one-shot completion is bound to
the originating sync instance, group, membership epoch, local device, expected provider and exact
signed request. The actor rechecks cancellation and the complete current manifest-set identity
before accepting it. Restoring/replacing an owner, unlisting, changing the manifest set or changing
membership invalidates old work before storage.

There are four concurrent attempts per server and eight per process. Each independent attempt holds its own
permits through the actual transport completion and through ready-but-unconsumed responses.
Application cancellation/deadline does not refund a still-live lower stream. Drop explicitly signals
cancellation, including abort of an unpolled worker; the driver rejects a cancelled/closed queued
request before admission. Requests prefer existing live, authenticated member sources, then known live candidates under
the same total cap to preserve restart bootstrap. A candidate is never trusted as a member: its
actual response signer must be in the current roster and authenticate the exact request. This
explicit-read fallback can reveal a requested ciphertext CID to an unproven connected candidate,
as the released fetch path already could; it never promotes endpoint trust and cannot redial.
Search admits connected providers in batches of four, continuing beyond absent or busy preferred
providers. One bounded tried set admits at most 320 distinct candidates per chunk/round; each
candidate can be tried against the admitted exact encrypted variants (at most four). A busy
candidate stays queued while later batches are searched. These are search bounds, separate from
the concurrent-attempt permits above. Each attempt has a twelve-second deadline (transport timeout
plus two seconds), beside a sixty-second quiet read/chunk deadline. Accepted page progress refreshes
the quiet deadline. At most 1,024 accepted pages per chunk and six retry rounds after the initial
search bound repeated partial work; retry rounds preserve rejected exact variants. Ordinary reads
have no persistent execution budget; new-format Keep work adds the durable allowance below. Whole-job
admission fails with a busy error when its bounded inventory is full. Once admitted, a read waits
quietly for lower-attempt permits, retaining its exact candidate, deadline and remaining allowance.
Cancellation cannot refund an older request's actual lower ownership. Media ranges are at most one
`CHUNK_BYTES` window (8 MiB), including windows crossing two chunks.

Queued actor commands take priority over transfer completions. Local kept copying yields between
chunks as well. A wholly local copy may still delay background sync while its finite sequence of
ready steps runs; other existing actor operations (upload verification, storage scans, legacy
document/avatar fetches) retain their earlier scheduling. This change does not claim that every
actor operation is nonblocking or provide production latency measurements.

## Independently created indexes

File-index reads and mutations project all conflicting `ROOT/FILES` list objects in stable object
order. One total 256-position budget includes malformed rows across all lists. Publication counts
all admitted list lengths; deletion, expiry updates and authenticated own-row repair use the same
bounded projection. Independent first uploads are tested both with and without a shared seed list.
No CRDT/wire migration is introduced; older clients may still expose only their winning list.

Automerge 0.10 materializes conflicting property values internally even for `get`. The application
projection bounds its row/list work, not allocation of arbitrary inherited CRDT history. Incompatible
or excessive encrypted-manifest variants still fail closed; this is not Byzantine index availability.

## Keep on this device

Keeping is an explicit per-file local action. Nothing automatically downloads backups, and a peer,
wiki pin or unsigned circulation expiry cannot enable retention or release another device's copy.
Each server has a fixed **1 GiB / 32-file** allocation limit, including conservative padded ciphertext,
disk-encryption overhead and sealed metadata. A maximum of 128 chunks is admitted per exact plan.

The composite blob store preserves ordinary cache/upload behavior. Legacy completed copies remain
at `blobs/<stable-group>/kept/<plaintext-CID>/`; new copies use
`kept/transfers-v2/<plaintext-CID>/`. Legacy/V2/V3 copies own one exact original encrypted manifest. New multi-variant V4 transfers
own the complete original compatible set and its original ciphertext chunks. Separate copies deliberately do not share refcounts. Every admitted candidate's complete
plan is reserved before fetching; partial, failed and interrupted writes cannot gain free quota from
changes to the replicated index. Kept fetch responses are authenticated and opened before being
written directly into this reservation, bypassing ordinary uncapped cache storage. A new Keep
prefers a wholly present exact variant, then verifies it incrementally. It never mixes alternatives
into an unadvertised retained manifest.

One OS-backed exclusive lease protects each retained directory from simultaneous mounts. A second
mount cannot sweep the live owner's pending directory or admit against a stale quota inventory.
Retained attachment failure disables new kept operations while preserving the ordinary blob store.
Startup enumerates bounded directories/record metadata and checks conservative charges. Legacy
`pending` is cleanup-only: it cannot distinguish abandoned work from an intentional release.
New resumable work is never written there. A permanent sealed `format-v2` marker prevents older
readers from treating the new namespace as legacy cleanup. V2 reads, reconciliation and effects
require the full native MLS group and local DeviceId, independently of the UI server number.
Only the canonical lowercase full-group namespace permits retention; aliases remain ordinary caches.
Cleanup failures block admission instead of refunding surviving bytes. Only completed copies are
served. Ordinary delete/staging cleanup affects primary cache
storage; reads fall back to completed kept chunks even when the primary record is corrupt.

Each new-format transfer has a bounded sealed `current` record and at most one exact `next` successor.
The record binds the full scope, random transfer identity, exact manifest, original listing digest,
phase, whole-chunk progress and remaining execution allowance. Its purpose-separated
`kept_metadata_key` is unavailable to the arbitrary blob-sealing API: copying chosen cache
ciphertext offline cannot create a valid transfer or grant. A valid successor must name the exact
predecessor digest/revision. Corrupt or conflicting records block recovery; an older record is
never selected as a fallback that would restore already spent allowance or undo deletion.
Both full record slots, all reserved chunk bytes and the format marker count against the same quota.

Every saved chunk passes file and directory flushes before progress is recorded, including matching
bytes found after an earlier failed flush. On resume the app reopens and authenticates the prefix
and rebuilds the whole plaintext hash from chunk zero. Partial ciphertext stays private to its
exact runtime run. Only a verified whole-file CID can publish Completed. New-format records and
directories use real directory barriers on Unix and Windows; legacy completed-copy repair keeps
its previous platform durability seam. Process restart and injected I/O failures test the ordering;
they are not physical power-loss evidence. Release first publishes Deleting, then moves into
`deleting-v2`, then performs bounded nonrecursive cleanup. Neither a crash nor missing cleanup
metadata can restore serving or fetch authority. The format marker remains after the last release.

New-format work starts with a native fixed allowance: 131,072 requests/pages, three times the
declared ciphertext bytes plus 16 MiB of search overhead, and 120 fifteen-second execution slices.
Bounded request/byte batches and each slice are debited durably before use; a restart loses unused
prepaid capacity instead of renewing its deadline. Page request lengths and debits share one bound,
and an authenticated total beyond the manifest's ciphertext reservation is rejected before staging.
The legacy whole-blob wire carries only a CID, so its fallback conservatively prepays a full 16 MiB
released frame even for a small file. A provider ignoring a requested window remains subject to
transport frame/resource limits; these debits do not claim to measure physical network bandwidth.
An immutable deadline and revocable run owner follow queued requests to driver admission.

Runtime cancellation, capacity refusal and shutdown pause new-format work without releasing its
reservation or resetting allowance. Explicit Cancel/release takes the durable deletion path.
Exhaustion persists and requires explicit Retry. Retry names the exact transfer, expected grant
and a request identity; its compare-and-swap grants one fixed allowance once. Repeating its exact
receipt returns metadata, including while a new run is active, without replenishing work.
Damaged metadata retains conservative quota and blocks admission pending manual recovery; no
automatic repair or silent deletion of ambiguous records is implemented.

**Kept here** means the complete copy passed verification in this store instance. Restart resets that
evidence to **Saved copy; needs checking**. The explicit **Check and repair** action uses the saved
exact manifest, including after shared unlisting, and may fetch missing chunks within the existing
reservation. It does not republish metadata or authorize automatic media loads. Native UI-generation
and server-instance checks prevent stale result publication; locking cancels active keep leases.
Frontend request sequencing prevents an older inventory from resurrecting a released/unchecked copy.
The copy manager separately lists saved-part progress with Resume, Retry and Cancel. Newly
requested partial copies resume on one actor-owned 30-second wake while the native account
session is unlocked. Selection rotates across saved intents; it starts no providerless work unless
the complete exact variant is plausibly local. Every fresh run revalidates membership, saved
manifest and prefix, and uses the unchanged remaining allowance. It does not re-create a missing
saved intent from an index entry. External destination downloads do not use this scheduler.

Explicit Pause/Resume has its own checked intent revision, separate from progress revisions and
budget grants. Even same-value commands advance that order, so a held old Pause cannot undo a
newer Resume. Exhaustion still needs Retry; Retry and exact receipt replay cannot undo Pause.
A deterministic plaintext/manifest integrity failure blocks automatic rehash until explicit Resume.
The original native unlock generation and revocable watch reach actual lower admission. Lock/close
retires captured work synchronously; a fresh unlock cannot revive its token or replenish its budget.

Record V3 adds closed pause/intent/blocked fields. V2 records retain their exact canonical encoding
and predecessor hashes during read and crash reconciliation, including old V2 pending successors.
An explicit successor may upgrade to V3; V3-to-V2 is refused. V2 partial work requires one explicit
Resume because its manual-only format did not distinguish user Pause from a runtime stop.
Completed-copy checks retain an exact cancelable inline lease and Stop checking control, preserving
ownership on stop. An exhausted completed V3 copy exposes Retry check without releasing it.
Legacy completed repair has no V2 journal and does not claim persistent execution limits.

## Whole-variant Keep fallback (V4; integration checks in progress)

New multi-variant transfers capture all two to four canonically ordered compatible manifests from
the original resolved listing. Admission and every resumed run revalidate each exact digest,
whole CID, layout, plaintext equivalence and ciphertext bounds, plus the original set identity.
Unlisting or later index changes cannot add candidates. A missing file key refuses Keep before
reservation or execution; it is not evidence that a saved reference is corrupt.

Each candidate has separate private whole-chunk progress and a durable bad-reference verdict.
The working selection is separate from an optional committed winner. A deterministic failure to
open authenticated ciphertext excludes that exact candidate; transient provider absence does not.
Fallback retires the old run before publishing the selection, then issues a fresh opaque run.
The actor restarts the complete plaintext hash, chunk position, staging, paging and provider search.
It visits at most four candidates per run. All candidates share the original transfer identity,
allowance, explicit Retry grant, Pause/intent and native cancellation. Switching and restart do
not replenish any of them; unused prepaid lower windows are lost, not refunded. Only a verified
whole-file CID can promote the selected candidate to the committed winner. Stopping/failing an
alternative never releases or demotes a previous completed copy. Ordinary get/has/CID enumeration
and completed inventory expose only the winner, even though other candidate bytes share its directory.
Providerless automatic eligibility requires one whole non-excluded candidate to be plausibly local;
a mixture of available chunks from different candidates cannot start work.

V4 reserves every candidate's full ciphertext bounds, conservatively charging duplicate addresses,
and two 400 KiB record slots. Its working-plan mirror plus four maximum plans fit below 360 KiB
including the disk seal; cleanup/validation admits at most 512 unique chunk leaves plus current/next.
This conservative quota can refuse a multi-variant file that a single variant would fit. V2/V3
retain their exact bytes, predecessor hashes and two 80 KiB slot charges. They and legacy completed
copies remain exact-only: there is no implicit enrichment from today's index or release/recreate
migration. Unsupported readers refuse the V4 domain without deleting its saved state.

## Private durable downloads

Native Save first acquires encrypted data into a purpose-separated `mewtual/download-transfer/v1`
record. Its immutable random transfer ID names a canonical `d-<id>` directory; Keep remains keyed
by CID. Both purposes share the same 32-record/1 GiB reservation, deletion remnants, exclusive
mount and persistent execution allowance. A same-CID Download cannot read, pause, release or
change the Keep owner. Completed Download records never enter public `get`, `has`, CID enumeration
or owned source-worker preparation. Only an exact mounted transfer-generation export lease can
read their chunks, at the saved bound. Existing V2–V4 Keep bytes are unchanged; older CID-only
readers refuse Download directories, making the retained store unavailable without deleting the
private transfer. Ordinary cache remains separate. Returning to a current reader preserves spend.

The native actor reuses bounded whole-file acquisition, including member bridging and whole-variant
fallback. It revalidates every original saved manifest and whole plaintext CID before export.
Restart preserves verified whole ciphertext chunks, Pause and spent budgets; it does not preserve
an external plaintext offset or path. Explicit Resume captures fresh native/Root/actor/storage
authority and a freshly resolved Downloads destination. Exhaustion requires a separate explicit
CAS Retry grant. Repeated Save/provider search/reopen does not refill it. Download has no automatic
startup acquisition/export scheduler. Four whole-export slots remain owned through queued plaintext
results, writes and publication. Final publication is no-replace; cancellation removes staging only.

Cleanup after publication is fenced to the exact exported generation, so it cannot erase a later
run of the same record. A stop after publication but before cleanup/response may leave both the
external file and encrypted progress. There is no exactly-once external publication receipt: the
UI tells users to check Downloads before saving again, which may create another copy. Same-user
staging mutation and abrupt native/physical-network acceptance remain outside current evidence.

## Resumable external saves (21 October, implementation in progress)

For destinations that can carry a journal, the export above is now resumable rather than always
fresh. The partly written `<name>.mewtual-part` is tracked in the sealed Download record by its
handle identity (volume, file index or inode, birth time) and a checkpoint of fsynced whole chunks.
Resume reopens it without following links, re-verifies its prefix against plaintext regenerated
from the private copy, continues writing, and publishes with a verified no-replace hard link. A
missing, moved, replaced or relinked file, a removed or different folder and revoked permission
become typed halts ("Folder or file unavailable", "Partly saved file changed") that keep encrypted
progress and offer Save new copy. Remove deletes only the proven partial and never the saved file.
Filesystems without stable identities, birth times or hard links keep the non-resumable writer.
See [EXTERNAL-DOWNLOAD-RESUME](communication-recovery/EXTERNAL-DOWNLOAD-RESUME.md).

## Remaining limits

- Remote possession acknowledgements, automatic replication and holder-aware eviction are not
  implemented. A second device must explicitly keep/download a copy while a holder is reachable.
  The per-server cache limit ([cache housekeeping](design-cache-housekeeping.md)) evicts by
  listing order without a holder probe; Keep in progress, Downloads, own uploads, Studio
  references and files in use are pinned, and a checked completed Keep copy replaces the cache's
  duplicate.
- Legacy/V2/V3 exact-only copies cannot fall back to candidates they never saved. New V4
  whole-variant fallback is implemented with focused integration checks and review in progress.
- Indirect authenticated file service and private durable destination re-export have focused
  native evidence. The combined crash/load/mixed-version matrix and physical acceptance remain open.
- Unlisted kept copies can be checked, repaired, served by ciphertext CID and explicitly released,
  but standalone export still requires a shared listing. This is disclosed in the copy manager;
  independent recovery/export remains a follow-up. Leaving a server now removes its Keep copies
  and Downloads after a confirmation that names them (cache housekeeping).
- A session verification is not continuous disk scrubbing. External deletion/corruption and loss of
  access to group keys can invalidate a saved copy; opening/repairing continues to authenticate bytes.
