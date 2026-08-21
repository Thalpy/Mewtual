# Mewtual; User Guide

Mewtual is a **peer-to-peer, end-to-end-encrypted, invite-only** group chat. There are
**no accounts and no central servers**; you talk directly to other members, and every
"server" is its own encrypted group that lives on the members' own devices.

This guide covers the **desktop app** (`apps/desktop`).

---

## 1. Running the app

**From a release build (to share with others):**

```sh
cd apps/desktop
npm install
npm run build
npm run tauri build -- --bundles nsis
```

Send the installer beneath `src-tauri/target/release/bundle/nsis/`. The unsigned alpha may
trigger a Windows SmartScreen warning. The recipient needs the **WebView2 runtime** (built into
Windows 11; a free installer for older Windows). Use `--no-bundle` instead only when you
specifically need a local portable executable.

> **Don't share a debug build** (`target/debug/…`). A debug build loads its UI from a local
> dev server and shows a "can't reach the page" error on any other machine.

**For development:**

```sh
cd apps/desktop
npm install
npm run tauri dev
```

To test with two members on one machine, run a second instance: launch the built exe
directly (`./src-tauri/target/debug/mewtual-desktop`) in another terminal while the dev
server is running.

---

## 2. First launch & your vault secret

On launch, Mewtual asks for a **vault secret**. This isn't an account; it is the input that
**encrypts everything on this device at rest**. A passphrase is the recommended option; the
setup screen also offers a drawn multi-factor **sigil** or a played **melody**.

- **First run:** the secret you enter *becomes* your vault secret. There is **no recovery**; if
  you forget it, the local data is unreadable.
- **Later runs:** enter the same secret to unlock. Your servers, channels, history,
  profiles, files, wiki, and roles all come back; **readable offline**, even before any
  other member is online.

Everything on disk (group state, messages, files, avatars) is sealed under this passphrase
(Argon2id + XChaCha20-Poly1305). A stolen laptop or a copied app-data folder is opaque
without it.

Use **Settings → Vault & Lock → Change vault secret** to authenticate with the current passphrase,
sigil or melody and choose any new method. The change atomically rewraps the same random root key
under a fresh Argon2 salt; it does not decrypt or bulk-rewrite server data. Older backups are not
revoked and continue to require the secret they had when they were exported.

To add another computer or device without pretending both are the same cryptographic device,
open **Settings → Devices → Link a new device**. The two devices compare a short code; the grant
can travel by paste, QR or sound. The new device receives its own key and joins each granted
server when that server's owner is online to serialize the admission safely. Owners can inspect
linked devices under **Server settings → Devices**, and you can revoke your own companions there.

Use **Settings → Backup & Recovery** to create a coherent encrypted copy of the unlocked vault in
your Downloads folder. The copy remains protected by the same vault secret, so keep that secret
separately. No plaintext is exported, but the new copy is an additional offline guessing target and
exposes filesystem metadata such as names, sizes, timestamps and layout; it also preserves old state
and keys even if they are later deleted from the live vault. One-click restore is deliberately not
offered yet: replacing a live vault safely needs a locked-screen, staged verification and rollback
flow. Keep the exported folder intact until that flow lands.

---

## 3. Quick start

1. **Found a server**; type a display name, click **Found a server**.
2. **Invite a friend**; right-click the server icon → **Server settings → Invites** →
   **Copy invite** (or use the invite button under the member list). Invites are single-use;
   click **Generate new invite** whenever you need another one.
3. **They join**; your friend opens Mewtual, sets a passphrase, pastes the invite into the
   join box, clicks **Join**.

You're now in a shared, encrypted `#general` channel. Type and send.

---

## 4. The layout

- **Left rail**; your servers (one icon each), **DMs**, the cross-server **Inbox / News** feed,
  **Server Space** (the orbit icon), **＋** to add one, and **💬** to send feedback
  (a bug report or feature request: **File on GitHub** opens a prefilled issue on the tracker in
  your browser for you to review and submit, and **Copy report** puts the same text on your
  clipboard if you would rather send it another way) + **⚙** for Settings.
- **Updates**; a few seconds after launch Mewtual asks GitHub whether a newer release exists. If
  one does, a card appears bottom right with the release notes and three choices: **Update and
  restart**, **Later** (it asks again next launch), or **Skip this version** (it never asks about
  that one again). Nothing installs unless you press Update, and **Settings → Updates** can check
  on demand, including for a version you skipped.
