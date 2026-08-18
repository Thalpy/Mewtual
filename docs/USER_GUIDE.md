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

## 2. First launch & your passphrase

On launch, Mewtual asks for a **passphrase**. This isn't an account; it's the key that
**encrypts everything on this device at rest**.

- **First run:** the passphrase you type *becomes* your encryption passphrase. There is **no
  recovery**; if you forget it, the local data is unreadable.
- **Later runs:** type the same passphrase to unlock. Your servers, channels, history,
  profiles, files, wiki, and roles all come back; **readable offline**, even before any
  other member is online.

Everything on disk (group state, messages, files, avatars) is sealed under this passphrase
(Argon2id + XChaCha20-Poly1305). A stolen laptop or a copied app-data folder is opaque
without it.

---

## 3. Quick start

1. **Found a server**; type a display name, click **Found a server**.
2. **Invite a friend**; open **⚙ Settings** (gear on the left rail) → **Invite someone** →
   **Copy invite** (or use the **＋ Invite someone** button under the member list). Invites are
   single-use; click **Generate new invite** any time you need a fresh one (e.g. after a
   restart, or once the last one was used).
3. **They join**; your friend opens Mewtual, sets a passphrase, pastes the invite into the
   join box, clicks **Join**.

You're now in a shared, encrypted `#general` channel. Type and send.

---

## 4. The layout

- **Left rail**; your servers (one icon each) + **＋** to add one + **💬** to send feedback
  (a bug report or feature request: **File on GitHub** opens a prefilled issue on the tracker in
  your browser for you to review and submit, and **Copy report** puts the same text on your
  clipboard if you would rather send it another way) + **⚙** for Settings.
- **Updates**; a few seconds after launch Mewtual asks GitHub whether a newer release exists. If
  one does, a card appears bottom right with the release notes and three choices: **Update and
  restart**, **Later** (it asks again next launch), or **Skip this version** (it never asks about
  that one again). Nothing installs unless you press Update, and **Settings → Updates** can check
  on demand, including for a version you skipped.
- **Sidebar**; the server's channels and its member list (with role badges).
- **Main pane tabs**; **Chat · Files · Status · Wiki · Profile**. Click a tab to switch.

### Right-click menus & keyboard shortcuts
- **Right-click** almost anything for quick actions: a **message** (copy text, quote, copy
  sender), a **wiki page** (open, **post a link to it in chat**, copy link), a **file** (open
  details, download, post to chat, copy address, delete), a **member** (copy fingerprint,
  make/demote admin, remove; owner), or a **server icon** (copy invite, settings, leave).
- **Keyboard**: **Enter** sends, **Shift+Enter** adds a newline; **Esc** closes the open
  menu/panel; **Ctrl/Cmd+1–5** switch tabs; **Ctrl/Cmd+K** jumps to the message box.
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

