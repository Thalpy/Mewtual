# Gate 6 Score composer component

`ScoreEditor.svelte` is a production composer shell over the existing typed native `ScoreModel`,
not a second document store, audio engine, fixture fallback or persistence path. Its parent owns
scope, ordinary/retained Save identities, pending/error status, recovery, playback authorization
and lifecycle cancellation. The parent must key the component by object identity and provide
`onplay` only when the current stable group and all playback prerequisites are available.

The public props are `model`, `readonlyReason`, `loading`, `error`, `patchLibrary`, `onop`,
`onaddpatch`, optional `onplay`/`onstop`/`playing`, and `onnotice(text, "error"|"info"|"success")`.
`onaddpatch` returns the verified recipe hash; it does not make that recipe active. Note placement
requires the hash in the current native model, so a queued or ambiguously saved recipe cannot
authorize dependent cells. The integration parent must freeze the target before asynchronous
recipe work, as it does for other Studio saves.

The grid keeps all 25 chromatic pitches C4–C6 (MIDI 60–84) and pages sixteen sixteenth steps at
a time through the authored 1–256-step length. Header controls write native title/BPM/beats/steps
operations. Notes carry an explicit authored sixteenth-step hold; drums select receiver-owned
pads 0–9. There is no fixed audio hold or duplicate synthesizer. Continuation cells are visual
only. Known same-pitch overlap refuses local entry without shortening either note; concurrent
overlap remains retained native evidence and makes the score unready.

Clicking a filled onset clears it. Shrinking the header never trims cells. All raw header,
cell and recipe slots remain inspectable, including concurrent removals, losing setters,
out-of-length cells and overflow. The inspector pages slots (24/page) and alternatives (8/page)
to bound DOM work without dropping evidence. It displays complete source operation/author/nonce
and recipe/cell values. Explicit value choices author fresh ordinary operations; an old source id
is diagnostic, never replay authority. Clear/remove cleanup stays available while unready, at
capacity or outside the current step header. Loading/read-only states refuse writes.

Focused coverage includes pure operation boundaries and actual compiled-Svelte interaction tests:
chromatic endpoints, partial pages, numeric/Unicode validation, authored holds, adjacency/overlap,
capacity and shrink cleanup, raw conflict/removal/source preservation, evidence pagination,
read-only writes and recipe publication-before-readback. The primary integration agent owns the
mandatory complete root/native/frontend suites, production composition, visual fixture validation
and final combined adversarial review. This isolated component does not close Gate 6.

Bounded verification: all 12 focused pure/compiled-component tests pass; Svelte checking reports
zero errors/warnings, and staged diff checking passes. An independent read-only actual-file review
reports no BLOCKER/HIGH/MEDIUM/LOW finding and independently ran the same 12 tests successfully.
Its integration condition (key by immutable target and route callbacks through session gates)
remains a parent obligation. These are not results for the final composed app or full Gate 6.
