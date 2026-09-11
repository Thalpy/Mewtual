# Changelog

All notable changes to Mewtual are documented here.

## [Unreleased]

### Added

- **Chat recognises five more services.** A link on a line of its own from **SoundCloud, Vimeo,
  Mixcloud, Apple Music** or **Bluesky** now opens out into that service's player, alongside the
  Spotify and YouTube cards. Every one is gated identically to the two that came before: inert
  until permitted, and mounted only while it is on screen in a visible window. Providers are now a
  small table rather than a pair of special cases, so each is a parser and two URL builders with no
  ability to change any of that. A Bluesky link only becomes a card when it carries the author's
  DID; the embed rejects handles, and turning a handle into a DID would mean contacting Bluesky
  about a link that merely scrolled past, so those stay ordinary links.
- **The jukebox can queue SoundCloud and Vimeo as well as YouTube.** The LINK tab (formerly
  YOUTUBE) takes any of the three, and the room shares one transport across all of them as before.
  Those three and no others because keeping a room together needs a player that will take a seek
  and report where it has got to: Spotify, Mixcloud and Apple Music do not offer both, and pasting
  one now says so rather than queueing a track nobody can play. A SoundCloud track correctly plays
  without claiming the call's video surface, which a 166-pixel audio strip has no use for.

- **Settings → Chat & Media can load player cards without asking.** Off by default, device-wide
  rather than per-server, and sealed in the vault with the other content preferences, so it is not
  readable from disk and is dropped when the app locks. Read what it covers before turning it on:
  it applies to every allow-listed embed host in `tauri.conf.json`'s `frame-src`, currently seven
  (`open.spotify.com`, `www.youtube-nocookie.com`, `w.soundcloud.com`, `player.vimeo.com`,
  `player.mixcloud.com`, `embed.music.apple.com`, `embed.bsky.app`), and any host added to that
  list later is covered by the same switch without being asked for again. Each of those companies
  learns this device's address and what it is looking at for every such link that scrolls past. It
  replaces the click and only the click: an auto-loaded card still exists only while it is on
  screen in a visible window, so the setting buys fewer interruptions rather than frames running
  where nobody is looking. It reads as off whenever the sealed record is unreadable. Jukebox
  playback of a linked video keeps its own per-track approval regardless.

### Security

- **`frame-src` now admits seven embed hosts, not two.** SoundCloud, Vimeo, Mixcloud, Apple Music
  and Bluesky join Spotify and YouTube, so the list a previous release described as "exactly
  `open.spotify.com` and `www.youtube-nocookie.com`" is out of date. Nothing else about the policy
  changed: still no third-party **script** origin, frames still sandboxed without
  `allow-top-navigation`, and what a frame reports back can move only the local player. Each host
  is still click-to-load by default, but the **load these cards without asking** preference above
  is a single switch over the whole list rather than a per-service one, so an existing "yes" now
  covers five more companies than it did.

### Fixed

- **A channel could stop catching up and never finish.** When you reconnect, your client tells a
  peer what it already holds so the peer can send only the rest. It could name at most 64 points in
  a channel's history, and a channel that had branched more widely than that could not describe
  itself: the peer subtracted too little and re-sent history you already had, then did it again on
  the next round, forever. Two changes fix it. The limit is now 512 points, which is past any
  realistic group and still a fraction of the frame budget. More importantly, catch-up now sends
  pages from a remembered position instead of recomputing the whole difference each time, so a
  duplicate is consumed rather than offered again and every exchange makes progress however wide
  the history is. Old and new builds still talk to each other: the position is an optional field,
  and a build that does not send one is answered exactly as before.
- **A member that has fallen too far behind on membership changes is now detected instead of
  looking healthy.** If more removals have happened than any peer you can reach still keeps a
  record of, your client cannot advance its routing label and quietly ends up listening on topics
  nobody publishes to, while every check reports it as up to date. That case now has a name and a
  count, and the repair task no longer retires itself after such an exchange, so a peer with a
  longer record still gets asked. This is detection only: nothing yet repairs the gap for you, and
  what is reported is what the peers actually reached could offer, never a claim about the group.