- **Sidebar**; the server's channels and its member list (with role badges). Owners/admins also have
  a dedicated **Moderation** button. Compact **Storage** / **Connectivity** controls form one bottom
  stack immediately above your profile; DMs omit the server-governance controls.
- **Main pane surfaces**; **Chat · Files · Announcements · Wiki · Profile · Transfers · Events**,
  plus the operations surfaces opened from the sidebar.
  Click one to switch. If that list feels like a lot, **Settings → Feature Guide** is a searchable
  map with direct **Open** buttons.
- **Top-bar ticker**; new announcements, wiki changes, events, and messages that notify you crawl
  across the full space between **Forward** and **Minimise**. Each item appears only once in an
  unlocked session. Hover or keyboard-focus a headline to pause it; click it to open the exact
  item (including switching server/channel and flashing a message). Non-message headlines use the
  short three-pip news cue configured under **Settings → Notifications**.

### Server Space

- Open the 360° server room from the rail's orbit button or with **Ctrl/Cmd+O**. Drag empty
  space to look around; drag a server to move it; hold on empty space and draw a loop to lasso
  several servers. Drops make room automatically, so icons do not overlap.
- Create named **neighbourhoods** in **Settings → Server Space**. Every neighbourhood then
  appears in the panel inside Space, even while it is empty. Press its **＋** to add or remove
  servers, click its name to look at it, or drag a server directly onto its panel row or floating
  label. Adding an unplaced server hangs it in that neighbourhood automatically.
- Press **T** for the server tray; its per-server neighbourhood menu is another way to assign
  one while arranging the room. **Arrange neighbourhoods** gives each group its own region.
- Entering a server uses a short focus-and-zoom transition with an original painted-portal
  sound. Both the zoom and the entry sound can be switched off independently in
  **Settings → Server Space**; the sound also follows the app's global sound preference.

### Right-click menus & keyboard shortcuts
- **Right-click** almost anything for quick actions: a **message** (copy text, quote, copy
  sender), a **wiki page** (open, **post a link to it in chat**, copy link), a **file** (open
  details, download, post to chat, copy address, delete), a **member** (copy fingerprint,
  make/demote admin, remove; owner), or a **server icon** (copy invite, settings, leave).
- **Keyboard**: **Enter** sends, **Shift+Enter** adds a newline; **Esc** closes the topmost
  menu/panel; **Ctrl/Cmd+1–7** switch surfaces; **Ctrl/Cmd+K** opens the quick switcher for
  channels, surfaces, servers and DMs; **Ctrl/Cmd+L** locks; **Ctrl/Cmd+O** opens Server Space.
- The **member list** and **wiki page list** get a search box once they grow; server icons show
  an **unread badge**.

### Searching a conversation
- **Ctrl/Cmd+F** (or the 🔍 in the channel header) searches the open conversation. Matches
  highlight in place; **Enter / Shift+Enter** (or ↑ / ↓) step through them, with an _n / m_
  counter. **Esc** closes it.
- **Filters** (the button in the search bar, or **Ctrl/Cmd+Shift+F** to open straight into it)
  narrow it further; they combine, and they work **with or without** any typed text:
  - **In**: this channel (the default), **all channels** in this server, or one you pick.
  - **From** a member · **Mentions** a member; both are type-to-filter boxes: start typing a
    name, pick with ↑ / ↓ + Enter or a click, empty the box to drop the filter.
  - **When**: **After** / **Before** a date, with **Today** / **7d** / **30d** shortcuts.
  - **Has**: image · video · audio · file (a non-media attachment) · link.
  - **Is**: reply · has replies · pinned · edited · mentions me · from me.
  - **Reactions**: any · ones you reacted to · a specific emoji.
  - **Match**: **Aa** for case-sensitive, **Whole word** to stop `cat` matching `catalogue`.
- **Sort** the results **oldest / newest first**, by sender **name (A–Z)**, by **most reactions**
  or by **most replies**. The result list under the filters shows each hit (channel, sender,
  time, snippet); clicking one jumps to it; **including into another channel**, which keeps
  the search open so you can carry on stepping. ↑ / ↓ follow the same order.
