# Changelog

All notable changes to Mewtual are documented here.

## [Unreleased]

## [0.3.0-alpha.9] - 2026-08-28

### Added

- **Announcements became a noticeboard instead of a list of posts.** Posting belongs to the owner
  and admins by default, and a chip in the surface header opens the feed to every member for
  servers that want that; the switch rides on the replicated feed itself, the way the wiki's review
  window does, and is enforced by every member's app rather than by the group keys, the same
  residual as deleting a message. Posts now take reactions from anyone (chat's picker, server custom
  emoji included), an edit from their author carrying the edited mark chat already uses, a deletion
  from the author, the owner or an admin, and a pin from an owner or admin that lifts them into a
  highlighted block at the top of the feed. A second header chip reaches this server's news-cue
  switch without a trip through server settings; it is the same per-device preference that panel
  already held, and still nothing other members can see.
- **Announcements remember what you have already read.** A **NEW** divider sits above the oldest one
  you have not seen, the surface's tab carries the outstanding count, and reading the surface marks
  them read. The mark is per server and survives a restart, and it is rebuilt from the posts that
  exist rather than from events the app happened to be awake for, so announcements posted while the
  app was closed are still new when it opens. A post's timestamp is its sender's clock, so the mark
  refuses to advance past what this machine finds plausible: one wrong clock in a feed can no longer
  park the mark years ahead and silence every real announcement after it.
- **Inbox → News reads per item rather than per app.** Rows you have not seen are highlighted
  individually, off the same per-server read state the surface uses, in place of the single
  "something changed" flag that could only be on or off. Filter chips (All / Announcements / Events,
  plus a server picker), a **Pinned** section holding each server's latest pinned announcement, and
  a **Mark all read** button were added alongside it, and a row now jumps to the exact post and
  flashes it instead of opening the surface and leaving you to find it. The rail's inbox badge counts
  unseen announcements one by one. Events deliberately never raise it: an event's timestamp is its
  scheduled start rather than the moment it was written, so it cannot honestly be "unseen"; the
  ticker and the news cue still announce event changes.
- **A server with unread announcements says so from the orbit view.** Its billboard wears a
  herald: a slow ripple plus a small **◈ N** at the bottom left, the one corner the existing signals
  did not already claim (unread chat top right, mention flare top left, voice bottom right; the
  compass is unchanged). Hovering the billboard shows a card with that server's latest announcement,
  and clicking it still enters the server. With motion effects off the ripple renders as a static
  ring.

### Fixed

- **The News feed showed each server's five oldest announcements.** The aggregation took the first
  five posts of a newest-first feed and called them recent, so a server that had ever posted six
  times showed the same five forever and nothing posted since ever appeared there. It now aggregates
  the real feed.
- **An edit, a reaction or a pin on an announcement never reached anyone else.** Change detection
  compared how many posts a feed had, which an edit, a reaction and a pin all leave alone, so other
  members' views kept rendering the version they last happened to load. The comparison now covers
  what the posts say, not just how many there are.
- **A refused announcement disappeared without saying why.** A post the server would not accept was
  dropped in silence, taking the words with it and leaving no reason on screen. The refusal is now
  reported, and the draft stays in the composer.

- **A half-written announcement followed you into the next server, and stayed behind the lock
  screen.** Unlike a chat draft, which is kept per channel and restored to the one it was written
  in, the announcement composer held a single box that nothing ever cleared: switching servers
  showed server A's unfinished post in server B's noticeboard, and locking left it in memory
  along with it. It is now dropped with the rest of the group's contents, on the switch and on the
  lock.

- **The Linux build crashed while packaging the AppImage**, even though a full icon set already
  sat in `src-tauri/icons/`. Tauri's Windows installer gets its icon from `build.rs` embedding
  `icon.ico` directly into the exe, independent of `tauri.conf.json`; the Linux and Debian
  bundlers instead build their icon set purely from `bundle.icon` in that config, which this repo
  never set. With nothing to choose from, picking the largest square icon panicked outright
  instead of bundling with none. `tauri.conf.json` now lists the standard icon set (the same
  files already generated for Windows/macOS), so `--bundles deb,appimage` has an icon to use.

## [0.3.0-alpha.7] - 2026-08-22

### Fixed

- **A screen share died on everyone else's screen seconds after it started.** The picture appeared,
  froze on the first frames, and then dropped back to the avatar, while the person sharing kept
  watching their own preview and had no reason to think anything was wrong. A room sends a
  heartbeat every five seconds carrying who is muted, and that heartbeat had no field for who is
  sharing video. Whoever received it read the missing field as "they stopped", so the first
  heartbeat after a share began retracted it. Frames were still arriving the whole time; nothing
  was left to draw them. The heartbeat now carries the video state alongside the mute states, and
  a message that says nothing about video no longer counts as saying it stopped, which also means
  a share announcement that gets lost now repairs itself within five seconds instead of never.

- **Stopping a share and starting another one made the call heavier every time.** Each camera or
  screen share is supposed to travel in one reserved slot per person, reused for the whole call.
  Stopping one tore that slot out instead of parking it, and the replacement could not be reused,
  so every stop and restart left another dead video section attached to the connection and grew
  the negotiation between the two of you. A slot is now kept and refilled, so toggling a share
  costs nothing after the first one.