- **Joining a jam or jukebox deck part-way through a take no longer starts with silence.** Seeking
  into a recorded take began at the next event past the offset, which threw away every note that
  was already sounding: you heard nothing for the length of a held chord, then a run of note-offs
  for voices that had never opened. Seeking now works out what the take is holding at that point
  and revives those notes at the level they had reached, rather than re-striking them. Drums are
  deliberately not revived, because firing a fresh crash whose tail happens to cross the seek point
  would add a hit the take does not contain.
- **The shared-patch drawer no longer merges two different people's recipes into one tile.** It was
  keyed by content address, and two people who build the same recipe produce the same bytes, so one
  tile silently stood in for both. Both are kept now.
- **SAVE no longer destroys your oldest recipe.** The saved list kept the last twelve by quietly
  dropping the front of it, so the thirteenth save deleted a recipe with no warning, and so did a
  shared patch that arrived and was kept without being selected. Saving at the cap now refuses and
  says so. Overwriting a name you already used still works, because that is what typing it means.
- **The jam filter's neutral setting is labelled WIDE, not OFF.** There is no bypass: every voice
  runs through a lowpass, and at the top of the range it is simply open as far as it goes (lower
  still where the sample rate forces it). The old label and tooltip claimed the tone passed through
  untouched, which was not true. The sound is unchanged; only the label is honest now.
- **The patch editor says when a change has actually taken effect.** Picking a preset marked it live
  immediately, but a frame may not leave for up to the debounce plus the announce interval, so a key
  pressed straight afterwards played the previous sound and the editor looked broken. A LIVE /
  APPLYING chip now reads the bytes that were actually published, and picking a discrete option
  skips the debounce that exists for dragging a slider.
- **A Studio document that needed a third recovery slot could sit waiting forever.** The seven-day
  grace after which a stale recovery warning is evicted was written but never actually applied, so
  the document stayed in Closing until somebody pressed Acknowledge, which is exactly what the
  grace exists to avoid. The deadline is now enforced on all four settlement and adoption paths,
  and Acknowledge only brings the eviction forward. None of this is something you can see yet: the
  Studio backend is not wired up to the app, so nothing the Studio tab shows you is running on it.

### Changed

- `docs/MESSAGE-FLOW.md` is new: it traces how a message travels from send through gossip to
  catch-up, and corrects two claims about relaying that were wrong in the older documents.

## [0.3.0-alpha.18] - 2026-09-10

### Fixed

- **A file that showed as unavailable can now repair itself.** Uploading checked whether the same
  content was already stored and reused it without confirming the local bytes were still intact, so
  a damaged or partly-removed copy could be published as if it were whole and then fail for
  everyone. Uploads now verify what they are reusing, repairing keeps the fresh encrypted chunks it
  fetched, and ownership and expiry survive the repair. Downloads, previews and the file list also
  now agree on which encrypted variant of a file they are talking about, instead of each deciding
  separately.
- **Background file fetching is bounded, and a copy you already hold is kept.** Fetches run at most
  four at a time per server and eight per app, with deadlines, and they are cancelled properly when
  you leave. A file confirmed present on this device is retained and re-verified rather than pulled
  down again. The file list is also more careful about what it claims: **Cached here**, **Partial**,
  **Remote copy unconfirmed** and **No connected provider** are separate states, because a member
  being online has never been evidence that they hold a particular file.
- **The release workflow can no longer split one release in two.** Publishing the draft before the
  Linux job finished left it unable to find the draft, so it created a second release whose update
  manifest named only Linux. That is what happened to v0.3.0-alpha.17. The Linux job now uploads by
  release id, and a third job reads the finished release back and fails the run unless every bundle,
  every signature and both platforms are on it.

