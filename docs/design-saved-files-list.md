# Saved-files list

Status: owner's decisions recorded on 24 October 2026; concrete design added on 26 October 2026
(below "Design"), before its read-only design review. This PR is stacked on the cache
housekeeping PR, which is stacked on PR #51 (`communication-external-download`).

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

## Design

### 1. The record

`servers/<id>.saved`, beside `.bin`, `.net`, `.cache` and `.storage`. It is written by native under
the store lock with the existing atomic record helper (temporary name, sync, rename) and sealed
under the vault `db_key` exactly like `.storage` (`frame(seal(db_key, plaintext))`). It is a
non-inventoried record (`write_saved_files_record`, addressed by server number, never by a
caller-supplied path), so it does not rotate the inventory generation.

Plaintext, version 1 (`catcoms_wire` encoding; the codec lives in
`catcoms_storage::kept::saved`, because entries carry the external-file identity type that only the
storage crate constructs):

```
bytes  domain     "mewtual/saved-files" (exact; a different record moved here reads as unreadable)
u8     version    1
bytes  group      1..=256 bytes; must equal the server's group id
u32    count      0..=256
count times, oldest first:
  bytes  id         16 bytes, random (catcoms-rt RNG); the renderer's handle for actions
  bytes  cid        32 bytes, the file's plaintext CID
  u64    size       plaintext bytes, <= 1 GiB (the export ceiling)
  u64    saved_at   wall-clock milliseconds (catcoms-rt Clock) when the save published
  u8     platform   1 Windows, 2 Unix (selects how `folder` decodes, as in the export journal)
  bytes  folder     platform-encoded absolute folder path, 1..=16384 bytes
  bytes  leaf       UTF-8 file name, the journal's leaf rules (`valid_leaf`)
  u8     identity   0 none, 1 present
  if 1:
    bytes folder_id 25 bytes, the export journal's identity layout, platform must match
    bytes file_id   25 bytes, likewise
```

Rules checked on decode: ids unique, CIDs unique (**one entry per file**), count and lengths in
bounds, no trailing bytes, and the whole plaintext at most 1 MiB (a 256-entry list of maximal paths
would be about 4.5 MiB, so the size bound, not the count, is what caps a list of very long folder
paths; typical entries are about 200 bytes). The reader reads at most 1 MiB plus the seal overhead.

**Cap: 256 entries per server** (proposed; it matches the file index's own 256-row cap, so a list
can hold one entry for every file a server can list at once). Adding an entry first removes the
existing entry for the same CID, appends the new one, then drops the oldest entries until both
bounds hold. Dropping an entry only forgets it; it never touches a file.

**Loading** returns one of:

- `Missing`: no record, or a record naming another group (a stale file under a reused server
  number). Reads as an empty list; the next save writes a fresh record over it.
- `Found(list)`.
- `Unreadable`: a damaged record, another domain, or a version this build does not know (a newer
  Mewtual wrote it). **Non-destructive**: nothing writes, rewrites or removes it except leaving the
  server. Saves still succeed but are not recorded (the result says so), Clear and Show in folder
  refuse, no replace runs, and Transfers says the list could not be read. Going back to a build
  that reads it restores everything.

**Who writes it, and when.** Native, right after a save publishes, under the store lock and then
the registry lock, after checking that the server entry is still the same incarnation (`instance`).
This is the order `persist_captured` and the vault backup already use, and leave removes the
registry row before `remove_server`, so a save that races a leave either writes before the row goes
(and `remove_server` deletes the record) or sees the row gone and writes nothing. An account switch
replaces every incarnation, so a save can never write into another account's vault. A failed write
does not fail the save; the result adds "Mewtual could not add this file to its list of saved
files."

What an entry records, by publication path:

| Publication | Entry | Identity |
| --- | --- | --- |
| Journaled export (`export_sequence`, also its reopen recovery) | folder, folder identity, final leaf, file identity from the Published journal | yes |
| Resume preflight recovery (crash after the link) | the same, from the recovered journal | yes |
| Non-resumable writer (no stable identity, network folders) | folder and leaf of the published path | none |

Identity comes only from a journal, because the journal is only started where the filesystem's
identity is trusted (birth time, local volume). An entry without identity cannot be checked: it
gives the chat mark and a SAVED row, but no Show in folder and no replace.

