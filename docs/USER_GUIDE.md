# CatComs — User Guide

CatComs is a **peer-to-peer, end-to-end-encrypted, invite-only** group chat. There are
**no accounts and no central servers** — you talk directly to other members, and every
"server" is its own encrypted group that lives on the members' own devices.

This guide covers the **desktop app** (`apps/desktop`).

---

## 1. Running the app

**From a release build (to share with others):**

```sh
cd apps/desktop
npm install
npm run build
npm run tauri build -- --no-bundle      # self-contained exe → src-tauri/target/release/
```

Send `src-tauri/target/release/catcoms-desktop` (`.exe` on Windows). The recipient needs
the **WebView2 runtime** (built into Windows 11; a free installer for older Windows).

> **Don't share a debug build** (`target/debug/…`). A debug build loads its UI from a local
> dev server and shows an "can't reach the page" error on any other machine.

**For development:**

```sh
cd apps/desktop
npm install
npm run tauri dev
```

To test with two members on one machine, run a second instance: launch the built exe
directly (`./src-tauri/target/debug/catcoms-desktop`) in another terminal while the dev
server is running.

---

## 2. Quick start

1. **Found a server** — type a display name, click **Found a server**.
2. **Invite a friend** — open the **Invite someone** panel in the sidebar and **Copy
   invite**.
3. **They join** — your friend opens CatComs, pastes the invite into the join box, clicks
   **Join**.

You're now in a shared, encrypted `#general` channel. Type and send.

---

## 3. Connecting across a network

When you found a server, open the **Network (optional)** section. How others reach you:

| Where the other person is | What to enter |
|---|---|
| **Same machine** (two windows) | Nothing — leave it blank. |
| **Same Wi-Fi / LAN** | Your **LAN IP** (e.g. `192.168.1.5`) in *Reachable address*. |
| **Over the internet (port-forward)** | Your **public IP** (or `host:port` if the forwarded port differs) in *Reachable address*, and forward that TCP port on your router. |
| **Over the internet (no port-forward)** | A **relay** — see below. |

Find your LAN IP with `ipconfig` (Windows) / `ip addr` (Linux). The invite carries every
address you advertise, and the joiner dials all of them — the one that works wins.

### Using a relay (no port-forwarding)

A relay is a helper node on a reachable host that forwards **encrypted** traffic between
two peers behind NATs. It is **zero-knowledge** — it only routes ciphertext and can never
read your messages.

1. On a reachable host (a cheap VPS, or a friend's port-forwarded box), run:
   ```sh
   cargo run -p catcomsctl -- relay --port 4000
   ```
   It prints its address, e.g. `/ip4/203.0.113.9/tcp/4000/p2p/12D3KooW…`.
2. When founding, paste that into the **Relay address** field.
3. Share the invite as usual. Your friend joins **through the relay from anywhere** — no
   port-forwarding on either side.

---

## 4. Channels

- Every server starts with **#general**.
- **Create or join a channel** by typing its name into **join #channel…** in the sidebar.
  Anyone who opens the same name lands in the same channel (no setup needed).
- Click a channel to switch. A **dot** marks a channel with unread messages.
- Opening a channel that already has history pulls the **backlog** from another member.

---

## 5. Your profile

Open **Your profile** in the sidebar to customize how you appear to everyone in the
server:

- **Name**, **Color**, **Font**.
- **Effect** — an animated text style (rainbow colour-wave, wave, pulse).
- **Avatar** — pick an image; it's downscaled to a small picture and shared with the group.

Your name, color, effect, and avatar show next to your messages and in the Members list.
Changes apply to all your messages (past and future).

---

## 6. Sharing files

Open the **Files** panel in the sidebar:

- **Share a file** — pick a file; it's stored content-addressed and listed for the group.
- **Download** — click a file name; it's fetched from a member who has it and saved.

> Files are encrypted in transit and only served to members, but are currently stored
> **unencrypted at rest** in each member's local cache. Don't share secrets you wouldn't
> keep in a plain folder until at-rest encryption lands.

---

## 7. Multiple servers

You can be in **several servers at once** — the left **rail** shows them.

- **Found a server** or **Join** adds one to the rail and switches to it.
- Click an icon to switch; the **+** button opens the found/join form again.
- A **dot** on an icon means there's new activity in that server.
- **Leave server** (bottom of the sidebar) removes one.

Each server is a separate encrypted group with its own channels, members, profiles, and
files — they share nothing.

---

## 8. What's protected (and what isn't)

**Protected:**

- **Message content** is end-to-end encrypted (MLS / RFC 9420) with **forward secrecy**
  and **post-compromise security** — a compromised key can't decrypt past or (after a key
  rotation) future messages.
- **Membership is invite-only.** Invites are **single-use** and **bound to one device**;
  a stolen invite can't be reused or used from another device.
- **Relays are zero-knowledge** — they route only ciphertext and can't read content or
  impersonate anyone (the handshake is authenticated end-to-end).

**Not protected / be aware:**

- **Metadata.** Peers and relays you connect to can observe *that* you're communicating
  (connection timing, IP addresses), even though they can't read *what*.
- **Nothing persists yet.** The app keeps everything **in memory** — closing it loses your
  servers, channels, and cached files (you re-found or re-join with an invite). In transit,
  content is encrypted and served members-only. Disk persistence (and encrypting it at rest)
  is planned together; until then there's no on-disk cache to protect.
- **Display names aren't identities.** A name/color/avatar is whatever a member sets;
  members are cryptographically identified by their **device fingerprint** (shown in the
  Members list), not their chosen name. Two members could pick the same name.
- **You trust whoever sent you the invite.** Joining a server means trusting the inviter
  and the members already in it.

---

## 9. Limitations & troubleshooting

- **"Can't reach the page" on another machine** → you sent a *debug* build. Send a
  **release** build (§1).
- **Can't connect over a network** → check the founder advertised a reachable address
  (LAN IP / public IP), the TCP port is forwarded (for the public-IP route), or use a
  **relay** (§3). Both peers must be running while connecting.
- **An avatar/file shows as unavailable** → the member who has it may be offline; it
  appears once a peer holding it is reachable.
- **No automatic peer discovery yet** → you still paste an invite to join. Discovering a
  server with *no* address (rendezvous) is planned.
- **Local cache grows over time** → fetched files/avatars are kept in memory for the
  session; size-bounded eviction is planned.

---

For the project's architecture, security design, and roadmap, see
[`ARCHITECTURE.md`](ARCHITECTURE.md) and [`HANDOVER.md`](HANDOVER.md).
