# SourceSaved terminal: exact codec and writer contract

Design reviewed against worker checkpoint `a3deac5f`. The terminal DATA codec and seven local
regressions are authored UNRUN. Root's connected warm writer/closure/caller is also now authored
with eight further UNRUN regressions; its final actual-diff static review is clear.
Compiled tests/builds remain paused. The worker's actual `SavedBootstrapSourceFiles` is a
historical original completed-run result, not a cold token. This small durable terminal closes
the successful-last-pass crash window before a later V2 Current activation.

## Closed DATA grammar

Add `IntentKind::SourceSaved { context, source, marker }` at previously unsupported kind tag5.
Use the SAME complete payload as SourcePrepared: reserved payload-version0, framed marker32,
framed complete SourceId32 and framed complete BootstrapSourceContext (maximum692B). Existing
tags1..4, domains, VERSION1, debit/marker encodings and SourcePrepared bytes do not change.
Older readers reject unknown tag5; no unsupported-kind fallback or cleaner is introduced.

The maximal plaintext remains1008B within MAX_INTENT_PLAIN1024; ciphertext cap remains1064.
The transition seed remains at most2366B within4096 (frame4136). It reuses existing fixed
`.intent`, `.intent-stage`, `.intent-seed` slots. No additional persistent file, OS identity,
native incarnation, SDK snapshot, random result or public protocol field is introduced. The
complete context ALREADY binds record and manifest PageIds; do not duplicate caller-chosen IDs.

Standalone strict `validate/decode` must require:

- exact invocation and immutable debit, signer/vault/limits/planned revision1 matching the debit;
- `1 <= io_spent <= debit.io_allowance` (the initial unspent SP is not a saved write);
- `sequence = 3 + io_spent`, checked, and parent present;
- complete borrowed payload framing/count/limit checks and EOF before bounded group copies;
  the subsequent debit/counter/context binding and independent marker checks remain mandatory.

The ONLY new successor is exact `SourcePrepared -> SourceSaved`, where the actual selected SP
has sequence `2 + io_spent`, candidate.parent is its complete IntentId, candidate.sequence is
old.sequence+1 and context/SourceId/marker/io_spent are unchanged. SourceSaved has NO successor
in this grammar. In particular it cannot reopen SP, debit another pass, close/refund a creation,
or become another SourceSaved. Same-candidate physical repair is equality with its original
saved operation, not a second successor. A later activation uses a separate reviewed transaction.

DATA construction/decoding and `successor` prove these statements only. They cannot prove a
worker completed, mint SavedFiles/SourceSaved/Current, or authorize reading a large file. The
transition-seed parser still checks its parent label and nested MAC/marker; the physical writer
MUST additionally open the actual selected parent and call `candidate.successor(actual_sp, id)`
or handle exact already-candidate repair. A matching parent hash alone is insufficient.

## Actual producer and single writer

Root owns the connected writer/caller in Store persistence, worker and SourcePreparation. It
accepts the original CapturedFounderInvocation, concrete Transferred source guard, Registered
cell and ACTUAL `SavedBootstrapSourceFiles`, not raw frames/status booleans. Before preparing
any terminal candidate, `SavedFiles::validate_original` must prove the SAME cell and completed
run, latest selected saved SP, actual source/target/Root, output names/lengths and negatives.
The run must be DRAINED and its real completed output must match the latest SP's execution.

The writer derives terminal fields from that actual SP/cell, reserves all bounded control
candidate/seed/return storage, and caches the exact complete terminal cipher and repair seed
BEFORE the first mutation. The source's actual file/parent barriers precede this candidate.
No reseal/new nonce on a lost return or retry. Metadata RNG uses the existing injected seam;
there is no SDK/group creation call or new large-file read/write pass.

Under original admission -> pins/canonical -> archive Root -> registry/cell/output ordering,
recheck original source/cell, current and stage journal/seed, exact completed output and cheap
negatives after waits. Both current/stage authenticate before settlement. Write/sync the
independent exact terminal seed, stage, promote/sync current, and settle the seed through the
existing small-record barriers. Retain the complete candidate/debt on every uncertain result.
The final actual output-object proof is after all potentially blocking comparer/quota waits;
no later wait or callback may turn it into stale success. Full frame/hash/AEAD verification
belongs to the already spent worker pass, not an uncharged terminal retry.