- **A screen share that followed a camera was sent at the camera's quality.** The two travel in
  the same slot, and the quality budget was only set when the slot was first opened, so turning
  your camera off and sharing your screen sent text and windows at a budget meant for a face.
  Sharing your screen to someone who joined while you were already sharing had no budget applied
  at all. Both now follow what is actually being sent.

- **Sharing a file took the whole app down with it.** An upload would stop at 10% and everything
  around it would stop responding until it finished, or looked like it never would. Two separate
  bottlenecks, both of which scaled with the size of the file. The file was handed to the desktop
  in one piece, so the window spent the transfer building and posting a single message the size of
  the file rather than drawing anything; and the server then encrypted and stored the whole thing
  in one uninterrupted step, during which it stopped talking to the group and stopped answering
  anything else you asked it. A share now moves in slices and is encrypted a chunk at a time, so
  the window keeps painting, the server keeps syncing, and the progress bar reflects work that has
  genuinely happened. An upload that fails or is cancelled cleans up after itself instead of
  leaving encrypted fragments behind, and one whose window went away (a reload, say) is now
  collected rather than holding space until you lock.
- **Saving a shared file to Downloads pulled it through the window twice.** The file was read out
  in full, handed to the window, and handed straight back to be written, so saving a large file
  froze the app for the same reason sharing one did. It now goes from the server to the file
  directly, checking as it lands that what was written is what was shared. It is written under a
  `.part` name and only becomes the real file once that check passes, so a failed or interrupted
  save never leaves something in Downloads that looks like the finished thing.

- **An upload interrupted by closing the app no longer leaves anything behind.** A share encrypts
  its pieces as it goes, and until the share is published those pieces belonged to nothing. If the
  app closed mid-share they stayed on disk invisibly, and nothing could tell them apart from real
  files afterwards, so they were never cleaned up. They are now written somewhere separate and only
  moved into place once the share completes; anything still sitting there when the app starts is
  from a share that never finished, and is cleared.