- Search covers the messages **loaded** for the conversations in scope; the backlog you've
  pulled, not the whole of history that other members still hold. Channels other than the open
  one are read once when you set the scope, so a message posted there mid-search shows up after
  you reopen the search.
- Mentions are matched on the **name in the marker**, so a member who has since renamed matches
  under the name they were mentioned by.

### Channels
- Every server starts with **#general**. Create/join a channel by typing its name into
  **join #channel…**. Anyone who opens the same name lands in the same channel.
- A **dot** marks unread. Opening a channel pulls its **backlog** from another member.
- Long channels initially mount only the latest messages so scrolling and new arrivals stay
  responsive. Scroll to an edge or use **Load older/newer messages**; search, reply and unread
  jumps mount a bounded window around their target automatically.
- Per-channel drafts and read positions are encrypted in the local vault. They survive a normal
  restart, are cleared from memory when you lock, and are never copied into plaintext browser
  storage. This continuity is local to this device; it is not synced to other members.

### Profile message studios
- Custom message frame backgrounds are temporarily disabled in live chats while their rendering
  cost is investigated. The studio, preview, and saved profile choice remain available, so no
  configuration is lost; message arrival choices are unaffected.
- Open **Profile** (or **Settings → My Profile**) for the **Message Frame Studio**. Pick a
  surface and one of five chassis, then build an ordered stack of scan, pulse, trace, and flicker
  layers. Each layer can be tuned, reordered, temporarily disabled, reset, or removed. Scan uses
  one shared top-to-bottom sweep across the visible message stack and appears only inside frames
  whose authors enabled it; Trace follows the bottom edge of the message box. Tune opacity and
  signal-edge strength while the side preview renders the same layered chat
  markup. Consecutive messages from you share one continuous frame without overlapping layers,
  and the frame adds only a small inset around the text rather than widening the message lane.
- **Message Arrival Studio** on the same card chooses how newly received posts from you arrive:
  still, glide, fly in, pop, or drift. Duration, travel or scale depth, starting visibility, entry
  direction, and response curve are configurable. It affects only genuinely new rows from your
  profile, not another member's posts or history when a channel opens. The side preview loops the
  draft and offers **Replay** so the short entrance can be checked immediately.
- These choices travel with your server profile. Under **Settings → Appearance**, you can disable
  peer frames or arrival motion locally. Flattening hides other members' frames while leaving your
  own visible; disabling arrivals suppresses every member's entrance locally. Neither control
  changes what other members see.

### DMs, friends, Inbox & News
- Open **DMs** from the left rail to create a private 1:1, redeem a friend code, sort friends by
  recent/activity/reconnect, or accept an authenticated friend request from someone you share a
  server with. A DM is its own encrypted two-member server, keeping that identity unlinkable from
  your other communities.
- Open **Inbox** for every message that mentions you or replies to you across all servers and DMs.
  Switch it to **News** for recent announcements and upcoming events across your communities.

### Notification sounds

- **Settings → Notifications** has a device-wide sound master plus separate **Messages**,
  **Mentions & replies**, and **News ticker** switches. Each category can use its built-in cue or
  a local custom MP3/WAV/OGG/WebM/M4A/AAC/FLAC file (up to 8 seconds / 384 KiB).
- Open **Server settings → Notifications** to leave each category on **Inherit global**, force it
  **On/Off for this server**, choose the built-in cue, or import a custom tone just for that
  server. Both panels show the effective enabled/disabled state and tone source.
- Voice-room banners remain individually switchable per server on that page and use the server's
  effective **Mentions & replies** tone. All custom files and preferences stay on this device;
  they are never sent to other members.