### Rich text, links & emoji (in chat, status, and the wiki)
- **Markdown** works: `**bold**`, `*italic*`, `` `code` ``, lists, `> quotes`, links.
- **`[[Page Name]]`** links to a wiki page, and **`[[Page Name|shown text]]`** links it under
  different text (click to open; a **red** link means it doesn't exist yet; click to create it).
- **`:code:`** inserts a custom emoji (see Files → emoji below). The 😀 button in the
  composer opens a picker.
- **Link cards**; a reference that sits **on a line of its own** unfurls into a small card
  showing what it points at: the opening of a wiki page, a file's size and who shared it, a
  status post's text, an event's time and description, plus a picture where there is one. A
  reference written *inside* a sentence stays an inline chip, so your prose isn't broken up
  (and a bulleted list of links stays a list).
  Clicking a card opens the thing it describes; right-clicking one offers its actions.

### Sharing media (drag-and-drop embeds)
- **Drag an image, video, or audio file onto the chat box** (or use the 📎 button) and it
  **embeds inline** in your message. The same works in the Status composer and the Wiki
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
| **Over the internet (no port-forward)** | A **relay**; see below. |

Find your LAN IP with `ipconfig` (Windows) / `ip addr` (Linux). The invite carries every
address you advertise; the joiner dials all of them and the one that works wins.

> After you **reopen** the app, a server comes back up on a new port, so its *old* invite may
> no longer reach new joiners; found-time addresses can change. Click **Generate new invite**
> after reopening to mint one that carries the current address. Members who were already
> connected are re-dialed automatically when they're reachable.

### Using a relay (no port-forwarding)

A relay is a helper node on a reachable host that forwards **encrypted** traffic between two
peers behind NATs. It is **zero-knowledge**; it only routes ciphertext.

1. On a reachable host, run `cargo run -p catcomsctl -- relay --port 4000`. It prints its
   address, e.g. `/ip4/203.0.113.9/tcp/4000/p2p/12D3KooW…`.
2. When founding, paste that into the **Relay address** field.
3. Share the invite as usual; your friend joins **through the relay from anywhere**.

---

## 6. Files, folders & custom emoji

The **Files** tab is a folder browser:

- **Share a file here** uploads into the current folder; **＋ new folder** organises them.
  Uploads (everywhere: files, chat, status, wiki) show **toast notifications** for each step,
  and sharing a file whose **content already exists** on the server reuses the existing copy
  instead of storing a duplicate.
- **Click a file to open its info pane (Properties)**; a preview (for images/video/audio),
  whether it's already on your device or will be fetched from a peer, who uploaded it, its
  size, type and folder, plus a **Download** button. Owners and admins also get a two-click
  **Delete** there (deleting unlists the file for everyone; members who already downloaded it
  keep their copy). You can also **right-click any embedded image/video/audio or file chip**
  in chat, status, or a wiki page and pick **Properties**.
- Properties also shows **Circulates until**; every shared file gets a **30-day circulation
  window** (the date it stops being offered around automatically; nothing is ever deleted
  from anyone's device, and it stays fetchable while any member holds a copy). The uploader
  or an owner/admin can toggle **Keep forever**. A file **embedded in a wiki page never
  expires** (marked 📌 in the file list); and **Used in** lists exactly where the file
  appears: the wiki pages (click to open) plus chat/status reference counts.
- Files are **end-to-end encrypted**: stored as ciphertext under a per-group key, sealed at
  rest, and only members can open them.

**Custom emoji** live in the `emoji/` folder. Add one in **⚙ Settings → Custom emoji**:
type a `code` and upload an image. Then anyone types `:code:` to use it.

---

## 7. Wiki, status & events

- **Wiki**; collaborative pages, each written in **Markdown or Wikitext** (a per-page
  `md / wiki` switch in Edit mode, shared with every member; wikitext gives you
  `== headings ==`, `'''bold'''`, `{| … |}` tables, MediaWiki-style). `[[links]]` between
  pages (with `[[Page|display text]]` piping, in chat too); pages with **3+ headings get an
  automatic Contents box** (`__TOC__` forces it, `__NOTOC__` hides it); hover a heading in
  Read mode to **edit that section**; `#REDIRECT [[Target]]` pages forward readers; a **What
  links here** list; **rename/delete** in the page header; an editor **toolbar + live
  preview**; drag images/video/audio into the editor (or use the toolbar 📎) to embed them
  **at the caret**, and the toolbar's **+ insert** opens the same picker as the chat composer
  (link/embed shared files, status posts, other pages, events). Files embedded in a wiki page
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
- **Status**; a per-server feed of short posts (announcements/activity). Supports the same
  rich text + media embeds.
- **Events**; a per-server calendar (**Ctrl+7**). Any member can add one with a title,
  start/end time, description and an optional **image**; the image is shared like any other
  file and shows on the event itself and on every link to it. The event's author, an owner or
  an admin can delete it. Upcoming events also appear in the cross-server **News** feed.

---

## 8. Roles & server settings

Open **⚙ Settings → Server**:

- The **owner** is the server's founder. Owners can **promote/demote admins** (member list);
  roles show as badges in the sidebar.
- **What roles do today:** the owner is cryptographically anchored (it's the group's MLS
  committer), and an admin grant is **signed by the owner**, so a member **cannot forge** an
  admin role; the badges are trustworthy against tampering.
- **What roles do NOT do yet (be aware):** roles are *not* a full access-control system.
  Everyone in a server can read everything in it (they hold the group key); a role does not
  restrict what a member can see. Removing/kicking a member isn't in the UI yet. Ownership
  follows the founder, so if the founder leaves the group ownership passes to the next member.
  Treat roles as **trusted designation + display**, not a hard permission wall.

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
- **Metadata.** Peers/relays can observe *that* you communicate (timing, IPs), not *what*.
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
- **An old invite stopped working after a restart** → the server came back on a new port; the
  owner should share a fresh invite (§5).
- **An avatar / file / embed shows as unavailable** → the member who has it may be offline; it
  appears once a peer holding it is reachable.
- **No automatic peer discovery yet** → you still paste an invite to join.

---

For the project's architecture, security design, and roadmap, see
[`ARCHITECTURE.md`](ARCHITECTURE.md), [`design-persistence.md`](design-persistence.md), and
[`HANDOVER.md`](HANDOVER.md).
