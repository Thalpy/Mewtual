# Gate 6 stateful sound design

Combined review identified two structural vault readers which had not consumed Score's mandatory
family trailer. Full restore, signed-operation framing and source-preservation now share one
exact target-selected frame finish. Legacy art/Index still have no trailer. Missing, substituted
or extra family bytes refuse in every complete reader, including Prepared handoff evidence after
restart. Two new genuine signed-source/Closing handoff regressions were demonstrated red before
the correction; all26focused Score and16broader handoff tests pass. Independent actual-diff
re-review closes the MEDIUM finding; this does not enable unfinished native Closing Save/P5.

This extends the existing Studio P1 target, not a second store, journal or transport. All writes
retain signed actor/scope checks, intent-before-source durability, exact seed/recovery preflight,
and recovery-before-replacement. No new native surface is enabled until its complete path works.

## Score state

Root headers are immutable `v:1`, `kind:"score"`, object id, channel id and physical epoch.
Optional authored registers `h/title`, `h/bpm`, `h/bpb`, `h/steps` use the actual Automerge winner.
Absent values mean empty title, 120 BPM, four beats per bar and 16 sixteenth steps, without an
invented author. These deterministic defaults match a one-bar initial composer; changing them
later is a format/compatibility decision, not a preference loaded from one device.

Cells use `c/<step>:<row>`, recipes `p/<hash>`. Values are bounded byte records containing version,
full author and the existing canonical DomainOp, as in Index records. Cell content contains no
protocol metadata. The complete live register (actual winner plus all conflicts) is retained.
ClearCell and RemovePatch store an explicit empty alternative: a removal wins among live
concurrent alternatives. A causally later Set supersedes every exact same-property predecessor
it observed; an unseen concurrent removal still wins. Clearing an empty coordinate records this
intent, not an invented durable NoChange. Coordinates remain reusable; patch hashes remain reusable.
RemovePatch on an absent hash likewise records removal intent. Effective presence is separate
from the raw Automerge winner: any live removal makes the slot empty even if a Set is the raw
winner. Active-recipe checks, capacity and compaction use this effective state.

Before conflict-vector allocation, readers enforce the existing epoch-derived primitive rail;
all keys/values and seed bytes are charged against a 6-MiB byte rail. Historical readers apply
the same rails and reject unknown dependencies before querying their frontier.

A causal reader checks every key/value/winner at the same complete dependency frontier. SetCell
must fit that frontier's step header and refer to an active recipe (drums need none). Clear remains
allowed for retained/out-of-current-range coordinates; removal remains allowed for overflow.
Every change must contain exactly its canonical record, fresh marker and genuinely absent immutable
headers, with exact same-property predecessors. Nested objects, cross-property predecessors,
header cleanup/rewrite, provenance forgery and inherited source-id reuse refuse.

Concurrent valid edits may create missing recipes, overlap, header-length overflow or cap overflow.
Keep all evidence and flag export readiness; do not reject based on receiver-only state or silently
trim notes. The live playable prefix is deterministic: first 4096 noncleared coordinates and first
64 active recipe hashes. Unready/conflicted sources cannot be exported as resolved content.
Cleanup remains possible while unready. Local growth policy and remote causal admission stay separate.

## Seeds, recovery and family dispatch

Score payloads get an explicit new discriminator, preserving all existing Index/art bytes.
Do not infer family by trying operation parsers: title and recipe bodies overlap. Bind the selected
family to the trusted target throughout restart, checkpoint, recovery, overlays and native delivery.

Compaction omits an entire removed slot, never its dominating removal alone. It retains bounded
selected content/conflicts and reports omitted evidence. All omitted/overflow/removal/history data
is first preserved in the existing typed recovery envelope. This bounds removed recipe history
across repeated rotations. Every post-seed change causally includes the immutable seed; normal
explicit registers replace the entire seeded fallback, never merge with a seeded removal.
Seed operation ids are not predecessors for another root property.

Automatic historical Score replay must conservatively refuse when it cannot distinguish current
absence from intervening removal. Manual restore/copy authors fresh operations after current-scope,
fingerprint, capacity and lifecycle checks; it never resurrects a signed old change as current
authority. Complete typed validation precedes an empty score blob-reference result; recipe hashes
are not blob CIDs. Flipnote SFX/export references remain separately validated and held.

## Required scenarios

Cover clear/retype and remove/readd before/after rotation, both concurrent delivery orders and
partial observation of removals; header shrink/overlap/missing-recipe cleanup; source and seed
scope/author/marker/predecessor attacks; exact seed and complete recovery limits; restart and
manual recovery; conservative replay; and unchanged Index/art fixtures. Later native publication
and playback must retain authored durations, actual-byte/CID checks and Deafen/leave revocation.
