# Original bootstrap source custody and write execution (design for review)

The reviewed creation chain now reaches private source registration and its initial small SP
journal. This document also specifies the remaining physical consumers; those planned sections
are not implementations and activate no Chat V2 path.
Compilation/tests/builds remain paused; source and design review are not execution evidence.
The warm initial small SP journal described below is now authored and has passed combined
adversarial static review after a final-current ordering fix. Eight regressions are authored
UNRUN. Large execution arms/files, cold negative repair and the asynchronous worker are still
design/implementation gates; this document does not imply they have been delivered.

## Actual source before journal effects

Authority's next private source owner retains the SAME creator before its first scope mutation.
PendingPrivateStage cannot fall back to untouched Planning. A consuming freeze failure closes
that original rather than signing again. Only completed actual Bootstrap preparation can supply
the later original source guard; after transfer its checks use the producer's existing
`validate_transferred_source`, exact original node/key/context/SourceId and original negatives.
No decoded source reader, scalar phase flag or raw Server constructor enters that guard.

Root's registration accepts the actual detached `PreparedBootstrapSource`, never raw replacement
bytes. The caller retains it in an Option before attempting registration. The registration API
borrows that Option and the original prepaid custody fee mutably. It validates all inputs and
prepares bounded metadata before moving either value; refusal leaves both original values in
the caller. Successful insertion moves BOTH into Store before any SourcePrepared mutation. A
lost reply/caller Drop after insertion therefore cannot lose the only complete ciphertext.

The cell contains the detached ciphertext, full bounded context/SourceId, immutable original
FoundingIntent/target/debit labels and file identities, original weak negatives and fees. It
contains no Server/T/device/key/Store or creator sender. Root selects only the known target and
flat invocation slots derived from its actual original root. No caller path or lifetime scan
is accepted. Another context/source/creator cannot replace a registered cell by matching labels.

## Separate real memory payments

The creator retains its existing actual node/encoding invoice until its owned node drops. Do not
move that fee away from a still-live node merely because ciphertext is transferred. The new
source-phase wrapper has its own separately reserved bounded metadata fee before construction.

Acquire an additional actual source-custody reservation from the SAME configured pool BEFORE
detach/registration. Root retains it after caller Drop. Its explicit requested-buffer inventory
is the complete detached encrypted frame, two bounded full-frame read/verification buffers, the
closed page/manifest frames and fixed small journal/control metadata. Begin conservatively at
`4 * MAX_BOOTSTRAP_SOURCE_FRAME_BYTES + 512KiB`, using checked arithmetic and a concrete layout
assertion. The final implementation must map every actual allocation/overlap to that invoice.
This may conservatively overlap the creator's earlier encoding payment; it never silently
reuses a released fee or claims a whole SDK/native/allocator/RSS limit.
Drop the two verification-read buffers BEFORE calling the existing three-buffer offline source
opener. Those are separate phases of the same invoice, not five simultaneous full-frame buffers.

Source seed and destination writes stream from the same retained ciphertext; they do not make
another full secret/plaintext source copy. Bounded complete encrypted reads used for verification
are separately charged. Files/ciphertext/DTOs precede their possibly last fee owner in drop order.
No pool acquisition, last fee destruction or async wait occurs under admission/pins/Root writers.

## SourcePrepared before the large seed

Registration alone grants no I/O arm. The small journal transition first compares the ACTUAL
current FoundingIntent frame with the actual saved original parent, not just a scalar parent
label supplied by a seed. Its successor must preserve the full immutable debit/marker/facts and
bind every field of BootstrapSourceContextData plus the complete SourceId. Sequence2/io_spent0
is the only initial SourcePrepared transition.

