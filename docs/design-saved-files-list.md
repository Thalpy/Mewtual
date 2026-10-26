# Saved-files list

Status: owner's decisions recorded on 24 October 2026; concrete design added on 26 October 2026
(below "Design") and revised after its read-only design review (revision 2; dispositions at the
end). This PR is stacked on the cache housekeeping PR, which is stacked on PR #51
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
bytes  owner      1..=512 bytes: the server's group id followed by this device's member id in it
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

Rules checked on decode: ids unique, **one entry per saved copy** (no two entries name the same
place: the same folder and name, or the same file identity), count and lengths in bounds, no
trailing bytes, and the whole plaintext at most 1 MiB (a 256-entry list of maximal paths would be
about 4.5 MiB, so the size bound, not the count, is what caps a list of very long folder paths;
typical entries are about 200 bytes). The reader reads at most 1 MiB plus the seal overhead.

**Several copies of one file** (revision 2, review M1): saving the same file twice makes two copies
on disk, so the list keeps two entries and Transfers shows two rows, each with its own name. A
newer entry replaces an older one only when both name the same place (the same folder and name,
compared case-insensitively for Windows entries, or the same file identity): that name now holds
the newer copy, so the older entry's file is certainly not there any more. A replace that does not
happen therefore never forgets the earlier copy.

