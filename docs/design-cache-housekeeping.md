# Cache housekeeping

Status: planned, with the owner's decisions recorded on 24 October 2026. Implementation and review are
still to come. This PR is stacked on PR #51 (`communication-external-download`). The saved-files
list PR is stacked on this one.

## Why

A review of how files are stored found five problems in the encrypted blob cache
(`<vault>/blobs/<group>/`).

1. **The cache only grows.** There is no size limit and no eviction. The 30-day file expiry is
   recorded and displayed but never enforced, although `USER_GUIDE.md` implies more, and the
   storage crate's docs describe retention eviction that does not exist.
2. **Leaving a server leaves its files behind.** `remove_server` deletes the snapshot, network
   and cache-address files, but not `blobs/<group>/` or its Keep copies.
3. **Cache writes are not crash-safe.** A chunk is written with a plain `fs::write`, with no sync
   and no atomic rename. "Held" only checks that the path exists, so a truncated chunk can count as
   cached until a read fails its check and fetches it again.
4. **Saving a file does not fill the cache.** A private Download acquires into its own store and is
   deleted after the export, so viewing the same file in chat later fetches it again.
5. **The docs overstate expiry and retention**, as described in item 1.

## Owner decisions

- **Display the file we have.** A file held locally and verified is shown from disk and never
  fetched again. Files are addressed by a hash of their encrypted bytes, so a fetch can only return
  identical bytes; a changed file would be a new listing with a new address. Only missing or
  corrupt chunks are fetched.
- **Saved files propagate.** When a file is saved out of the app, its verified chunks also go into
  the shared cache. The device can then serve them to other members, as it already does for files
  it has viewed.
- **Crash-safe writes.** Cache chunks are written to a temporary name, synced, then renamed. A chunk
  counts as held only after it has been verified.
- **Leaving a server cleans up.** Leaving removes that server's cached and Keep files and reports
  the space freed. It never removes anything Studio still references.
- **A per-server cache limit, set by the user for each server.** When the limit is reached, newer
  files take priority and the oldest files are dropped first, ordered by the file's listing time,
  not by when it was last viewed. Never evicted:
  - files pinned with Keep;
  - files Studio references;
  - files being displayed or transferred right now;
  - files the saved-files list relies on, if any, as agreed in that PR.
- **Footer warning.** The status bar shows, for the currently selected server, a bar of how much of
  its cache limit is used, and an alert with the number of files "leaving soon", meaning the files
  next in line to be dropped. Users are warned before the cache is full.
- **Truthful docs.** The guide and the storage crate's docs must match what the code does. Expiry
  stays a displayed date unless a later decision changes that.

## Open points for the design review

- **The default limit.** Whether it is on by default, and at what size, has to be proposed with
  reasoning and confirmed by the owner.
- **"Leaving soon".** Exactly how it is computed (for example, the files that the next N MB of new
  data would push out, or everything past a fullness threshold), and the "nearly full" threshold.
- **Where the setting lives.** It is per server and per device. How it interacts with server
  livery, if at all.
- **Eviction and the rest of the system.** How eviction interacts with retained Studio sources,
  Keep reservations, in-flight Downloads, and the members-only serving path. What a peer sees when
  it asks for an evicted chunk.
- **Migration.** Existing caches have no listing-time index, so a one-time scan is needed.

## Verification

- Unit tests for eviction order, the pins, and the threshold and footer counts.
- Crash tests for a torn write, the rename and verify step, and recovery at mount.
- Leave-server cleanup tests that include Studio references.
- Footer UI tests and visual-fixture screenshots.
- The full suites, `test:startup` and `test:flows`.
- An adversarial review of the diff, as `AGENTS.md` requires.