### Changed

- **Almost all of this release is work you cannot see yet.** The Studio and Flipnote backend reached
  its fourth gate: canonical operations, deterministic projections, causal validation of index and
  frame edits, typed checkpoints, durable exchange of saved art, bounded paging, checkpoint
  discovery and adoption, owner rotation and recovery inspection. The saved-registry work alongside
  it gained durable paging, recovery-first checkpoint installation, owner rotation from durable
  decisions and receipt completion. None of that backend is reachable from the app: it is wired to
  nothing, and the creative suite's screens call no backend command. The screens themselves *are*
  reachable, and always have been: **Studio** is in the sidebar, in the quick switcher and on
  Ctrl/Cmd+8, and it opens and draws. Treat it as a preview only. It runs entirely on data held in
  memory, nothing you make there is shared with the group, and it is discarded rather than
  saved: close the tab or the app and the work is gone, with no warning and no way to get it back.

## [0.3.0-alpha.17] - 2026-09-07

Everything listed under 0.3.0-alpha.16 below ships in this build. That version was prepared but
never tagged or published, so this is the release that carries it, together with the following.

### Added

- **The start screen was rebuilt around the two things you actually do there**, joining with an
  invite and founding a group, laid out side by side with the identity and file-trust choices
  beside them rather than buried below.
- **A join now draws the route it is taking while it runs.** The invite, the direct dial, a relay
  circuit, a member switchboard, the two-way reply and the admission each appear as a row that
  turns active, succeeds, fails or is skipped, with the verdict naming which one it stopped on.
  Rows are named by kind and never by address or by person: the addresses stay in the connection
  check for whoever wants them, and nothing here names your inviter or a member who helped. Nothing
  is claimed that was not recorded, so a dial reads as "tried" until something says how it ended.
- **File trust's middle setting is now "media", and it is the default.** Images, audio and video
  from the group load as you scroll; documents, archives and anything not recognised as media still
  wait for a click. It replaces the old "specific" setting, which now reads as on-demand with its
  trusted list intact, so upgrading changes nobody's effective choice.
- **You can override file trust for one person**, in either direction: always fetch that person's
  attested files even under on-demand, or keep them click-only even under everyone. Where the two
  disagree the block wins, because that is the one that fails closed.
- **Messages that arrive behind where you had read get an "arrived late" divider** and a jump from
  the header, and they stay marked until the row has actually been on screen in a focused window.
  The marks are stored with your read position, so a restart keeps them. Per server, on by default,
  under Appearance.
- **Each shaping stage in the jam patch editor has an OFF.** Envelope, filter and sends can be
  switched off and back on, and what the stage held is remembered so switching it back returns the
  sound. "Off" is written as an ordinary neutral value rather than a flag, so a patch stays exactly
  what the format already allowed.

### Fixed

- **The jam's chorus, echo and reverb were close to inaudible.** A send at 100 meant half of the
  voice, which then met each effect's own quiet return: a maxed knob sat roughly 16 dB under the dry
  sound, so the controls moved and nothing changed. A send now means all of the voice, and the three
  effect returns were raised to levels where each reads as the thing it is named after. The room is
  still deliberately a small one.
- **Notes no longer click when a patch has no release.** A release of zero stepped a sustained
  waveform straight to silence, which is loudest on exactly the settings people choose when they
  want a hard gate. There is now an 8 ms floor, taken from inside the existing safety window rather
  than added to it, so it can never extend a voice past what that window already allowed.
- **A send is accepted when the message is written, not when it finishes publishing.** If publishing
  was refused after the message had already been added to the channel, it was reported to you as a
  failed send while the message sat there untracked. It is now queued in the bounded outbox and
  retried.
- **The right-click menu opens on the pointer instead of sliding over it.** Near an edge it mirrors
  to the other side of the click, and it is squeezed against the edge only when neither side fits,
  which is what it always used to do.

