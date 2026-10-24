# Saved-files list

Status: planned, with the owner's decisions recorded on 24 October 2026. Implementation and review are
still to come. This PR is stacked on the cache housekeeping PR, which is stacked on PR #51
(`communication-external-download`).

## Why

When a file is saved out of the app, the private encrypted copy used to acquire it is released and
nothing remembers where the file went. The path is shown once and kept only in the current
session's transfer list. That has three consequences:

- the Transfers SAVED group is empty after a restart, and usually even within a session;
- Show in folder works only on the rare Saved row left behind by an interrupted save;
- the owner's "Replace my older saved copy" setting has nothing to compare against when the same
  file is saved again from chat. It was deferred from PR #51 for this reason.

## Owner decisions

- **Keep a list of saved files.** It is a small per-server record inside the vault, sealed like the
  rest of the vault's records. Each entry holds the file's address, where it was saved (folder and
  name), the saved file's identity, and when it was saved. An entry is written when a save
  publishes.
- **What it enables:**
  - a SAVED group in Transfers that survives restarts, with where each file went and a working
    Show in folder;
  - an "already saved" mark in chat;
  - the deferred "Replace my older saved copy" setting, off by default.
- **What the replace setting must do.** When a file is saved again, Mewtual may replace the copy it
  saved before, and only if that copy is provably Mewtual's own and unchanged, checked by identity
  plus a read-only content check. Otherwise it saves a new copy as today. A file Mewtual did not
  save is never replaced. PR #51's review findings for this setting apply in full: delete only
  through the verified object, publish without replacing, and refuse in shared folders where
  deletion cannot be proven safe.

## Constraints

- **Paths stay native.** Saved locations stay inside the sealed record. The renderer gets only what
  it needs to display, and Show in folder works by entry ID.
- **Bounded.** The number of entries is capped. Entries are removed on Clear and when leaving the
  server.
- **Records, not copies.** The list records saved copies; it never keeps one. Cache eviction (see
  `design-cache-housekeeping.md`) and the saved list are independent unless the design review
  decides otherwise.
- **Compatibility.** This is a new sealed record type, so it needs version handling, and older
  readers must stay non-destructive.

## Verification

- Record round-trip, version and compatibility tests.
- Tests for the replace setting at every interleaving: the old copy changed, replaced, missing or
  its name taken, and a swap at each step.
- UI tests and visual-fixture screenshots of the SAVED group.
- The full suites, `test:startup` and `test:flows`.
- An adversarial review of the diff, as `AGENTS.md` requires.
