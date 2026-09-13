# Prompt for the Flipnote UI builder

Status update, 2026-09-13: the initial integration below has landed (`0b6e870`).
For follow-up work, extend `studio-native.ts`, `studio-session.ts` and the connected
surface. Do not create another adapter or rebuild the mockup. The original handoff follows;
current command availability and remaining backend gaps are in `FLIPNOTE-UI-HOOKS.md`.

Connect the existing Flipnote mockup to the real backend. Preserve its current design, layout,
components and interactions; build on what is already there.

Read `docs/FLIPNOTE-UI-HOOKS.md` first for callable commands, response shapes, events and retry
rules. Use `docs/design-creative-suite.md` for intended behavior and `docs/HANDOVER.md` for
current backend status.

Implement the core integration:

- Sidebar list, open/create Flipnote, frame/header edits and Index metadata edits.
- PIX publication and bounded fetching using the existing `pix.ts` codec.
- Refresh through `studio-updated` and `settlement-changed`.
- Read-only awaiting-tenure previews.
- Available recovery list/read/preview/apply, backup export and eviction acknowledgement.

Introduce a typed native adapter. The existing `studio-store.ts` and some `studio-contract.ts`
types still describe fixtures; do not cast native responses into those shapes or treat fixture
CIDs as real published blobs.

Preserve unsaved work on errors and refreshes. Retry uncertain saves with the same complete
request, including nonce and epoch. Keep identifiers lossless, preserve conflict/overflow/deletion
evidence, and discard stale asynchronous results after navigation or session changes.

`awaitingTenureReceipt:true` means a read-only history preview. `provisional:true` alone also
occurs on ordinary editable local views. Never invent settlement, receipt or publication claims.

Keep actions requiring unfinished backend support unavailable: durable edits during rotation,
signed repair, claims/Ask/Pass, sound/linked Music and `.pixa` export. Recovery backup export is
already available.

Another agent is continuing Gate 4 backend work. Focus on `apps/desktop/src` and frontend tests;
coordinate any native/Rust or shared-document changes. Do not overwrite unrelated work. Run
relevant frontend checks and report what is connected and what remains unavailable.
