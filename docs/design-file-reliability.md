# File fetching and explicit kept copies

The shared file index is metadata. Publishing confirms the uploader's local copy and the index
write, not another device's possession. The desktop says **Cached here**, **Partial**, **Remote
copy unconfirmed**, or **No connected provider**. A live member connection alone never becomes a
claim that this particular file is downloadable. Preview errors likewise do not assert that every
peer lacks the file.

## Fetch scheduling

`FetchFileChunk` and `ReadFileRange` prepare authenticated requests on the server actor, suspend
only network I/O in tasks, and authenticate/open/store results back on the actor. Workers have no
MLS owner, file keys, blob store or transport event consumer. A one-shot completion is bound to
the originating sync instance, group, membership epoch, local device, expected provider and exact
signed request. The actor rechecks cancellation and the complete current manifest-set identity
before accepting it. Restoring/replacing an owner, unlisting, changing the manifest set or changing
membership invalidates old work before storage.

There are four attempts per server and eight per process. Each independent attempt holds its own
permits through the actual transport completion and through ready-but-unconsumed responses.
Application cancellation/deadline does not refund a still-live lower stream. Drop explicitly signals
cancellation, including abort of an unpolled worker; the driver rejects a cancelled/closed queued
request before admission. Requests prefer existing live, authenticated member sources, then known live candidates under
the same total cap to preserve restart bootstrap. A candidate is never trusted as a member: its
actual response signer must be in the current roster and authenticate the exact request. This
explicit-read fallback can reveal a requested ciphertext CID to an unproven connected candidate,
as the released fetch path already could; it never promotes endpoint trust and cannot redial. At most four distinct providers per encrypted variant are
considered, with an eight-second attempt deadline and a sixty-second read/chunk deadline. Admission
fails with a busy error instead of waiting on actor-owned capacity. Media ranges are at most one
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

The composite blob store preserves ordinary cache/upload behavior and adds
`blobs/<stable-group>/kept/<plaintext-CID>/`. Each copy owns one exact original encrypted manifest and
its original ciphertext chunks. Separate copies deliberately do not share refcounts. One complete
plan is reserved before fetching; partial, failed and interrupted writes cannot gain free quota from
changes to the replicated index. Kept fetch responses are authenticated and opened before being
written directly into this reservation, bypassing ordinary uncapped cache storage. A new Keep
prefers a wholly present exact variant, then verifies it incrementally. It never mixes alternatives
into an unadvertised retained manifest.

One OS-backed exclusive lease protects each retained directory from simultaneous mounts. A second
mount cannot sweep the live owner's pending directory or admit against a stale quota inventory.
Retained attachment failure disables new kept operations while preserving the ordinary blob store.
Startup enumerates bounded directories/record metadata, checks conservative charges, and cleans the
recognizable `pending` namespace. Cleanup failures block admission instead of refunding surviving
bytes. Only committed directories are served. Ordinary delete/staging cleanup affects primary cache
storage; reads fall back to completed kept chunks even when the primary record is corrupt.

Every retained chunk is file-synced. The ordered plaintext is hashed across all chunks before the
pending directory is committed, with directory sync on Unix. Recreated records in a repair require
the containing directory barrier too. Windows retains the vault's existing file-flush-only durability
seam. Release first renames a copy into the non-serving pending cleanup namespace, making even an
interrupted deletion of its manifest recognizable after restart. Failed cleanup requires an application
restart (explicit UI lock/unlock retains the actors and is not a remount).

**Kept here** means the complete copy passed verification in this store instance. Restart resets that
evidence to **Saved copy; needs checking**. The explicit **Check and repair** action uses the saved
exact manifest, including after shared unlisting, and may fetch missing chunks within the existing
reservation. It does not republish metadata or authorize automatic media loads. Native UI-generation
and server-instance checks prevent stale result publication; locking cancels active keep leases.
Frontend request sequencing prevents an older inventory from resurrecting a released/unchecked copy.

## Remaining limits

- Remote possession acknowledgements, automatic replication and holder-aware eviction are not
  implemented. A second device must explicitly keep/download a copy while a holder is reachable.
- A corrupt presence-ranked exact candidate can make Keep fail even if another compatible variant
  is healthy; bounded whole-variant retry remains a follow-up. Ordinary reads retain variant fallback.
- Unlisted kept copies can be checked, repaired, served by ciphertext CID and explicitly released,
  but standalone export still requires a shared listing. This is disclosed in the copy manager;
  independent recovery/export and server-leave cleanup remain follow-ups.
- A session verification is not continuous disk scrubbing. External deletion/corruption and loss of
  access to group keys can invalidate a saved copy; opening/repairing continues to authenticate bytes.
