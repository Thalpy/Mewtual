# Changelog

All notable changes to Mewtual are documented here.

## [Unreleased]

### Added

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