### Changed

- Registry recovery gained its durable groundwork: local edits are prepared durably with exact
  retries, settlement inputs are receipt-bound, checkpoints are installed after recovery rather than
  before, and saved intents replay through checked recovery. Backend only, with no user-visible
  effect in this release.

## [0.3.0-alpha.16] - 2026-09-07

### Added

- **Releases now carry Linux builds.** The release workflow adds an `.AppImage` and a `.deb`
  alongside the Windows installer, signed with the same updater key and listed in the same
  `latest.json`. The AppImage updates itself the way the Windows build does; a `.deb` install has
  to be replaced by hand, because Tauri's updater has no format for it.

- **Spotify and YouTube links in chat open out into player cards.** A link on a line of its own
  becomes a chip naming the service, and clicking it is what contacts them; a link written inside a
  sentence stays an ordinary link. Nothing loads on its own by default, because a player frame has
  no file attestation and discloses this device's address to the service. Unlike a remote image it
  also keeps running once loaded, so a grant to load is not the end of it: a card exists only while
  it is on screen in a visible window, and reverts to its chip when you scroll away, switch tabs or
  minimise. The frame is built in code from a parsed id and never from a member's text, so chat
  markup still cannot create one, and the share-tracking token on a Spotify link is dropped rather
  than passed on.
- **The jukebox can queue a YouTube video by link**, from its own tab beside Audio, Video and
  Takes in **Add from share**. It plays on the deck's own screen like any
  other video and follows the same shared transport, so play, pause, skip and seek still move the
  whole room together. It differs from a queued file in the way that matters: nothing is shared
  through the server, so each listener fetches it from Google themselves and each is asked before
  their own player loads. A queued link is a claim by whoever queued it; no device checks it on a
  peer's behalf, because checking would be the disclosure. Drift is corrected by seeking rather
  than by the gentle rate easing a shared video gets, since the embedded player offers no usable
  rate control.
- Spotify is deliberately **not** a jukebox source. Its embedded player needs a Premium session
  signed in to `open.spotify.com` in this webview plus a working Widevine decoder, neither of which
  a desktop webview has, so it plays a preview of about thirty seconds or refuses outright. A room
  cannot listen together to that, so Spotify stays a chat card.

### Fixed

- **The jukebox deck's player now fills its surface.** Four style rules, including both that size
  the deck's picture, were written with Svelte's global escape hatch inside `app.css`. That file
  is a plain stylesheet, so nothing compiled the syntax away: it reached the browser verbatim,
  which discarded each rule as invalid and left the player at an element's default 300x150 in the
  corner of a 16:9 box. Nothing warned, and the source read as correct. A source-hygiene test now
  keeps that syntax out of standalone stylesheets.
- **YouTube players no longer fail with "Error 153".** The frames were sent with a `no-referrer`
  policy, and YouTube declines to configure a player embedded by nobody. They now send the app's
  origin and never a path. The `origin` parameter is also omitted unless the window is served from
  a web origin, since Tauri uses the custom scheme `tauri://localhost` on macOS and Linux and
  naming that produced the same refusal there.

### Security

- `frame-src` now admits exactly `open.spotify.com` and `www.youtube-nocookie.com`, and nothing
  else changed: no third-party **script** origin is allowed, so the jukebox drives its YouTube
  frame over postMessage rather than loading the provider's API script into the app's document.
  Frames are sandboxed without `allow-top-navigation`, and what a frame reports back (position,
  player state) can move only the local player, never the room's transport.
- A queue entry names exactly one source. The channel-document reader skips any entry carrying an
  unknown source, a malformed link, or both a content address and a link, rather than picking one
  of two claims that disagree; the call-transport frame applies the same rule at the wire edge.
  Unplayable tracks are now remembered by deck address rather than by content address, so one dead
  video no longer silently drops every other linked track from the queue.

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
