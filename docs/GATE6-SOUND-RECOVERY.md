# Gate 6 explicit sound recovery

The existing native recovery/retained-draft Copy commands accept one additive closed choice:
`{kind:"soundRecord",value:<64 lowercase hex source operation id>}`. No caller-supplied sound
body, descriptor, target id, duration or historical author is accepted. The planner finds that
exact operation in the authenticated historical Flipnote sound records and derives the proposal,
original attribution and consumed source id. It uses the existing same-group/channel/family,
current-fingerprint, replan, membership, epoch and ordinary fresh Save path; old signatures never
become current authority. Historical automatic sound replay remains manual.

Restore accepts only unconflicted, selected, effective active source setters. It adds recipes,
SFX, immutable export births or a nonnull Score link to wholly absent destination slots; it never
overwrites another value or negative intent. Reusable recipe removals (including retained history)
block additive Restore, while explicit Copy may re-add the recipe through ordinary remove-wins
register semantics. SFX/export deletions anywhere in current or retained/staged destination
history forbid same-id resurrection in either mode. Export births are immutable: exact existing
content is Unchanged, different/competing births conflict even under Copy. Explicit removals and
null Score links are Copy-only. Caps are current effective 64 recipes / 4096 SFX / 8 exports;
SFX also needs a live in-prefix frame and effective in-prefix recipe. Cleanup remains available.

Linked Score existence cannot be established by an id, Index hint or detached projection planner.
Recovery/Copy preview and Apply independently inspect the exact group/channel/family source
wrapper and fully load its typed authenticated Score. Fresh ordinary link Save checks the same
condition before writing an intent. Missing/wrong-channel/wrong-family/foreign-group targets
cannot authorize a link. No fetcher is started. An exact accepted own-log retry is acknowledgement,
not fresh link authority, and does not require a dependency that later disappeared.

Export recovery and retained-draft Copy still enter ordinary SetExport Save. Fresh references
require exact already-held CID/length and complete actual-byte PIXA validation/promotion before
an intent; there is no declaration-only bypass or network fetch at this boundary. A successful
publication alone is not a saved export record. The receiver executes ordinary Save even after
its recovery/Copy exact-retry shortcut, so ordinary Save independently recognizes an accepted
SetExport using exact current signed own-log equality. That acknowledgement does not reload lost
media or evicted/disposed source history. A pending intent alone cannot take the shortcut.

Focused coverage includes all sound operation families, attribution/body derivation, source
conflicts/removal wins, unknown/art/cross-family choices, destination caps and retained permanent
deletions; real actor recovery and draft Copy, missing/invalid held PIXA with no intent, stale
preview, independently typed link scope/disappearance, fresh-mount persistence/media pinning,
snapshot eviction or source disposal plus media loss followed by exact accepted retry, and the
closed native selector. Full root/native/frontend suites and final composed Gate 6 acceptance
remain the primary integration agent's responsibility. No unfinished repair, direct Closing
Save, P5 authority, new persistence format or new native command is introduced.

Bounded verification on this checkpoint: all 10 focused sound planner/real actor tests pass,
the broader 18-test Restore set passes, and the native closed-choice regression passes. Root
`catcoms-app` all-target/all-feature strict Clippy and separate native all-target/all-feature
strict Clippy pass; root formatting and staged diff checking pass. Independent actual-diff
review and the tiny re-review close the LOW identical-readded-recipe diagnostic correction;
no BLOCKER/HIGH/MEDIUM/LOW remains in this bounded backend/native slice. The full combined
root/native/frontend suites, CI and final cross-layer review are not claimed by this record.