### Rich text, links, emoji & text effects
- **Markdown** works: `**bold**`, `*italic*`, `` `code` ``, lists, `> quotes`, links.
- **`[[Page Name]]`** links to a wiki page, and **`[[Page Name|shown text]]`** links it under
  different text (click to open; a **red** link means it doesn't exist yet; click to create it).
- **`:code:`** inserts a custom emoji (see Files → emoji below). The 😀 button in the
  composer opens a picker.
- **Link cards**; a reference that sits **on a line of its own** unfurls into a small card
  showing what it points at: the opening of a wiki page, a file's size and who shared it, a
  announcement's text, an event's time and description, plus a picture where there is one. A
  reference written *inside* a sentence stays an inline chip, so your prose isn't broken up
  (and a bulleted list of links stays a list).
  Clicking a card opens the thing it describes; right-clicking one offers its actions.
- **Text effects** work in chat (including edits), profile bios, announcements, wiki prose,
  and event titles/details. Highlight some words and the floating **Aa** strip appears above
  the selection. Its previews show the real effect; hover one for a plain-language explanation,
  click it to apply, or use **+** for the complete searchable catalog. The **Aa FX** button beside
  each editor opens that catalog even when nothing is selected.
- The copyable form is `[fx:cyber]signal online[/fx]`. The catalog includes shaky and wavy
  motion, rainbow sparkles, one-shot Speakese that pops in letter by letter with phoneme-varied
  voice blips, The Red Truth's sharp metal strike and rising/falling noise-wash entrance,
  entry-shed petals on Perfect Cherry Blossom, angry flame, gloom, cyber and CRT signals,
  click-to-reveal censorship, and a broad set of named pride flags. The catalog's copy button
  makes markup easy to move between
  surfaces without memorising it. Existing `[fx:animalese]...[/fx]` posts still render as Speakese.
- Under **Settings → Appearance → Message text effects**, choose **Full**, **Low**, or **Plain**.
  Full effects can animate, respond to the pointer, and play quiet effect audio. Low keeps static
  colour/type cues but has no movement, pointer response, petal shedding, or effect audio. Plain shows
  ordinary text. Your operating-system reduced-motion setting forces Low, and censored text
  remains concealed in every mode until you reveal it.
- The ten original quick effects have defaults from **Alt+Shift+1** through **Alt+Shift+0**;
  **The Red Truth** uses **Alt+Shift+R** and **Perfect Cherry Blossom** uses **Alt+Shift+C**. Open
  **Settings → Keybinds → Text-effect shortcuts** to change, clear, or reset any effect's binding,
  including every pride variant. Bindings are local to this device and require Ctrl/Cmd or Alt,
  so ordinary typing is never captured.

### Sharing media (drag-and-drop embeds)
- **Drag an image, video, or audio file onto the chat box** (or use the 📎 button) and it
  **embeds inline** in your message. The same works in the Announcements composer and the Wiki
  editor. Embedded files are stored in the fileshare (under `embed/…` or `wiki/<page>/…`).
- **Click an embedded image to view it full screen**; Esc or a click outside closes it, and
  **Actual size** switches between fit-to-window and 1:1. **Right-click** an image for
  **Properties** (size, type, who shared it, where it's used), **Download**, its address, and
  the message's own actions.

---

## 5. Connecting across a network

When you found a server, open the **Network (optional)** section. How others reach you:

| Where the other person is | What to enter |
|---|---|
| **Same machine** (two windows) | Nothing; leave it blank. |
| **Same Wi-Fi / LAN** | Your **LAN IP** (e.g. `192.168.1.5`) in *Reachable address*. |
| **Over the internet (port-forward)** | Your **public IP** (or `host:port`) in *Reachable address*, and forward that TCP port. |
| **Over the internet (automatic)** | Leave the network fields blank; Mewtual tries UPnP, IPv4 PCP/NAT-PMP, and IPv6 PCP firewall pinholes on its stable port. |
| **Over the internet (mapping unavailable)** | A **relay**; see below. |

Find your LAN IP with `ipconfig` (Windows) / `ip addr` (Linux). The invite carries every
address you advertise; the joiner dials all of them and the one that works wins. If you leave
the fields blank, Mewtual also gives UPnP IGD, IPv4 PCP/NAT-PMP and IPv6 PCP a chance to obtain
TCP and UDP/QUIC routes from a cooperative home router. On IPv6, PCP opens the router firewall for
the exact global listener address rather than translating it. This is best effort: a mapping still
needs AutoNAT verification, and double NAT, CGNAT or an upstream firewall may keep it unreachable.
The Connectivity assistant shows which protocol/transport answered and preserves its failure
reason (including a non-public/CGNAT upstream address or a missing IPv6 gateway). “IPv6 pinhole
granted” is deliberately not “reachable”: the host firewall, an upstream firewall, or the other
peer's IPv6 connectivity may still fail. When a lease expires, its verified mapping state and any
mapping-derived/NPTv6 address are withdrawn from the live bootstrap and peer record. A matching
global listener address can remain as an unverified candidate because the IPv6 address itself is
still configured even though its router pinhole closed. The next invite displayed reflects that
current state. A signed code
you already copied cannot be rewritten, so relay/rendezvous fallbacks still matter.

The server's network identity and port persist across restarts. Opening **Server settings →
Invites** refreshes an unused invite when its reachable-address set has changed. Generate a new
invite after the previous one is consumed, or whenever you want to deliberately replace it.
Ordinary founder invites expire after one hour; standing switchboard routes inside an assisted
invite expire much sooner.

After joining, the original inviter is no longer the only route. Mewtual keeps the newest signed
IPv4/IPv6 candidate record for each member, polls a bounded sample of known connections through
authenticated peer exchange, and retries unavailable members with increasing randomized delays.
A newly signed address update is tried immediately. Any reachable member can supply the current
records for the rest of the group, so reconnecting through one member is enough to start healing
the address book and catching up missed messages. “Offline” in the roster means this device has no
live authenticated connection from your current vantage point; another member may still reach it.
About once per discovery cycle, Mewtual also rechecks the IPv4/IPv6 addresses selected by the local
network. Moving between networks or receiving a new dynamic address therefore republishes a fresh
signed record without restarting the app, although detection may take roughly a minute.

Mewtual does not keep probing every historical public IP forever. Dynamic addresses can be
reassigned to another subscriber, so the latest signed record replaces withdrawn addresses while
retaining all IPv4/IPv6 routes that member currently advertises. If every member has only stale
routes and there is no reachable rendezvous, relay, switchboard or out-of-band reply, the group
cannot discover a new address from nothing.

### Two-way connection reply

If none of the inviter's routes answers, the join screen can produce a
`mewtual-reply-v1:` code for the next 60 seconds. Send it back through the same human chat and keep
both applications open. The named inviter can paste it under **Server settings → Connectivity →
One-time connection help**; an eligible current member that is already connected to that inviter
can do the same. Mewtual validates at most four public TCP/QUIC candidates and repeatedly dials
them. Every callback must prove possession of the code before the joiner sends its bearer invite or
KeyPackage, and only the invite's named inviter can sign the Welcome.

This is signalling and NAT punching, not a relay. It often helps QUIC through ordinary home NAT,
but requires overlapping 60-second sessions and cannot reliably cross symmetric NAT or CGNAT.
Anyone who saw the original bearer invite can also make a valid reply, so replacing a different
active joiner requires explicit confirmation.

### Member switchboards

An established group can use a reachable current member as a short admission bridge without a
Mewtual-operated server. Hosting is **off by default** and is enabled per device/server under
**Server settings → Connectivity → Group hosting**. A standing host publishes a two-minute signed
candidate offer. Fresh assisted invites can endorse up to three such members; when a recipient
pastes one, Mewtual previews the fallback and asks permission before contacting any member after
the direct attempt fails.

The switchboard forwards only the bounded admission exchange to the invite's named inviter, then
applies the same MLS Add before it becomes the new member's first encrypted sync path. It cannot
admit someone or forge/read the inviter-signed Welcome. It is already a group member, so helping
grants no additional content access, but it learns the joiner's IP address/timing and spends
bandwidth. Assisted invite recipients learn the host's stable device/transport identities and
advertised candidate addresses. Turning hosting off refuses new forwards immediately; cached or
already-copied offers can remain dial-visible only until their short signed deadline.

A signed offer is not proof that its address currently works. Switchboards help only once a group
has a reachable third member. If the founder and first joiner are both unreachable, they still need
a public IPv6/manual/router mapping, a configured relay, or another mutually reachable third party.

### Using a relay (no port-forwarding)

A relay is a helper node on a reachable host that forwards **encrypted** traffic between two
peers behind NATs. It is **zero-knowledge**; it only routes ciphertext.

1. On a reachable host, run `cargo run -p catcomsctl -- relay --port 4000`. It prints its
   address, e.g. `/ip4/203.0.113.9/tcp/4000/p2p/12D3KooW…`.
2. When founding, paste that into the **Relay address** field.
3. Share the invite as usual; your friend joins **through the relay from anywhere**.

### Using rendezvous discovery

A rendezvous node helps members find one another without putting a member's hard-coded address in
the invite. Set a default under **Settings → Network**, or provide one while founding. The desktop
registers the founder, discovers them when a valid invite is pasted, and keeps re-registering after
the join so members can reconnect after a restart. The rendezvous sees opaque namespaces and
network metadata, not group keys or plaintext. It is not a public directory: the invite is still
required and remains single-use.

---

## 6. Files, folders & custom emoji

The **Files** tab is a folder browser:

- **Share a file here** uploads into the current folder; **＋ new folder** organises them.
  Uploads (everywhere: files, chat, announcements, wiki) show **toast notifications** for each step,
  and sharing a file whose **content already exists** on the server reuses the existing copy
  instead of storing a duplicate.
- **Click a file to open its info pane (Properties)**; a preview (for images/video/audio),
  whether it's already on your device or will be fetched from a peer, who uploaded it, its
  size, type and folder, plus a **Download** button. Owners and admins also get a two-click
  **Delete** there (deleting unlists the file for everyone; members who already downloaded it
  keep their copy). You can also **right-click any embedded image/video/audio or file chip**
  in chat, announcements, or a wiki page and pick **Properties**.
- Properties also shows **Circulates until**; every shared file gets a **30-day circulation
  window** (the date it stops being offered around automatically; nothing is ever deleted
  from anyone's device, and it stays fetchable while any member holds a copy). The uploader
  or an owner/admin can toggle **Keep forever**. A file **embedded in a wiki page never
  expires** (marked 📌 in the file list); and **Used in** lists exactly where the file
  appears: the wiki pages (click to open) plus chat/announcement reference counts.
- Files are **end-to-end encrypted**: stored as ciphertext under a per-group key, sealed at
  rest, and only members can open them.

**Custom emoji** live in the `emoji/` folder. Add one in **Server settings → Emoji & Stickers**:
type a `code`, choose its display size and upload an image. Then anyone types `:code:` to use it.

---

## 7. Wiki, announcements, events & calls

- **Wiki**; collaborative pages, each written in **Markdown or Wikitext** (a per-page
  `md / wiki` switch in Edit mode, shared with every member; wikitext gives you
  `== headings ==`, `'''bold'''`, `{| … |}` tables, MediaWiki-style). `[[links]]` between
  pages (with `[[Page|display text]]` piping, in chat too); pages with **3+ headings get an
  automatic Contents box** (`__TOC__` forces it, `__NOTOC__` hides it); hover a heading in
  Read mode to **edit that section**; `#REDIRECT [[Target]]` pages forward readers; a **What
  links here** list; **rename/delete** in the page header; an editor **toolbar + live
  preview**; drag images/video/audio into the editor (or use the toolbar 📎) to embed them
  **at the caret**, and the toolbar's **+ insert** opens the same picker as the chat composer
  (link/embed shared files, announcements, other pages, events). Files embedded in a wiki page
  are **pinned; they never drop out of circulation**. The **?** button shows in-app
  formatting help. Same-page edits from two members merge character-by-character.
- **Infobox**; the summary card that floats at a page's top right, Wikipedia-style. Write one
  `{{Infobox …}}` block (the **▤** toolbar button drops the skeleton in), with `| key = value`
  lines: `title`, `image` and `caption` are the card's own chrome, every other line becomes a
  labelled row **in the order you wrote them**, and a line with an empty value becomes a
  section band. Values take the usual markup: `[[links]]`, `:emoji:`, bold. The picture must be
  a real embed (drop a file in the editor or use **+ insert**, then move the marker onto the
  `image =` line), which is also what keeps it **pinned** so it never expires. One infobox per
  page, and it works the same in Markdown and Wikitext pages.
- **Organising pages**; put `/` in a page name (`Guides/Setup/Linux`) and the sidebar nests
  it under **collapsible folders**; the standard wiki-subpage scheme, so the page list never
  becomes one giant flat wall. Folders are just shared name prefixes: nothing extra to
  create, and a folder disappears with its last page. The search box still matches full
  names.
- **Page history**; every page has a **history** button (page header): each revision with
  **who wrote it, when, and what changed** (a line diff against the previous revision), plus
  renames, deletes, approvals and rollbacks. Select a revision and **restore this version**
  to roll the page back; a restore is recorded as a new revision, so nothing is ever erased.
- **Edit review** (optional, per server); an owner/admin can set a **review window** (1–30
  days) in the wiki sidebar's **Review** section. While it's on, a plain member's save
  becomes a **pending change**: the page stays as-is until an **owner/admin approves it**
  (it publishes immediately) or **declines it** (recorded in history, never published).
  **Unreviewed changes auto-publish when the window lapses**, so an absent moderator can't
  silence the wiki forever. Admins see the queue (with diffs) under **Review** above the
  page list; authors see an "awaiting review" note on the page. While review is on,
  rename/delete are owner/admin-only, and files embedded in pending edits stay pinned.
- **Announcements**; a per-server feed of short community updates. Supports the same rich text,
  text effects, and media embeds as chat.
- **Events**; a per-server calendar (**Ctrl+7**). Any member can add one with a title,
  start/end time, description and an optional **image**; the image is shared like any other
  file and shows on the event itself and on every link to it. The event's author, an owner or
  an admin can delete it. Upcoming events also appear in the cross-server **News** feed.

### Voice, video & shared play

- Choose **Join voice** in a chat channel's header. Voice rooms belong to channels and remain
  active while you move around the app. The call stage provides mute/deafen, per-person volume,
  input/output device selection, camera and screen sharing.
- **Instruments** opens an on-screen keyboard that also accepts computer-keyboard or Web MIDI
  input; notes are shared live with the room. The **Jukebox** queues audio already circulating in
  the server's Files area, so it does not upload a second copy.
- A hardware **MIDI controller** is set up under **Settings → Devices → MIDI controllers**. That
  panel lists every input the system reports, says which ones are routed, shows the messages
  arriving in a live monitor, and carries step-by-step setup and troubleshooting help. Controllers
  often publish several ports and only one carries the keys, so **input routing** can pin a single
  port; every connected input is the default. Devices may be plugged in at any time, and
  **Rescan** covers a controller that was still enumerating when the app asked. The same
  controller plays the **melody unlock lock**; the sustain pedal is honoured for the call
  instrument only, because the lock's secret depends on how notes were held.
- Media uses a peer-to-peer WebRTC mesh with authenticated signalling. STUN is configured under
  **Settings → Voice & Calls**; optional personal or server-provided TURN relays still-encrypted
  media when a direct media path cannot be made. Large rooms multiply every sender's upload, so
  the current mesh is intended for small groups.

---

## 8. Moderation, operations & server settings

Owners and admins can open **Moderation** in a server's sidebar. A lane-based activity graph shows
the flow between moderators and members; **View user** narrows both it and the chronological detail
scroll below to messages/events involving one person. Ordinary members do not see or navigate to
the moderation plane; when a kick case needs community input they receive a focused vote card in
chat instead.

- Owners and admins can select individual messages or **Shift-click a range**, enter a public
  reason, and **Warn & collapse** them. Other members see a compact warning marker and can expand
  the preserved message for context. A warning is a signed, immutable snapshot; deleting the live
  post later does not silently erase the evidence. Batch deletion requires a second confirmation.
- A moderator can make a removal case and attach signed warnings concerning that member. Members
  vote **yes/no** in public; the tally is advisory. Only the owner can resolve the case and only an
  owner-authorized MLS removal can actually remove the member and rotate the group keys.
- Signatures and signer attribution are verified when the timeline is read. Moderator authority is
  checked against the current owner-signed roles, so a forged or currently unauthorized entry is
  displayed as ignored rather than treated as policy. See `THREAT-MODEL.md` for the remaining
  honest-client enforcement boundary.

The two compact operations buttons above your profile provide local, evidence-based diagnostics:

- **Storage** verifies every file manifest and referenced encrypted chunk on this device. The first
  visit per server runs once per app process and saves that point-in-time report; revisiting it or
  receiving a file event does not quietly rescan. It breaks the inventory down by media type,
  wiki-pinned content and largest files, clearly separating exact verified ciphertext bytes from
  local/logical size estimates. **Repair from peers** is the explicit exception: it fetches missing
  or corrupt chunks through the authenticated, content-address-checked member path and replaces the
  cached report after verifying again. It cannot invent bytes when no reachable member holds them
  or repair an invalid manifest. The same health summary is explicitly available in **Transfers**.
- **Connectivity** explains what this device can actually observe: connected members, the current
  path evidence and existing network settings. It does not promise global reachability from a
  local observation. **Copy diagnostic** provides the same bounded report for troubleshooting.

Right-click the server icon and open **Server settings**:

- The **owner** is the server's founder. Owners can **promote/demote admins** (member list);
  roles show as badges in the sidebar.
- **What roles do today:** the owner is cryptographically anchored (it's the group's MLS
  committer), and an admin grant is **signed by the owner**, so a member **cannot forge** an
  admin role; the badges are trustworthy against tampering.
- **Removal:** the owner can remove a member under **Server settings → Members**. This is a real
  MLS removal: it advances the epoch, rotates discovery secrets and cascades to that member's
  linked devices.
- **What roles do NOT do yet (be aware):** roles are *not* a full access-control system.
  Everyone in a server can read everything in it (they hold the group key); a role does not
  restrict what a member can see. Treat roles as **trusted designation + moderation controls**,
  not a hard content-permission wall.

---

## 9. Multiple servers

You can be in **several servers at once**; the left **rail** shows them.

- **Found** or **Join** adds one and switches to it; **＋** opens the form again.
- A **dot** on an icon means new activity. **⚙ Settings → Leave this server** removes one.

Each server is a separate encrypted group with its own channels, members, profiles, files,
wiki, and roles; they share nothing.

---

## 10. What's protected (and what isn't)

**Protected:**

- **Message + file content** is end-to-end encrypted (MLS / RFC 9420) with **forward secrecy**
  and **post-compromise security**. Files are additionally encrypted under a per-group key.
- **At rest:** everything on disk is sealed under your launch passphrase (Argon2id +
  XChaCha20-Poly1305). A stolen disk / copied app-data folder is opaque without it.
- **Membership is invite-only.** Invites are **single-use** and **bound to one device**.
- **Relays are zero-knowledge**; they route only ciphertext.
- **Admin roles can't be forged**; grants are owner-signed (§8).

**Not protected / be aware:**

- **At-rest, not anti-malware.** Encryption protects a stolen disk, not a live compromise;
  while the app runs, keys are unsealed in memory. Same envelope as Signal-desktop. A
  keylogger capturing your passphrase, or malware running as you, defeats it.
- **Lock closes the app command window, not the network node.** Ctrl/Cmd+L clears frontend
  plaintext and native commands refuse reads/mutations until you unlock again, while the native
  actors may stay online to receive encrypted updates. It is not a defence against malware that
  already controls the process or operating system.
- **Metadata.** Peers/relays can observe *that* you communicate (timing, IPs), not *what*.
- **Remote images contact their host.** An HTTP(S) image pasted into chat is lazy-loaded with no
  referrer, but its host can still see your IP and request timing. Files shared through Mewtual's
  encrypted fileshare do not make that third-party request.
- **Display names aren't identities.** Members are cryptographically identified by their
  **device fingerprint** (shown in the member list), not their chosen name.
- **Roles aren't access control** (§8); being a "member" vs "admin" doesn't change what
  content you can read; everyone in the group can read the group.
- **You trust whoever invited you** and the members already in the server.

---

## 11. Troubleshooting

- **"Can't reach the page" on another machine** → you sent a *debug* build. Send a **release**
  build (§1).
- **Forgot your passphrase** → there is no recovery; the local data is unreadable. Start fresh
  (set a new passphrase; re-found / re-join with an invite).
- **Can't connect over a network** → check the founder advertised a reachable address, the TCP
  port is forwarded, or use a **relay** (§5). Both peers must be running while connecting.
- **An unused invite stopped working** → open **Server settings → Invites** while the owner is
  online and generate a fresh one; the owner's current reachable addresses are folded into it.
- **An avatar / file / embed shows as unavailable** → the member who has it may be offline; it
  appears once a peer holding it is reachable.
- **A rendezvous invite still needs to be pasted** → expected: rendezvous discovers the member's
  address, but it does not replace the single-use authorization carried by the invite (§5).

---

For the project's architecture, security design, and roadmap, see
[`ARCHITECTURE.md`](ARCHITECTURE.md), [`design-persistence.md`](design-persistence.md), and
[`HANDOVER.md`](HANDOVER.md).