Before the first small transition write, its complete encrypted candidate and complete independent
small repair seed are retained in that same Store cell. A complete seed includes the exact parent
ID, immutable marker and candidate ciphertext. Existing/current/stage/seed are all classified.
Partial first seed repairs only through its SAME original warm held File; complete independently
authenticated seeds may repair exact bounded stage prefixes under their actual parent. Candidate
successor checks apply against that actual parsed parent, or exact already-promoted candidate.
Wrong parents, contradictory counters, foreign objects and unknown stages preserve/refuse.

The opaque SavedSourcePrepared result is made only after actual file/current/parent barriers and
same original root/target/link/source checks. It is not reconstructed from parsed phase/counts.
No large source seed/write is possible before this actual result exists. The SAME original
SourcePrepared and full detached ciphertext remain retained after any ambiguous small write.

## Persistent allowance and one bounded large pass

Large I/O must NOT retain the global admission/pins/canonical/Root/registry mutexes for the
whole pass. Otherwise an8MiB source on a slow disk can stop unrelated text groups. Under short
original writers, save/debit the exact arm and install its unique running-work identity BEFORE
release. The admitted worker receives an immutable shared original source payload plus its
actual fee, fixed target/root identities, original negatives and a retained output/file cell.
Source/frame and fee move together into that immutable payload before initial registration;
workers clone only the small Arc, never the frame or creator/sender/Server/Store. Inert retained
debt owns no session. Actual running physical work additionally needs a minimal non-authorizing
lease on the SAME OS vault lock, acquired before irreversible arm admission. It contains no
Store/keys/source publisher and releases only after actual mutating syscalls drain. Dropping the
old Store must not let a new mount/cold repair race an already-blocked write. Cancellation or
final positive rejection cannot recall that syscall. The real constructor still reports Busy
while that physical lease exists. This is physical exclusion, not permission to expose source.

The per-source output cell retains newly created file handles BEFORE identity/write can fail.
Root keeps that cell and immutable payload even if the caller disappears. Only that actual
admitted work may mutate its file state; another arm/repair cannot race it merely because a
global writer was released. Immutable payload and SHORT run/metadata state are separate from
the LONG file/output mutex. Global writers may inspect an immutable completed outcome or use
nonblocking try_lock/Busy; they must NEVER wait for that long mutex. Preventing only the reverse
I/O→Root acquisition would still let Root wait on8MiB I/O and block other groups. No code may
acquire Root/admission while holding this per-source I/O mutex. Release the file/output guard
before final short admission→pins→canonical→Root
validation and saved-result installation. An output-cell handle does not confer Current.

After admission, cancellation may stop new large work but cannot pretend in-flight writes did
not happen. Retain exact data/file outcomes under a distinct narrow preservation obligation;
positive saved-source/native results still need the original final guards. Every worker keeps
its original payload/control fees through actual file work and result collection. Counter,
running identity and file-result transfer must be reviewed/tested before this asynchronous
admission is implemented. Add barrier regressions proving groupB's ordinary admission can
progress while groupA actually holds the file-I/O mutex, and that old Store Drop cannot enable
new-open/cold repair until that physical work drains. A queued job alone proves neither property.

Keep the one group-creation SDK unit distinct from the original source-write allowance1..8.
Each large pass durably writes the exact next SourcePrepared successor with io_spent incremented
once, same full context/SourceId/marker and contiguous sequence. Only the SAME live cell receives
its unique saved runtime arm after those actual barriers. Drop/timeout/restart/configuration cannot
refund it. A saved unconsumed original local result may be reobserved only through its same actual
cell and held files; two handles share one consumed flag. Consuming happens BEFORE the first large
filesystem effect, with no fallible capacity acquisition remaining. A failed/ambiguous pass stays
spent. Exhaustion retains source/debt and pauses this original; this slice exposes no refill API.
Count ALL full large-frame reads, verification, prefix comparisons and resync work inside that
one pass, including already-complete files. Replaying a lost return cannot repeat large reads
under the old spent unit. If the original actual completed result was retained before return,
the caller may recover that same result using bounded metadata checks only. Otherwise another
verification attempt needs another durably spent unit; matching files alone do not refund one.