**Leave.** `remove_server` deletes `.saved` with the other per-server records (#55's leave order
is unchanged). The saved files themselves are never touched. The leave confirmation adds "Files you
saved from it stay where they are."

**Cache housekeeping.** Decided here: the list is **not** a cache pin. It records copies outside
the vault; its entries name nothing the cache needs. `design-cache-housekeeping.md` is updated.

### 2. Renderer projection and commands

All four are fenced on the UI session generation and the server incarnation like
`check_saved_download`. None takes or returns a path.

| Command | Risk | Does |
| --- | --- | --- |
| `get_saved_files(server)` | account-read | `{ readable, files: [{ id, cid, name, savedAt, size, savedTo, checkable }] }`, newest first. `name` is the saved file's own name (the leaf, for example `report (1).pdf`), `savedTo` is `"downloads"` when the folder equals the resolved Downloads folder exactly, `"other"` otherwise, `null` when no Downloads folder resolves; `checkable` is "has identity". |
| `check_saved_files(server)` | account-read | Checks every checkable entry read-only (folder identity, then a no-follow open of the leaf compared by identity), newest first, in **one** blocking worker holding one of PR #51's two check slots, for at most 5 s on the sanctioned clock (a stop flag ends the worker between entries); returns `{ id, name, check }` for each entry checked, only for entries unchanged in a re-read of the list afterwards. Anything not reached is left out (the renderer shows "could not check"). |
| `reveal_saved_file(server, entry)` | os-facing | PR #51's `reveal_saved_sequence` over an entry: the bounded read-only check (3 s), the list re-read and compared, the session rechecked, then the file manager shows the file selected. Same fixed refusals. |
| `forget_saved_file(server, entry)` | account-write | Removes the entry (Clear). Never touches the file. Refused while the list is unreadable. |

`save_group_file` gains `replace_older: Option<bool>` (a flag, never a value). The signature pin
test is updated: still no path, folder or destination parameter.

### 3. Transfers: list and journal together

- **Needs you** and **In progress** rows still come only from private Download records (the export
  journal), unchanged.
- **Saved** shows list entries, newest first, plus any Saved private row (a Published record left
  by an interrupted tidy-up, PR #51). A list entry with the same CID as a Saved private row is not
  drawn separately: that private row is the file's one row and keeps its own actions. Clear of
  such a private row also forgets the list entry whose identity equals the published journal's, so
  Clear makes the row go away instead of revealing a second one.
- The live transfer list already leaves out files with a private row; it now also leaves out a
  finished download (`done`) whose CID has a list entry, so a finished save shows once, in Saved.
- A list row reuses the `.sdl` row design and icons:

  | Check | Icon, word | Sentence | Actions |
  | --- | --- | --- | --- |
  | present, not checked yet | saved, "Saved" | none | Show in folder, Clear |
  | could not check | saved (muted), "Saved", hint "Could not check the file" | none | Show in folder, Clear |
  | no identity | saved (muted), "Saved", hint "Mewtual cannot check files in that folder" | none | Clear |
  | missing | saved-missing, "Saved file missing" | The saved file is no longer there. Save again makes a fresh copy. | Save again, Clear |
  | replaced | saved-replaced, "Saved file replaced" | A different file is at the saved name now. Save again makes a fresh copy. | Save again, Clear |

  Where line: "Saved to Downloads, 26 Oct" or "Saved to another folder, 26 Oct" ("Was saved to"
  when missing or replaced; no folder named when `savedTo` is `null`). Save again is the ordinary
  chat save of that file and is offered only while the file is still listed in the server. Clear
  asks nothing (it forgets a record; the file stays). List rows stay in Saved when missing or
  replaced: nothing of the device is held for them, unlike a private row, so nothing needs the user.
- Checks: once per Transfers visit, one batch, results kept only for the same entry id and name.

### 4. The chat mark

From the loaded list only (no network, no disk): file cards in chat, the Files list and the file
details pane show a small "Saved" mark (the saved icon and the word) on a file whose CID has an
entry, titled "You saved this file on this device. Transfers shows where." Cards built before the
list loads are updated when it changes. The list reloads on server switch, after each save, after
Clear, and on Transfers visits.

### 5. "Replace my older saved copy"

**Setting.** Settings, App, Chat & Media, a "Saved files" section: a checkbox "Replace my older
saved copy", off by default, kept in `localStorage` on this device (like the other device
preferences), with the help text "When you save a file again, Mewtual replaces the copy it saved
before, if that copy is still unchanged. Files Mewtual did not save are never replaced. Applies on
this device, to every server." Every save (chat, Transfers, Save again, Save as) passes
`replaceOlder: true` while it is on.

