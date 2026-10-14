# Gate 6 frontend integration checkpoints

## Typed Score bridge and session

The first frontend checkpoint extends the existing Studio native adapter and session with a
Score family. It does not create another store, activate native commands or expose a production
composer/export button yet. Rust/native integration and final combined verification remain
owned by the parent integration; unsupported native commands still refuse rather than falling
back to the fixture store.

The existing Create/Read/Apply and recovery/local-draft commands carry optional
`objectKind: "score" | "flipnote"`. Omission retains the original Flipnote target. An Index
target cannot carry `objectKind`. Kind is selected by the Index/open target and frozen into exact
save requests, including Unconfirmed Save tickets, historical Apply and retained-draft Copy;
operation bodies are never guessed through multiple family parsers. Score commands refuse a
Flipnote response, and recovery/draft reads check returned content against the requested family.

Score content preserves optional title/BPM/beats-per-bar/steps registers with IndexSource,
raw cell/recipe slots `{removed, register}`, all losing values, and native readiness flags. The
adapter checks global cell coordinates, authored lengths, patch schemas, provenance shape,
duplicate per-register source ids and the typed snapshot's 68192-value rail. `removed` must
agree with **any** retained null alternative, not merely the raw Automerge winner. Models retain
raw slots separately from effective values. Empty defaults (title empty, 120 BPM, four beats per
bar, sixteen steps) invent no operation or source. Historical Score content is labelled as such.

The same session opens/creates Score objects, queues recipe/cell/header operations, preserves
exact nonce/epoch/kind/body on ambiguous retry, and blocks later same-document saves. A preview
still cannot authorize ordinary Save: it takes the existing ticketed local-only route. Pixel/frame
operations refuse a Score target before publishing blobs. Mutable cell/recipe input is copied into
the operation identity immediately. Re-authoring refuses to redirect a retained edit into a
different object family. Scope/read generations retain the existing late-result fences.

Recovery choices use `scorePatch`, `scoreHeader` and `scoreCell` with exact source operation ids.
Whole-version walks put recipes before dependent cells. The frontend conservatively offers
headers and selected removals through Copy, while Restore walks add selected effective recipes
and cells under the current header/extent. The native per-item planner additionally permits
additive Restore of an absent, unconflicted authored header (defaults are not authored values);
it never overwrites an existing header through Restore. Native preview/fingerprint/current-state checks remain
the authority, and all raw conflicts remain available in the read model. This frontend does not
claim automatic replay of historical Score edits or old-author authority.

Verification on this checkpoint: 65 focused native adapter/session regressions pass, including
ten new Score tests. `npm.cmd run check` has zero errors/warnings and `git diff --check` passes.
Actual-diff adversarial review is requested, not yet accepted. The parent runs the complete
mandatory root/native/frontend suites after the combined implementation; none are claimed here.

Remaining frontend work: sound/export DTOs and controls, Score composer, real App playback
lifecycle wiring, native export/readback, fixture updates and visual validation. This checkpoint
does not close Gate 6 or enable an unfinished repair command.

## Family fences and sound read model

The next checkpoint closes the Score bridge review findings without dropping unresolved Save or
Copy identities. A known/retained object family cannot be reopened as another family, including
offscreen retained Copy or any save variant after view-LRU eviction. Default reopen consults
the same retained identity as explicit reopen. An old-family ACK cannot patch another-family
surface or supersede its newer read. Every object-overlay await is fenced before the next IPC;
unknown-family navigation fences recovery listings/walks and clears view-only historical caches,
not ambiguous requests. New Score DTO records/maps/lists are closed data shapes: required fields
cannot disappear into defaults, future fields cannot vanish, and accessors are never evaluated.

Flipnote's optional native `sound` carries complete physical records, not just audible winners.
The parser validates each full operation, authored `duration_ms`, recipe schema and physical
key/source agreement, including losing alternatives. Its model keeps permanent SFX/export
deletions distinct from reusable remove-wins recipes; immutable duplicate export births select
the smallest source id and do not inherit an absent expiry from a losing birth. Readiness keeps
all overflow/missing-reference ids and the native Boolean conflict flag. Old art-only replies
still mean no sound state. The local fixture store now retains explicit authored demo durations;
it remains a development fixture, never a fallback for failed native reads or writes.

Stable `StudioView.group` is optional for compatibility and, when present, is bounded canonical
hex supplied by the native projection. Numeric server handles never substitute for it. Optional
Score family metadata on native target/ticket/lifecycle/Copy/event results is preserved and
validated; missing metadata retains legacy art compatibility.

The bounded actual-diff review and short re-reviews close the original family race,
closed-Score-shape omission, offscreen Copy/recovery residuals, dynamic-readiness-map accessor
and retained-Apply default-reopen liveness issues. No BLOCKER/HIGH/MEDIUM/LOW finding remains
on this DTO/session checkpoint. The reviewer independently ran 23 Score/Sound tests; the
primary frontend worker ran all 87 focused adapter/session/sound/fixture tests, `check` (zero
errors/warnings) and diff checking. Production controls/export/playback wiring and final combined
suites remain open. The new unconnected media helper work is a separate next checkpoint.
