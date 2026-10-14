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
Whole-version walks put recipes before dependent cells. Restore adds selected effective content;
headers and selected removals are Copy-only. Native preview/fingerprint/current-state checks remain
the authority, and all raw conflicts remain available in the read model. This frontend does not
claim automatic replay of historical Score edits or old-author authority.

Verification on this checkpoint: 65 focused native adapter/session regressions pass, including
ten new Score tests. `npm.cmd run check` has zero errors/warnings and `git diff --check` passes.
Actual-diff adversarial review is requested, not yet accepted. The parent runs the complete
mandatory root/native/frontend suites after the combined implementation; none are claimed here.

Remaining frontend work: sound/export DTOs and controls, Score composer, real App playback
lifecycle wiring, native export/readback, fixture updates and visual validation. This checkpoint
does not close Gate 6 or enable an unfinished repair command.