One pass attempts only the fixed large source seed, destination and two exact genesis pages,
under explicit read/write byte/operation bounds. It never internally retries until success or
recreates a group/source. Small control transactions have their separate reservation and cannot
pretend to pay themselves with the large-pass arm they are writing. No deadline creates work
credit; deterministic counters and injected protocol seams govern the original allowance.

`source.seed` contains the complete already encrypted source frame, independently selected by
the actual saved SourcePrepared context/ID. The first original seed uses create_new and retains
its actual File before fallible identity/write. A complete authenticated seed plus the actual
journal may repair only an exact destination prefix. Neither a partial cold seed nor an absent
source can recreate the detached bytes from a digest. Those states stay held/unavailable.

After full destination/page verification and actual file/target/parent barriers, return only an
opaque saved UNEXPOSED source result. Root may then retire the exact bounded transient cell;
ambiguity retains it. The original creator keeps its life throughout. Source completion is not
scope-table/head installation, protocol selection, Current or native publication.

## Cold and cancellation boundaries

A cold opener never reconstructs an old SDK or warm large-pass runtime arm. Known invocation/target lookup
classifies all fixed small controls under real bounded fees. Before a new pass, large-file
inspection is limited to bounded metadata/presence; it does not read/hash/decrypt the large seed
to prove completeness for free. An actual complete SourcePrepared and the selected seed's bounded
file observation may obtain a DISTINCT fresh negative-only per-pass admission. It first durably
advances the actual current SP io_spent and
sequence, preserving exact context/SourceId/marker/parent and consuming only remaining allowance.
It never reconstructs/reuses any old warm saved arm from decoded bytes. All full seed/destination
verification and resync work belongs to that new pass, including complete destinations. No
live/Fresh/Current source or positive saved-source result is reconstructed. Full seed verification
then occurs INSIDE that spent pass: a truncated/corrupt seed consumes the pass and remains held.
A known missing file may refuse through metadata alone without performing any large read.
Missing/partial first large seed remains unavailable and preserves all known records. In
particular, SourcePrepared absence-of-file is not an unexposed-closure witness.

Original creator/mount cancellation prevents new positive admissions and saved-source exposure;
already admitted exact ciphertext preservation can finish only under a separate narrow inert
repair admission, never the full live-source guard weakened with a boolean. Store preserves the
complete debt and its fees while that negative repair is pending. The original source, execution
budget and expiry are never reset to avoid presenting an error.

## Required regressions before any readiness claim

- Actual original founder to transferred source and Store custody, including registration
  refusal before transfer and caller Drop after insertion but before SourcePrepared reply.
- Every small transition/large file barrier, lost local return, exact held-object retry,
  foreign equal/prefix files, same-object corruption and original root/target replacement.
- No large write before saved SourcePrepared and a real saved once-only arm; exact shared-arm
  consumption, no pre-handoff allocation failure after debit, and exhaustion across restart.
- Wrong actual parent/candidate/marker/context/source/counter, including a seed that names an
  earlier valid parent but illegally changes SourceId under a later actual SourcePrepared.
- Cancel before registration/arm/large-write/return, distinct negative repair after creator loss,
  closed task/no new SDK, fee lifetime through delayed file/read work and other-group progress.
- Complete source/page mismatch refusal and strict offline DATA open against independently
  saved context/SourceId; no cold source/Fresh/Current adopter or history truncation.

SDK typed restore, accepted-index production/governor, public root-kind2, durable typed chat
send/receive, administrative full history and native/UI activation remain separate required work.

Design review: the restricted warm source-cell/SourcePrepared/one-shot-pass design is static
clear with the phase-separated invoice and distinct cold negative admission above. Actual source
and physical implementation/review plus all focused/full validation remain required. The added
short-lock asynchronous worker requirement is a further design-review gate before implementation.
