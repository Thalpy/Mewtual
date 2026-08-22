# Changelog

All notable changes to Mewtual are documented here.

## [Unreleased]

### Fixed

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