Only those actual barriers mint a private-field `SavedBootstrapSourceTerminal` (proposed name),
binding the same source/run, terminal IntentId and saved small-operation identity. Parsed tag5
DATA never constructs it. SourcePreparation retains the actual saved result before any later
fallible work and reports only SourceSavedUnexposed, never Current/native success.

`Registered`/lookup must select a NEW exact saved terminal journal phase after `.intent`
replacement. Do not weaken old SP/SavedFiles validators to accept arbitrary descendants or a
scalar `allow_terminal`. The new saved-terminal validator uses that closed phase and original
completed outcome. Use a NEW actual `SavedSourceClosure` event/result (name to agree with Root),
retaining the same completed run and exact saved arm as parent. Cache that concrete prepared
closure BEFORE effects; idempotent retry after promotion selects it directly, rather than
calling the deliberately arm-current-only SavedFiles API and getting stranded. The closure
validator accepts only its exact original arm or actual prepared/saved terminal candidate and
corresponding repair seed, not an arbitrary later head.

Terminal saving does NOT retire the source registry slot: these are still unresolved creations,
and exact registry membership remains part of the validator. The later actual Current/source
handoff must provide a concrete owning retirement witness before removing that exact slot;
no terminal status or missing-slot exception substitutes. The five pending slots must not become
a lifetime five-group limit once Current is implemented, but this codec must not silently solve
that by relaxing membership checks. Cipher/control fees drain outside writers, after their
actual payload and physical work owners.

## Cold reader and interruption result

A cold reader authenticates current AND retained stage/seed under its actual new mount and
selects only an exact known SP->SourceSaved transition or exact terminal duplicate. Unknown,
partial, foreign, older/current disagreement stays held. It never reconstructs the old warm
SavedFiles/run, spent SDK unit or write arm. Without actual terminal current barriers, it may
repair the exact small transition from its complete seed; it cannot simply relabel an SP.

A durable terminal is historical original-producer evidence permitting the NEW paid source
read/typed-restore capture required by the Current design. It does not prove the current large
files remain available/intact. Missing/corrupt source/pages produce Incomplete/repair, never
Absent/new founding. New reads have their OWN allowance; no write-arm refund or exhaustion reset.
Success with `io_spent == io_allowance` is valid: cold activation can use terminal+new read/SDK
admission without another source-write pass. A crash before terminal durability remains exact
small-debt repair or held incomplete; it does not invent completion from matching files.

## Required review and regressions

Codec tests use local DATA fixtures, explicitly no paid/Saved claim: full max1024/1008 geometry,
all old bytes/versions unchanged; exact SP->Saved at spent1 and exhausted spent8; zero/overflow/
wrong sequence/parent/debit/context/source/marker refusal; no terminal->SP/Closed/Saved reset;
strict unknown kind/payload-version/trailing/framing; exact marker/seed binding and a parseable
retarget candidate that still fails actual-parent successor validation. Private Debug stays
sanitized. The new variant must be handled exhaustively by actual consumers before compilation.

Root's actual writer tests must use genuine completed worker evidence: no terminal before
completion or from old/foreign run; latest-arm/parent mutation during wait; each small barrier;
NeverRng retry/lost return; same-object output/Root replacement and original close; exhausted
successful final pass -> terminal -> drop/reopen -> new paid read without SDK/write renewal;
later actual Current slot retirement beyond five creations; unknown records preserved. No literal boolean
may substitute for completed work. The original source/run/output/fees remain in Store custody
through uncertain effects and caller Drop. Exact runtime/copy layouts must fit the real control
fee; unchanged wire caps alone are not that invoice.
The retained small-event bound becomes10, not9: initial SP + at most8 spent arms + terminal.
Root must count the actual closure/event layout and simultaneous candidate/seed/read/result
copies before allocating; merely changing the scalar event count is not proof of the invoice.

Design review is CLEAR for this codec and concrete producer contract. Direct rustfmt, whitespace
and ambient/raw-filesystem static gates passed; no compiler, compiled test or build ran. Codec
actual-diff review is CLEAR with no identified blocker/high/medium in this three-file packet;
all seven new DATA regressions remain authored UNRUN. The connected writer now requires the
actual retained completed-run closure before accepting this kind; DATA parsing alone still
supplies no SourceSaved result. Its eight real writer regressions remain UNRUN and final actual-
diff static review is clear; cold paid source/SDK/Current consumers and all compiled validation
remain required.