**When it runs.** All of these must hold, else the save is an ordinary new copy:

1. `replace_older` was passed;
2. the new copy was published by the journaled writer (it has an identity) and its private record
   was released (a record kept for a leftover temporary name means "not finished tidying");
3. the list held an entry for the same CID before this save's entry replaced it, with identity, on
   this platform, whose folder identity equals the new copy's, and whose leaf differs from the new
   copy's (equal means the old copy was already gone and the new copy took its name);
4. the per-(server, CID) replace claim is free (two saves of one file at once: the second makes a
   normal copy);
5. the list is readable.

**Order inside `save_group_file`.** With the setting off, or when the conditions above fail early,
the existing order is untouched (inspect and reveal, then release the record, then the final
fence). When a replace will run, the record is released first (`finish_download_export`; if that
fails, no replace), then the entry is recorded, the replace runs, and only then inspection and
reveal run on the final path, fenced by `invocation_current` (the recovered-save path already uses
that fence, because releasing the record retires the storage lease by design), followed by the
same final guarded fence.

**Sequence** (storage, on the blocking pool with one of the four cleanup slots; native checks the
invocation and server incarnation before each effect, and passes the same check in for the steps
inside one worker):

1. **Hold the folder** at the new copy's folder path, requiring the new copy's folder identity.
   Unix: classify it (the existing `pin_folder(.., classify: true)`); a folder other accounts can
   or may modify refuses here, before anything is touched.
2. **Prove the old copy, read-only.**
   - Windows: open `folder/old_leaf` with `DELETE | FILE_READ_DATA | FILE_READ_ATTRIBUTES |
     SYNCHRONIZE`, share mode `FILE_SHARE_READ` only, `FILE_FLAG_OPEN_REPARSE_POINT |
     FILE_FLAG_BACKUP_SEMANTICS`. A program that has it open for writing makes this fail (sharing
     violation: "could not be checked"), and while it is held nobody can write, rename or delete
     it, or rename its folders. Require a regular file, no reparse point of any kind, no read-only
     attribute, link count 1, identity equal to the entry's, then read the whole content through
     this handle: exact length equal to the entry's size and plaintext CID equal to the entry's.
   - Unix: `openat` no-follow in the held folder; regular file, identity equal, link count 1, owner
     write bit set (a read-only file is refused), then hash through that descriptor with its size
     and modification time read before and after the hash (a change during the hash refuses).
3. **Delete the old copy through the verified object.** Windows: `delete_by_handle` on the handle
   that was just hashed (the PR #51 fs_at path). Unix: rename the name aside (no replace, random
   name from the RNG seam) relative to the held folder, open the aside name, require the same
   identity, size and modification time as when hashed, then `unlinkat`; anything else puts it back
   without replacing and refuses. Hashing happens before the rename (PR #51 review H3), so the
   user's file is never hidden while it is read.
4. **Give the new copy the old name, without replacing.** Pin the new copy (Windows: the
   delete-denying pin of publication; Linux and Android: its descriptor) and require its identity,
   then the existing no-replace `link_open` to `old_leaf`, then read `old_leaf` where it was linked
   and require the new copy's identity. A taken name ("someone saved a file there meanwhile") stops
   here: the new copy keeps its own name and the user is told.
5. **Record the new name**: the list entry of this save changes its leaf from the new name to the
   old one, compare-and-swap on entry id and leaf, under the store and registry locks.
6. **Remove the new copy's first name** through `remove_verified` with the new identity (two names,
   so one goes). If that fails, both names stay (the same file) and the user is told.

Notes added to the save result (no "·", no em dashes):