**Cap: 256 entries per server** (proposed; it matches the file index's own 256-row cap). Adding an
entry appends it as the newest, then drops the oldest entries until both bounds hold. Dropping an
entry only forgets it; it never touches a file.

**Owner binding** (revision 2, review L3): the record names the group and this device's member id
for it. Leaving and joining again makes a new member id, so a record that survived a leave (a failed
`remove_server`, or one an older build left) reads as `Missing` and is written over instead of
bringing back the old list.

**Loading** returns one of:

- `Missing`: no record, or a record naming another owner (another group, or an earlier membership
  of this one). Reads as an empty list; the next save writes a fresh record over it.
- `Found(list)`.
- `Newer`: a version this build does not know (a newer Mewtual wrote it). **Never written over or
  removed** except by leaving the server; going back to a build that reads it restores everything.
- `Damaged`: it unseals to something that is not a valid list, or does not unseal. Left alone too,
  except by an explicit "Start a new list" in Transfers (revision 2, review L4), which replaces only
  a record that still reads as `Damaged`.
- `Unavailable`: it could not be read right now (an I/O error, for example another program holding
  it). Left alone; the next attempt reads again.

In the last three cases saves still succeed but are not recorded (the result says so), Clear and
Show in folder refuse, no replace runs, and Transfers says the list could not be read.

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
| `get_saved_files(server)` | account-read | `{ state, files: [{ id, cid, name, savedAt, size, savedTo, checkable }] }`, newest first. `state` is `"ok"`, `"newer"`, `"damaged"` or `"unavailable"` (files only with `"ok"`). `name` is the saved file's own name (the leaf, for example `report (1).pdf`), `savedTo` is `"downloads"` when the folder equals the resolved Downloads folder exactly, `"other"` otherwise, `null` when no Downloads folder resolves; `checkable` is "has identity from this platform". |
| `check_saved_files(server)` | account-read | Checks every checkable entry read-only, newest first: the folder is held by one handle and checked against its identity, then the name is opened relative to it with attributes-only access (revision 2, review L6: no read access, so antivirus and other programs' share modes do not delay it) and compared by identity. One blocking worker holding one of PR #51's two check slots, at most 5 s on the sanctioned clock (a stop flag ends the worker between entries); returns `{ id, name, check }` for each entry checked, only for entries unchanged in a re-read of the list afterwards. Entries not reached are left out and read as not checked yet, never as "could not check". |
| `reveal_saved_file(server, entry)` | os-facing | PR #51's `reveal_saved_sequence` over an entry: the bounded read-only check (3 s), the list re-read and compared, the session rechecked, then the file manager shows the file selected. Same fixed refusals. |
| `forget_saved_file(server, entry)` | account-write | Removes the entry (Clear). Never touches the file. Refused while the list is not readable; writes nothing when the entry is already gone. |
| `reset_saved_files(server)` | account-write | "Start a new list": writes an empty list only while the record still reads as `Damaged`. Refused otherwise (a newer version's record is never replaced). |

`save_group_file` gains `replace_older: Option<bool>` (a flag, never a value). The signature pin
test is updated: still no path, folder or destination parameter.

### 3. Transfers: list and journal together

- **Needs you** and **In progress** rows still come only from private Download records (the export
  journal), unchanged.
- **Saved** shows list entries, newest first, plus any Saved private row (a Published record left
  by an interrupted tidy-up, PR #51). Native marks such a private row with the ID of the list entry
  whose identity equals its published journal's (`savedEntry`), and that one entry is not drawn
  separately: the private row is that copy's one row and keeps its own actions. Other copies of
  the same file keep their own rows. Clear of such a private row also forgets that entry, once the
  record is actually released (a Clear stuck in Removing forgets nothing), so Clear makes the row go
  away instead of revealing a second one.
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
list loads are updated when it changes, by a pass over the existing `.ref-card[data-file-cid]`
elements that sets text and attributes only (never markup). The list reloads on server switch,
after each save, after Clear, and on Transfers visits.

### 5. "Replace my older saved copy"

**Setting.** Settings, App, Chat & Media, a "Saved files" section: a checkbox "Replace my older
saved copy", off by default, kept in `localStorage` on this device (like the other device
preferences), with the help text "When you save a file again, Mewtual replaces the copy it saved
before, if that copy is still unchanged. It replaces only a copy it can prove it saved. Applies on
this device, to every server." (Revision 2, review M3(g): no absolute "never", because Unix has a
documented same-account window.) Every save (chat, Transfers, Save again, Save as) passes
`replaceOlder: true` while it is on. "Every server" also means every account on this device that
uses the same Windows or Unix user.

**When it runs.** All of these must hold, else the save is an ordinary new copy:

1. `replace_older` was passed, and the per-(server, CID) replace claim was free when the save
   recorded its entry (two saves of one file at once: the second makes a normal copy);
2. the new copy was published by the journaled writer (it has an identity), **its entry was
   durably recorded** (revision 2, review M4), and its private record was then released (a record
   kept for a leftover temporary name means "not finished tidying");
3. the list held, at the moment the new entry was recorded under the same lock, an older entry for
   the same CID with identity from this platform whose folder identity equals the new copy's: the
   newest such entry is the candidate. Other copies stay listed and untouched;
4. the list is readable. A recovered save (the Resume preflight's crash-after-link path) records
   its entry but never replaces.

**Which effect** (revision 2, review M2 and M3(d)):

- the old name already holds the new copy (the old copy was gone and the new save took its name,
  possibly differing only in case on Windows): nothing to delete; the older entry is forgotten (its
  name is the new copy's now);
- the new save started from the old copy's name and was numbered because that name was taken:
  delete the old copy, then give the new copy the old name. "Started from" is the name a **fresh
  one-click save into Downloads** asked for (`Exported::started_from`, set only in that branch of
  `export_sequence`), compared case-insensitively on Windows. A Save as name, a reopened journal
  (a plain Resume of an interrupted Save as reopens the chosen folder's journal with the name the
  user typed) and a recovered save have none (adversarial review M2);
- otherwise (a different listed name, or a name the user typed in Save as): delete the old copy and
  leave the new copy's name alone. A name the user chose is never renamed.

**Order inside `save_group_file`.** The entry is recorded right after publication in every case
(revision 2, review L10). With the setting off, or when the conditions above fail, the rest of the
existing order is untouched (inspect and reveal, then release the record, then the final fence).
When a replace will run, the record is released next (`finish_download_export`; if that fails, no
replace), the replace runs, and only then inspection and reveal run on the final path, fenced by
`invocation_current` (the recovered-save path already uses that fence, because releasing the record
retires the storage lease by design; the review checked that it weakens no fence), followed by the
same final guarded fence.

**Sequence** (storage, on the blocking pool with one cleanup slot held from step 1 to step 6; a
`keep_going` check from native, the invocation and server incarnation plus a 2-minute deadline on
the sanctioned clock, runs before each effect and every 1 MiB of hashing, revision 2, review L9):

1. **Hold the folder** at the new copy's folder path, requiring the new copy's folder identity.
   Unix: classify it (the existing `pin_folder(.., classify: true)`), but act on "shared" only
   after step 2 has seen that the old copy is there and ours (revision 2, review M3(a): never say
   anything about a copy that may not exist). Windows reads the folder identity before opening the
   old copy, so a folder that went or moved reads as "could not be checked", never as "the old copy
   is missing" (which would forget an entry whose file still exists; adversarial review L4).
   **Pin the new copy** (adversarial review H1): Windows opens it with the delete-denying pin of
   publication (`open_leaf_pinned`), elsewhere with a descriptor, and requires its identity and
   size; a new copy that is already gone refuses here. The pin is held through steps 2 to 4. If the
   old name already holds the new copy (any case on Windows), nothing is pinned or deleted.
2. **Prove the old copy, read-only.**
   - Windows: open `folder/old_leaf` with `DELETE | FILE_READ_DATA | FILE_READ_ATTRIBUTES |
     SYNCHRONIZE`, share mode `FILE_SHARE_READ` only, `FILE_FLAG_OPEN_REPARSE_POINT |
     FILE_FLAG_BACKUP_SEMANTICS`. A program that has it open for writing or deleting makes this
     fail (sharing violation: "could not be checked"), and while it is held nobody can write,
     rename or delete it, or rename its folders. Read the folder identity again by path and require
     it (revision 2, review L1: the held leaf pins its ancestors, so this proves the same folder).
     Require a regular file, no reparse point of any kind (a cloud placeholder or deduplicated file
     reads "cannot check files in that folder", review M3(b)), no read-only attribute, link count 1,
     identity equal to the entry's, then read the whole content through this handle: exact length
     equal to the entry's size and plaintext CID equal to the entry's.
   - Unix: `openat` no-follow in the held folder; regular file, identity equal, link count 1, owner
     write bit set, and the folder's sharing rule applied to the file itself (owned by this
     account, no other-write bit, group write only for a proven user-private group without an
     extended ACL; review L8 and adversarial review L1, so a umask-002 export in the user's own
     folder is accepted while a file another account may write is not), then hash through that
     descriptor with its size, modification time and change time read before and after the hash
     (a change during the hash refuses).
3. **Delete the old copy through the verified object**, only if the pinned new copy still has its
   name (checked right before the delete, and on Unix again right before the unlink with a put-back
   otherwise; adversarial review H1). Windows: `delete_by_handle` on the handle that was just
   hashed (the PR #51 fs_at path); the pin stops the new copy's name going meanwhile. Unix: rename
   the name aside (no replace, random name from the RNG seam) relative to the held folder, open the
   aside name, require the same identity, size and modification time as when hashed, then
   `unlinkat`; anything else puts it back without replacing and refuses. Hashing happens before the
   rename (PR #51 review H3), so the user's file is never hidden while it is read. Native forgets
   the old entry once the storage call returns (after step 4 when it runs).
4. **Give the new copy the old name, without replacing** (only in the rename case above). Through
   the pin from step 1, the existing no-replace `link_open` to `old_leaf`, then read `old_leaf`
   where it was linked and require the new copy's identity. A taken name, or any other failure,
   stops here: the new copy keeps its own name and the user is told; if the new copy's own name is
   gone too, the note says the new copy was moved or deleted instead of naming it.
5. **Record the new name**: the list entry of this save changes its leaf from the new name to the
   old one, compare-and-swap on entry id and leaf, under the store and registry locks. If this does
   not succeed, step 6 does not run (review M4).
6. **Remove the new copy's first name** through `remove_verified` in a **must-keep-another-name**
   mode (revision 2, review H1): Windows requires a link count of at least 2 on the delete handle,
   which denies delete sharing, so the other name cannot go between the check and the delete; Unix
   requires it on the aside descriptor and otherwise puts the name back. When nothing was removed,
   both names are looked at again, read-only and by file identity (`FirstNameDrop`, re-review
   MEDIUM): the result names a second name only if both were seen to hold the copy; if only the
   first name holds it (the old name went or holds another file), the entry moves back to the first
   name and the result points there; if neither can be shown, no name is claimed.

Notes added to the save result (no "·", no em dashes; revision 2, review M3: none claims a copy
exists unless step 2 saw it):

| Outcome | Note |
| --- | --- |
| Replaced (renamed into place, or deleted beside a new name) | "It replaced the copy Mewtual saved before." |
| Old copy missing, or its name already holds the new copy | none |
| No candidate in this folder (only copies elsewhere) | "Mewtual did not replace the copy it saved before, because this one went to another folder." |
| Changed, or another object at the name | "Mewtual did not replace the copy it saved before, because that copy was changed or is not the one Mewtual saved." |
| Read-only | "Mewtual did not replace the copy it saved before, because that copy is read-only." |
| More than one name | "Mewtual did not replace the copy it saved before, because that file has another name too." |
| Shared folder or file (after seeing it is ours) | "Mewtual did not replace the copy it saved before, because other accounts may change it there." |
| Could not be checked (busy, held open, a name that could not be read, a second save of the same file, time out, a folder found changed once the new copy was pinned; adversarial review L2, re-review LOW 1) | "Mewtual did not replace the copy it saved before, because it could not be checked right now." |
| No identity (either copy), a reparse point, another platform | "Mewtual did not replace the copy it saved before, because it cannot check files in that folder." |
| The new copy was not at its name (or not its size) when the replace started, or went before the delete; a folder that moved lands here, since the new copy moved with it (adversarial review H1, re-review LOW 2 and 5) | "Mewtual did not replace the copy it saved before, because the new copy was moved, changed or deleted meanwhile." |
| List not readable, entry not recorded, record kept, a recovered save | "Mewtual did not replace any earlier copy this time." (adversarial review L3: no claim that an earlier copy exists) |
| Old deleted, the move failed (name taken or otherwise), the new copy seen at its own name; or, after the link, only the first name still holds the copy | "Mewtual removed the copy it saved before, but could not give this copy its name, so it keeps the name <new name>." |
| Old deleted, the move failed and the new copy's own name was seen gone too | "Mewtual removed the copy it saved before, but the new copy was moved or deleted meanwhile." |
| Old deleted, the move failed and the new copy's own name could not be read (re-review LOW 1) | "Mewtual removed the copy it saved before, but could not give this copy its name." |
| Second name left (both names seen to hold the copy) | "The saved file also kept the name <new name>. Both names are the same file, so deleting either one keeps the other." |
| After the link, neither name could be shown to hold the copy (re-review MEDIUM) | "Mewtual could not confirm which name this saved file has now." |
| Unix put-back failed | "Mewtual could not finish replacing the copy it saved before. It may be in that folder under a name starting with mewtual-removing-." |

**Crash table** (the list is the only record; the new copy's private record is already released):

| # | Crash after | State on disk | List | Next |
| --- | --- | --- | --- | --- |
| R1 | the new entry, before step 3 | old and new copies | both entries | Consistent: two copies, two rows. |
| R2 | step 3 (old deleted), before step 4 and before the old entry is forgotten | new copy at its own name | both; the old one reads "missing" | The old row offers Clear. |
| R3 | step 4 (linked), before the old entry is forgotten (adversarial review L5) | the new copy under both names | both; the old one reads "replaced" (a different file, by identity, is at its name), the new one present at its first name | The old row offers Clear and Save again; a second name stays. Consistent, and no file is claimed as the old copy. |
| R4 | the old entry forgotten, before step 5 | the new copy under both names | new entry at the new name (present) | A second name stays; consistent. |
| R5 | step 5, before step 6 | both names | new entry at the old name (present) | A second name stays; consistent. |
| R6 | Unix, inside step 3 between the rename aside and the unlink | old copy at `mewtual-removing-<random>` | the old entry is kept (it is forgotten only after the storage call returns), reading "missing" | Residual: the old (complete, verified) copy stays in that folder under that random name; the old row's "missing" is literally true for its name. |

Lock, a server change, the invocation ending or the deadline stops the sequence before its next
effect; every stopping point is one of the rows above.

**Authority** (THREAT-MODEL): the setting is a renderer preference, so a compromised renderer can
pass `replace_older` on any save. Native then deletes only an object that (a) the sealed list, which
only native writes after a real publication, names as this server's earlier saved copy of the same
file, (b) is that exact object by handle identity including birth time, (c) still has exactly the
bytes Mewtual saved (length and plaintext CID, which equals the copy just published), (d) has one
name, is not read-only and is not a reparse point, (e) is in the same folder as the new copy, and on
Unix neither that folder nor the file is one other accounts can modify, and (f) only while the new
copy, pinned, still has its name (on Windows the pin keeps it there through the delete). So
flipping it can only delete Mewtual's own unchanged earlier output, after an identical copy was
published beside it and while that copy is still there, and it never renames a name the user
typed. `forget_saved_file` and `reset_saved_files` only change list entries;
`reveal_saved_file` is PR #51's read-only check and reveal.

**Residuals.** Replacing removes the old copy as a whole, including anything another program added
that is not file content (Windows alternate data streams such as tags, extended attributes,
permissions or timestamps); the content check covers the data only. A same-account process that
already had the old copy open (Unix has no share modes) could change it between the hash and the
unlink, and file timestamps are too coarse to catch a same-size write in the same tick. On Windows,
a writable memory mapping made before step 2 is not covered by share modes, and a read-only
attribute set after the step 2 check is ignored by the POSIX delete (attribute writes are not
covered by share modes either). On Unix the new copy's descriptor does not stop its name going, so
a same-account process that removes or renames the new copy between the last check and the unlink
(a few system calls) leaves neither copy at a name; Windows has no such window. The new copy's pin
keeps its name but shares writing (as publication's pin does), so another program may edit the new
copy in place during the replace: the old copy is then removed beside a copy that is still there,
but no longer necessarily identical. The list's bulk
check can hold both of PR #51's shared check slots for up to 5 s per call, so a renderer that
spams it delays PR #51's checks and reveals (bounded, adversarial review L7). Recording holds the
registry lock across sealing and the `fsync` of the list (as `persist_captured` does); a
performance cost only (adversarial review L6). The Windows 64-bit file index on ReFS and the Unix
limits of PR #51 apply unchanged.

### 6. Compatibility

- A new file; builds before this one never open it. A vault backup copies it as opaque bytes.
- Version byte: a future version 2 is `Newer` here and stays untouched (above).
- Going back to the #55 build: the record is ignored, and that build's `remove_server` does not
  delete it, so leaving a server there leaves a sealed `.saved` behind. It names the group and the
  member id of that membership, so a later server reusing the number (even for the same group,
  after joining again) reads it as `Missing` and overwrites it on its first save. Documented.

### 7. Verification

- Storage: codec round trip, every bound, each malformed shape, unknown version, wrong domain,
  same-place replacement of entries and both caps; read-only entry checks; the replace sequence for
  each interleaving (old changed, replaced, missing, read-only, a second name, a reparse point, held
  open on Windows, name taken, a case-only difference, the user's own name kept, a swap before each
  step through test hooks, the other name removed between link and removal, the new copy removed
  before the delete, a rename onto an entry's place), on Windows and in WSL Linux.
- App: the sealed record round trip across a store reopen, another owner, each unreadable kind left
  byte-identical, `remove_server` deletes it.
- Native: recording at each publication path and the incarnation fence; get, check (bounded,
  unreached entries left out, stale entries dropped), reveal by ID, forget, reset only when
  damaged; Clear of a Saved private row forgets only the matching entry after release; the replace
  orchestration's conditions and stops, including a failed entry write.
- Frontend: projection validation, row views, merge with private rows by `savedEntry`, the
  live-list filter, the chat mark on cards built before the list loaded, the setting, and the App
  handlers; the visual fixture mocks the new commands; screenshots of the Saved group.
- An adversarial review of the diff, as `AGENTS.md` requires. CI runs the complete suites,
  `test:startup` and `test:flows`.

## Design review dispositions (revision 2, 26 October)

Read-only review of `62a144c6`: no blocker; one HIGH, four MEDIUM, twelve LOW. It checked and found
sound: releasing the record before a replace (and the `invocation_current` fence there), the list
write's lock order against leave, incarnation-bound writes, hash and delete on one Windows handle,
the cap and the byte bound.

| Finding | Disposition |
| --- | --- |
| H1: step 6 could delete the new copy's only name if the old name went between link and removal | Taken: a must-keep-another-name removal (link count of at least 2 checked on the delete handle on Windows, on the aside descriptor on Unix), and step 6 runs only after step 5's compare-and-swap. Test planned with a hook between link and removal. |
| M1: one entry per CID forgot the old copy before the replace was tried | Taken: one entry per saved copy; a newer entry replaces an older one only for the same place; the old entry is forgotten only after the old copy is gone. R5 is now visible as a "missing" row. |
| M2: replace renamed a name the user chose | Taken: rename only when the new save started from the old copy's name; otherwise delete the old copy and keep the new name. |
| M3: notes claimed things not shown (existence, reparse files, Mewtual's own second names, case, "temporary", move failures, "never") | Taken: notes say "did not replace" and are given only after the old copy was seen; reparse points read "cannot check files in that folder"; more names has its own note; the old name holding the new copy (any case) is checked by identity first; the second-name note names the file; a failed move has its own note; the help text drops "never". |
| M4: replace after a failed entry write; step 6 after a failed step 5 | Taken: both are conditions now. |
| L1: "same folder" not enforced on Windows | Taken: the folder identity is read again after the leaf is held. |
| L2: claim, lock and permit ordering | Taken as stated (claim before recording, candidate from the same locked read, store lock across load, change and write, no write when nothing changed, no session guard under the store lock, one cleanup permit held throughout, recovered saves never replace). |
| L3: a stale record could come back after a rejoin under the same number | Taken: the record names the group and this device's member id. |
| L4: a damaged record had no recovery | Taken: "Start a new list" for a damaged record only; a newer version's record stays untouchable. |
| L5: Clear of a Saved private row forgets its entry | Kept (Clear makes the row go away), but only after the record is actually released. |
| L6: batch check wording, order and read access | Taken: unreached entries read as not checked yet; attributes-only opens relative to one held folder handle; entries from another platform are not checkable. Newest first stays (documented). |
| L7: cards already built miss the mark | Taken: an explicit pass over existing file cards, text and attributes only. |
| L8: Unix content check | Taken: group- or other-writable files are refused, change time is compared too, and a failed put-back has its own note; the same-tick residual is documented. |
| L9: unbounded hash | Taken: `keep_going` every 1 MiB (liveness and a 2-minute deadline). |
| L10: crash between release and entry write | Taken: the entry is recorded first in every case. |
| L11: Windows residuals | Documented (memory mappings, a read-only attribute set after the check, "every server" means every account of the same OS user). |
| L12: documentation drift and the fixture | Taken: cache housekeeping, HANDOVER, THREAT-MODEL, INTERFACES and the visual fixture are updated with the implementation. |

## Adversarial review dispositions (26 October)

Read-only review of the whole diff `0c32f7ae..8ad6ce7a`, code and docs: no blocker; one HIGH, two
MEDIUM, seven LOW. It checked and found sound: lock order against leave and `persist_captured`,
the incarnation and owner fences, no path or identity crossing IPC, the command policy and
capabilities, unreadable records never written over, the pin on the other name before the first
name goes, claim, release and fence order, no `unsafe`, no ambient time or randomness, and no "·"
or em dash in app text. Fixes in `5466c28e` and `702fa266`; every new regression was break-checked
(it fails with its guard removed).

| Finding | Disposition |
| --- | --- |
| H1: the old copy could be deleted after the new copy had already gone (the user deleting the apparent duplicate, antivirus), leaving no copy, and the note then named a file that no longer existed | Taken: the new copy is pinned before step 2 (Windows: the delete-denying pin, so its name cannot go during the delete; elsewhere a descriptor) and must still have its name right before the delete and, on Unix, again before the unlink (put back otherwise). New refusal `NewGone` and outcome `RemovedNotMoved { still_named }`, with truthful notes. Tests: `the_old_copy_is_kept_when_the_new_copy_is_gone_before_the_delete` (Windows: the hook's delete of the new copy fails while pinned; Unix: refused, old copy kept). |
| M1: a failed `forget(old)` then a successful rename wrote two entries for one place, which reads as damaged | Taken: `SavedFiles::rename` drops any other entry at the target place; `save_saved_files` refuses to seal a list that would not decode. Tests: `renaming_onto_an_old_entrys_place_keeps_the_list_readable` (storage) and `a_rename_after_a_failed_forget_keeps_the_list_readable` (native, real store). |
| M2: a name typed in Save as could be renamed after a plain Resume (the chosen-folder journal reopened with destination Downloads) | Taken: `Exported::started_from` is set only by a fresh one-click save into Downloads. Test: `only_a_fresh_one_click_save_names_what_it_asked_for` (fresh, chosen, resumed and recovered). |
| L1: Unix refused group-writable files outright, so umask-002 exports never replaced | Taken: the folder's sharing rule (owner, other-write, proven private group without an extended ACL) applies to the file. |
| L2: `FolderChanged` read as "that copy was changed" before the copy was seen | Taken: it reads "could not be checked right now". |
| L3: "did not replace the copy it saved before this time" assumed an earlier copy | Taken: "Mewtual did not replace any earlier copy this time." |
| L4: Windows read a moved folder as "old copy missing" and forgot an entry whose file still existed | Taken: the folder identity is checked before the old copy is opened; an unreachable folder is "folder changed". |
| L5: no crash row between the link and `forget(old)` | Taken: crash row R3. |
| L6: the registry lock is held across sealing and `fsync` | Recorded as a residual: performance only, the same shape as `persist_captured`; narrowing it would change the leave fence this design relies on. |
| L7: a spamming renderer can hold both shared check slots | Recorded as a residual: bounded by the 5 s wait per call. |

### Re-review of the fixes

Read-only re-review of `8ad6ce7a..702fa266`: no blocker or high; H1, M1 and M2 hold on every path
it traced (the pin does not block the link, the post-link check or step 6; every delete is preceded
by a name check; `started_from` has one source; the decode backstop cannot stop recording). One
MEDIUM (from `b1b4d67c`, before the fixes) and five LOW, all fixed in `5c5af307` with break-checked
regressions.

| Finding | Disposition |
| --- | --- |
| MEDIUM: after the link, a failed step 6 always said "both names" and pointed the result at the old name, even when the old name had gone (then deleting the first name loses the only copy) or the first name had | Taken: `drop_first_name` returns `FirstNameDrop` from a read-only look at both names; native names a second name only for `BothNames`, moves the entry back and points at the first name for `OnlyFirst`, and claims no name for `Unknown`. Tests: storage `the_first_name_stays_when_the_old_name_went_after_the_link` (now `OnlyFirst`, and `Unknown` when both went), native `the_replace_sequence_changes_the_list_only_after_each_effect` (each answer). |
| LOW 1: an unreadable new-copy name read as "gone" | Taken: `still_named` returns a `Result`; before a delete an error refuses as "could not be checked" (on Unix after putting the old copy back); `RemovedNotMoved` carries `Option<bool>` and `None` names nothing. |
| LOW 2: "moved or deleted" missed a size change | Taken: "moved, changed or deleted". |
| LOW 3: a stop before the link asserted the new copy still had its name | Taken: it is checked. |
| LOW 4: the Unix check before the unlink and `still_named: Some(false)` had no test | Taken: `unix_the_new_copy_gone_after_the_rename_aside_puts_the_old_copy_back` and `the_new_copys_own_name_is_never_claimed_once_it_went_before_the_link` (Windows: the pinned copy cannot be deleted, so it links). |
| LOW 5: doc drift (a moved folder shows the new-copy note; `started_from` docs) | Taken: the notes table and the doc comments. |
| Design question: the Windows pin shares write access and checks only identity and size, so the new copy can be edited in place during the replace | Accepted and documented as a residual: it cannot be renamed, replaced or deleted, which is what keeps a copy; a stricter pin would refuse saves another program opens at once. |

## Ledger (26 October)

Branch `saved-files-list`, stacked on #55: merged at `0c32f7ae` (`0cf3d56e`) and again at
`bc891185` (`4bc47b1a`, clean). Commits: plan `804d8b1b`; design `62a144c6`, revision 2
`28a82504`; record and codec `4da7fef9`; one entry per copy and the verified replace `8bf24457`;
candidate reasons and plan accessors `93da0dbb`; native recording and orchestration `b1b4d67c`;
renderer, chat mark, setting and fixture `21c65ec2`; docs `8ad6ce7a`; adversarial fixes
`5466c28e`, `702fa266`; review record `249ec864`; re-review fixes `5c5af307`; then this ledger.

Focused checks run locally (CI runs the complete suites, `test:startup` and `test:flows`, which
were not run here, as briefed):

| Check | Result |
| --- | --- |
| `cargo test -p catcoms-storage --lib kept::` (Windows) | 192 passed |
| the same in WSL Ubuntu (`--locked`), plus strict Clippy for `catcoms-storage` there | 199 passed; Clippy clean |
| `cargo test -p catcoms-app --lib store::saved_files_tests` | 5 passed |
| `cargo test -p catcoms-app --lib actor::file_transfers` (after the second merge) | 64 passed with 4 threads; a run at full parallelism timed out five export tests in their 5 s `bounded()` helper, a known load flake (each passes alone) |
| native `cargo test --lib` filters: `saved_files`, `private_downloads`, `keep_session`, `cache_housekeeping`, `publish`, `fallback`, `reveal`, `six_client_recovery::files` (after the second merge) | 15, 47, 14, 10, 20, 7, 6 and 7 passed |
| `npm test` (complete frontend unit suite) | 2596 passed |
| `npm run check`, `npm run build` (after the last renderer change) | 0 errors, 0 warnings; built |
| strict Clippy (`-D warnings`, all targets and features): root `-p catcoms-storage -p catcoms-app`, and the native workspace | clean |
| `cargo fmt --all -- --check` and the native `cargo fmt -- --check` | clean |
| `scripts/check-no-ambient.sh`, `scripts/check-store-raw-fs.sh` | passed |
| `cargo deny check` | only RUSTSEC-2026-0387 (the advisory the brief ignores); bans, licences and sources ok |

Every regression added for a review finding was break-checked: it fails with its guard removed and
passes with it restored. Screenshots of the fixture states (900 by 640, headless Edge over CDP,
after `html[data-visual-ready="chat"]`) are kept outside the repository with the handoff.