- **A large picture or video in a message no longer freezes the window.** Anything shared and shown
  inline (message images and video, custom emoji, event posters, link previews, the preview in a
  file's properties) was fetched whole and handed to the window in one piece before it could be
  drawn, so scrolling past a message with a big file attached could lock the app up with no warning
  and nothing clicked. These now stream as they play or load, the same way the jukebox already did.

### Security

- **A shared file can no longer claim to be small and arrive huge.** A file's listing says both how
  big it is and which pieces it is made of, and nothing required those two to agree. A member
  running a modified client could publish a listing that reads as one byte and expands to gigabytes
  when anyone saved or previewed it, filling their disk from what the app showed as a trivial file.
  A listing whose pieces do not match its stated size is now rejected outright, before anything is
  fetched or written, and a transfer stops the moment more data arrives than the file declared.

## [0.3.0-alpha.5] - 2026-08-22

### Added

- **The jukebox shows the film it is playing.** A shared video used to have nowhere to be unless
  someone opened the focus view by hand: the room heard a clip nobody could see. The deck now
  carries its own screen, with one press to watch it full size, and the focus view still takes the
  whole window when it is open.
- **Add from share** can be narrowed to audio or video, with a count on each tab, so queueing a
  film in a share full of music no longer means reading past all of it.

### Fixed

- **Adding to the queue could take the whole app down.** A share may list the same file more than
  once (in two folders, or twice in one after a simultaneous add) and the picker assumed it never
  would, so opening it over such a share crashed with `each_key_duplicate`. Each piece of content
  is now offered once, whatever the share calls it.
- **A track already on this disk could still buffer.** Every media response read the file's first
  chunk to learn a size and a type the file index already carried, which is a whole 8 MiB decrypt
  on the single-threaded server, and with a small plaintext cache that read evicted the chunk
  being played: each window threw away the chunk it needed to read it back again. Playing a file
  through now decrypts each of its chunks exactly once, and the deck stops competing with the
  rest of the app for the server it is reading from.
- The focus view no longer pushes a shared film past the bottom of the window. On a wide
  fullscreen display the video band claimed a 16:9 height off the full width and the filmstrip
  and dock went off the edge; the band now takes the space that is actually left and letterboxes
  inside it.
- **The jukebox plays on Windows again.** Shared tracks were requested through a URL scheme
  WebView2 has no handler for, so every track failed the moment it was pressed and the deck
  reported it as one nobody would serve. Windows now uses the host form the toolkit actually
  intercepts; macOS and Linux are unchanged.
- The DJ's own playback is now the room's clock. Time spent waiting for a track's first bytes used
  to count as playback, so the deck seeked itself past what it had never played, announced that
  position to the room, and did it again at every ping: a track that took a moment to arrive never
  got going, and every pause/resume jumped forward by the accumulated gap.
- The queue is consumed as it plays: a track the room has heard (or skipped) comes off it, instead
  of collecting behind a play head that only moves forwards. A track nobody could serve stays
  queued, so it can be retried when whoever holds it comes back.
- An idle deck has a play button. Reaching the end of the queue, or joining a room where nothing
  is playing, previously left no way to start the room short of clicking a named track.

## [0.3.0-alpha.4] - 2026-08-22

### Added

- **About & Licences**: Settings → Help now names what this build is, summarises Mewtual's own
  terms in plain language, and reproduces the full licence text of every open-source component
  Mewtual is built from.

### Fixed

- The licence's source-disclosure clause no longer reads as though it required relicensing
  third-party components that their own licences forbid relicensing.

## [0.3.0-alpha.3] - 2026-08-22

First release cut by the signed release pipeline: installed copies of this version and later
update themselves in-app. Copies installed from the hand-uploaded alpha.2 builds have no update
channel and need this installer once, by hand.

### Added

- **Moderation plane**: servers now have a signed event timeline with Shift-range message
  selection, public warning reasons, collapsible warned posts, evidence-backed kick cases,
  advisory member votes and owner-only removal. The timeline discloses its honest-client audit-log
  limitation; votes never grant membership authority. The plane is owner/admin-only and now opens
  with a per-user lane graph above the detailed scroll; ordinary members vote from focused chat cards.
- **Storage and connectivity operations**: paired sidebar controls open authenticated local storage
  verification/peer repair and an evidence-based connectivity assistant. Transfers repeats storage
  health so a corrupt local chunk is distinguishable from an unavailable peer. Storage verification
  runs once per server/process session and adds category, wiki-pin and largest-file inventory views.
- **Durable history and encrypted backup export**: composer drafts and read positions are sealed in
  the vault across restarts, including safe migration of legacy plaintext read marks. Settings →
  Backup & Recovery creates a coherent encrypted export in Downloads; staged restore remains
  deliberately deferred until it can verify and roll back from the locked screen. Export risks are
  explicit, and Vault & Lock can atomically change a passphrase/sigil/melody by rewrapping the DEK.
- **Searchable Feature Guide**: Settings now includes a complete feature directory with plain
  descriptions, exact UI locations, shortcuts and direct **Open** actions for chat surfaces,
  DMs, Inbox/News, Server Space and both settings areas. The repository README carries the same
  high-level map so substantial features are no longer hidden in handover notes.
- **Wiki page history**: every page keeps its revisions (who changed what, when), with a line
  diff against the previous version and one-click restore. Rollbacks, renames, deletions and
  review decisions are all recorded, so nothing is silently erased.
- **Wiki edit review** (optional, per community): an owner or admin can set a review window of
  1 to 30 days. While it is on, a member's edit becomes a pending change that an owner/admin
  approves or declines from a **Review** queue above the page list, and any change nobody
  reviews publishes automatically when the window lapses.
- **Nested wiki pages**: page names containing `/` (for example `Guides/Setup/Linux`) group into
  collapsible folders in the sidebar, so a large wiki is no longer one flat list.
- **Wiki infoboxes**: a `{{Infobox …}}` block renders as the summary card floated at the top
  right of a page, in both Markdown and Wikitext pages, with a toolbar button and in-app help.
- **In-app updates**: Mewtual checks GitHub for a new release a few seconds after launch and, if
  there is one, offers it in a small card with the release notes: **Update and restart** downloads
  and installs it, **Later** asks again next launch, and **Skip this version** retires it for good.
  Nothing is ever installed without that click, a failed check stays silent, and **Settings →
  Updates** has a manual check. Releases are minisign-signed and verified before installation, so a
  tampered or unsigned build is refused. Only official builds carry the update endpoint, so forks
  and builds from source are never offered someone else's binary.
- **File feedback straight to the tracker**: the 💬 feedback window now has a title field and a
  **File on GitHub** button that opens a prefilled bug report or feature request (labelled `bug` or
  `enhancement`, with your app version and environment filled in) in your browser. Mewtual posts
  nothing itself: you review the form and submit it, so the issue is yours and maintainers can
  reply to you. **Copy report** is still there for anyone without a GitHub account.

## [0.1.0-alpha.1] - 2026-08-18

Mewtual's first public alpha. This release is intended for testing with people you trust, not
for protecting sensitive or high-risk communications.

### Included

- Peer-to-peer, end-to-end encrypted, invite-only communities without user accounts.
- Group chat with channels, replies, reactions, pins, mentions, editing, search, and Markdown.
- Shared wiki pages, files, media, profiles, status posts, events, and community theming.
- Multiple communities in one desktop app, encrypted local storage, and offline history.
- Direct TCP connections and ciphertext-only relay support.
- A Windows x64 installer carrying the Mewtual app icon.

### Known limitations

- Mewtual has not received an independent security audit.
- The Windows installer is not code-signed, so Windows SmartScreen may show a warning.
- Desktop rendezvous discovery is incomplete; internet connections may require port forwarding
  or a separately operated relay.
- At least one reachable member must be online for another member to join or synchronise.
- Forgotten local passphrases cannot be recovered.
- Packaging and testing on macOS, Linux, and mobile are not part of this alpha.

[0.1.0-alpha.1]: https://github.com/Thalpy/Mewtual/releases/tag/v0.1.0-alpha.1