| Outcome | Note |
| --- | --- |
| Replaced | "It replaced the copy Mewtual saved before." |
| Old copy missing (or its name already reused by the new copy) | none |
| Another folder | "Mewtual kept the copy it saved before, because this one went to another folder." |
| Changed, read-only, more than one name, or another object at the name | "Mewtual kept the copy it saved before, because that copy was changed since or is not the one Mewtual saved." |
| Shared folder | "Mewtual kept the copy it saved before, because other accounts may change that folder." |
| Could not be checked (busy, held open, error, a second save of the same file) | "Mewtual kept the copy it saved before, because it could not be checked right now." |
| No identity (either copy), unreadable list, record kept | "Mewtual kept the copy it saved before, because it cannot check files in that folder." (or "...could not be checked right now.") |
| Old deleted, name taken | "Mewtual removed the copy it saved before, but another file now has its name, so this copy keeps its own name." |
| Second name left | "This copy also kept its temporary name; you can delete one of the two." |

**Crash table** (the list is the only record; the new copy's private record is already released):

| # | Crash after | State on disk | List | Next |
| --- | --- | --- | --- | --- |
| R1 | the new entry, before step 3 | old and new copies | new entry at the new name; old entry gone | The old copy stays, unlisted. |
| R2 | step 3 (old deleted) | new copy at its own name | new entry at the new name | Consistent: one copy, under the numbered name. |
| R3 | step 4 (linked), before step 5 | the new copy under both names | new entry at the new name (present) | A second name stays; consistent. |
| R4 | step 5, before step 6 | both names | entry at the old name (present) | A second name stays; consistent. |
| R5 | Unix, inside step 3 between the rename aside and the unlink | old copy at `mewtual-removing-<random>` | old entry gone | Residual: plaintext of the old (complete, verified) copy stays in that folder under a random name nothing records. Same residual as PR #51's writers without a journal. |

Lock, a server change or the invocation ending stops the sequence before its next effect; every
stopping point is one of the rows above.

**Authority** (THREAT-MODEL): the setting is a renderer preference, so a compromised renderer can
pass `replace_older` on any save. Native then deletes only an object that (a) the sealed list, which
only native writes after a real publication, names as this server's earlier saved copy of the same
file, (b) is that exact object by handle identity including birth time, (c) still has exactly the
bytes Mewtual saved (length and plaintext CID, which equals the copy just published), (d) has one
name, is not read-only and is not a reparse point, (e) is in the same folder as the new copy, and on
Unix that folder is not one other accounts can modify. So flipping it can only delete Mewtual's own
unchanged earlier output, after an identical copy was published beside it. `forget_saved_file` only
drops list entries; `reveal_saved_file` is PR #51's read-only check and reveal.

**Residuals.** Replacing removes the old copy as a whole, including anything another program added
that is not file content (Windows alternate data streams such as tags, extended attributes,
permissions or timestamps); the content check covers the data only. A same-account process that
already had the old copy open (Unix has no share modes) could change it between the hash and the
unlink. The Windows 64-bit file index on ReFS and the Unix limits of PR #51 apply unchanged.

### 6. Compatibility

- A new file; builds before this one never open it. A vault backup copies it as opaque bytes.
- Version byte: a future version 2 is `Unreadable` here and stays untouched (above).
- Going back to the #55 build: the record is ignored, and that build's `remove_server` does not
  delete it, so leaving a server there leaves a sealed `.saved` behind (it names the group, so a
  later server reusing the number ignores it and overwrites it on its first save). Documented.

### 7. Verification

- Storage: codec round trip, every bound, each malformed shape, unknown version, wrong domain,
  one-per-CID upsert and both caps; read-only entry checks; the replace sequence for each
  interleaving (old changed, replaced, missing, read-only, a second name, held open on Windows,
  name taken, a swap before each step through test hooks), on Windows and in WSL Linux.
- App: the sealed record round trip across a store reopen, other group, unreadable record left
  byte-identical, `remove_server` deletes it.
- Native: recording at each publication path and the incarnation fence; get, check (bounded,
  stale entries dropped), reveal by ID, forget; Clear of a Saved private row forgets only the
  matching entry; the replace orchestration's liveness stops.
- Frontend: projection validation, row views, merge with private rows, the live-list filter, the
  chat mark, the setting, and the App handlers; visual-fixture screenshots of the Saved group.
- An adversarial review of the diff, as `AGENTS.md` requires. CI runs the complete suites,
  `test:startup` and `test:flows`.
