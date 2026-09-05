<p align="center">
  <img src="assets/mewtual-logo.svg" alt="Mewtual: a cat in a chat bubble with a heart lock inside a pastel security shield" width="180">
</p>

<h1 align="center">Mewtual</h1>

<p align="center">
  <strong>Private communities, owned by the people in them.</strong><br>
  Peer-to-peer · end-to-end encrypted · invite-only · no accounts
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-0.3.0--alpha.13-6f4fd6" alt="Version 0.3.0-alpha.13">
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux-444" alt="Windows and Linux">
  <img src="https://img.shields.io/badge/stack-Rust%20%2B%20Tauri%202%20%2B%20Svelte%205-b7410e" alt="Rust, Tauri 2, Svelte 5">
  <img src="https://img.shields.io/badge/crypto-MLS%20RFC%209420-2d7d46" alt="MLS RFC 9420">
  <img src="https://img.shields.io/badge/licence-source--available-777" alt="Source-available licence">
</p>

<p align="center">
  <a href="docs/USER_GUIDE.md">User guide</a> ·
  <a href="docs/ARCHITECTURE.md">Architecture</a> ·
  <a href="docs/THREAT-MODEL.md">Threat model</a> ·
  <a href="docs/HANDOVER.md">Current state</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

> [!WARNING]
> Mewtual is an experimental, early-stage project. The desktop app works and is used daily, but it
> has not received an independent security audit and should not yet be treated as production-ready.

The current alpha is **0.3.0-alpha.13**. Windows installers are published to
[GitHub Releases](https://github.com/Thalpy/Mewtual/releases); Linux is built from source
(`.deb` / AppImage) and is still experimental. Alpha installers are not code-signed, so Windows
SmartScreen may display a warning.

Once installed on Windows, Mewtual checks for a newer release on launch and offers it: updates are
minisign-signed and verified before anything is installed, and nothing installs without your click.
Maintainers: see [docs/RELEASING.md](docs/RELEASING.md).

## What is Mewtual?

Mewtual is a desktop app for private group communication without a central service holding the
community's messages, files, or membership database. Every Mewtual "server" is an independent
encrypted group stored and replicated by its members' devices.

The simplest mental model is **Signal-style group cryptography, a Discord-shaped community space,
and peer-to-peer networking**. Members can chat, share files, write wiki pages, post announcements,
run voice rooms, play music together, and move between several separate communities, all without
creating an account.

| A conventional hosted platform | Mewtual |
|---|---|
| Stores history on a provider's server | Replicates encrypted history between members |
| Identifies you through an online account | Gives each device a cryptographic identity |
| Lets anyone request or discover an account | Requires a single-use, device-bound invite |
| Depends on the provider staying online | Connects peers directly or through a ciphertext-only relay |
| Gives the operator access to platform metadata and policy controls | Leaves each community in its members' hands |

## What can it do?

The desktop app has a searchable **Settings → Feature Guide** that jumps straight to each of these
surfaces. The short map:

### Conversation

| Feature | What is included | Where |
|---|---|---|
| **Channels and chat** | Multiple channels, Markdown, replies, reactions, editing/deletion, pins, mentions, unread markers, per-channel drafts, evidence-based delivery ticks, pasted-image composing and a large catalogue of text effects | **Chat** (`Ctrl+1`) |
| **Search and inbox** | Server-wide filters for people, dates, media, links, replies and reactions, scanned off the main thread; plus a cross-server mentions/replies inbox | Chat → magnifier (`Ctrl+F` / `Ctrl+Shift+F`); left rail → **Inbox** |
| **DMs and friends** | Unlinkable 1:1 spaces, friend codes, activity sorting and authenticated in-server friend requests | Left rail → **DMs** |

### Knowledge and media

| Feature | What is included | Where |
|---|---|---|
| **Files and media** | Encrypted chunked sharing, folders, previews, inline image/audio/video, deduplication, circulation controls, wiki pins, trust-gated transfers and adaptive per-viewer streaming | **Files** (`Ctrl+2`), **Transfers** (`Ctrl+6`) |
| **Storage health** | A once-per-session integrity and inventory report with repair, category, pin and largest-file breakdowns, re-fetching only from authenticated peers | Server sidebar → **Storage** |
| **Shared knowledge** | Collaborative Markdown/Wikitext pages, nested pages, links/backlinks, redirects, infoboxes, history, rollback and optional edit review | **Wiki** (`Ctrl+4`) |
| **Announcements and planning** | A server noticeboard with owner/admin or open posting, reactions, pins, per-server read marks, a cross-server News feed, and shared events with optional artwork | **Announcements** (`Ctrl+3`), **Inbox → News**, **Events** (`Ctrl+7`) |
| **Durable history** | Read sealed local history while disconnected; drafts and read positions survive restarts inside the vault; catch up when another member becomes reachable | Automatic after unlock |

### People, community and trust

| Feature | What is included | Where |
|---|---|---|
| **Profiles and identity** | Per-server profiles, bios, banners, animated avatars, message bubbles, composable name effects and private fingerprint verification marks | **Profile** (`Ctrl+5`); **Settings → My Profile / Verification** |
| **Community management** | Owner/admin roles, protocol-enforced member removal, member badges, single-use invites, custom emoji/stickers and linked-device visibility | Right-click a server → **Server settings** |
| **Moderation and accountability** | A signed per-user activity graph and event timeline, range-selected warnings/deletion, collapsible warned posts, evidence-backed kick cases, member votes and owner-enforced removal | Server sidebar → **Moderation** |
| **Devices, vault and recovery** | Passphrase, sigil or melody unlock and atomic secret changes; session lock; encrypted backups with explicit exposure warnings; multi-device grants by paste, QR or sound; signed member recovery codes; per-device revocation | **Settings → Vault & Lock / Backup & Recovery / Devices** |

### Voice, play and appearance

| Feature | What is included | Where |
|---|---|---|
| **Voice and shared play** | Channel voice rooms, camera and screen sharing (with the surface's own audio), per-peer voice and screen-audio gain, and a room jukebox backed by server files | Chat header → **Join voice** |
| **Instruments and jam** | A shared Keys and Pads instrument played from screen, computer keyboard or Web MIDI, synthesized locally from events rather than streamed as audio, with recordable takes that can be queued back through the jukebox | Voice stage → **Instruments** / **Jukebox**; **Settings → Devices** |
| **Appearance and spaces** | Local themes, text/density/motion controls, safe shared server liveries and a configurable 360° server room | **Settings → Appearance / Server Space**; server settings → **Livery** |

### Operations

| Feature | What is included | Where |
|---|---|---|
| **Connectivity** | Direct, UPnP-assisted, relayed and rendezvous-discovered connections; a per-server connectivity assistant; post-join rediscovery, connection reports and join logs | Sidebar **Connectivity**; server settings → **Join Log** |
| **Diagnostics** | One correlated, redaction-aware event record behind an in-app console, with keyed per-session identifiers, deterministic report rendering, a Safe capture for sharing and a separately labelled raw debug log | **Settings → Diagnostics** |
| **Updates** | Minisign-signed release checks and explicit, user-confirmed installs (Windows) | **Settings → Updates** |

The feature guide and [`docs/USER_GUIDE.md`](docs/USER_GUIDE.md) describe the UI; implementation
status and security caveats live in [`docs/HANDOVER.md`](docs/HANDOVER.md) and
[`docs/THREAT-MODEL.md`](docs/THREAT-MODEL.md).

## How it works

1. A founder creates a server, which creates a new MLS encrypted group and device identity.
2. The founder shares a single-use invite. The invite is bound to the joining device when it is
   redeemed, so it cannot be reused as a permanent bearer credential.
3. Members connect over `rust-libp2p`, either directly over TCP or through a relay when direct
   connectivity is unavailable. Transport connections use Noise; application content remains
   encrypted end to end.
4. Chat, wiki, announcement, calendar, profile, and file-index data are replicated as encrypted
   CRDT documents. Concurrent and offline changes converge when devices reconnect.
5. Files use content addressing and encrypted peer-to-peer fetching. A device can discard an old
   cached copy and fetch it again while another member still holds it.

There is no always-on Mewtual application server. A community does still need at least one
reachable member to join or synchronize, and internet users behind restrictive NAT may need a relay
or port forwarding.

## Security model

Mewtual is designed so that infrastructure can move encrypted traffic without being trusted with
its contents:

- Every server is a separate **MLS group (RFC 9420)** with its own key schedule, forward secrecy,
  and post-compromise security properties.
- Messages and replicated operations are authenticated and encrypted before entering the network.
  Files are encrypted under group-derived keys.
- Relay and rendezvous nodes route or help locate peers but do not receive plaintext content.
- Local group state, messages, files, and identity keys are sealed under the user's passphrase
  using **Argon2id** and **XChaCha20-Poly1305**.
- Membership is invite-only, and removal rotates the secrets used for group traffic and discovery.

Important boundaries remain:

- Encryption at rest protects a copied disk, not a running device compromised by malware.
- Peers and relays may observe metadata such as IP addresses, timing, and traffic volume.
- A forgotten local passphrase cannot be recovered; Mewtual has no account or reset service.
- Display names are not identities. The stable identity is the device fingerprint shown by the app.
- Owner and admin roles are authenticated labels, not fine-grained content access controls: members
  of a server can read that server's shared content.
- The optional raw debug log is not filtered the way the in-app diagnostics console is. Read it
  before sharing it.

For the assumptions, attacker capabilities, mitigations, and remaining risks, read the
[threat model](docs/THREAT-MODEL.md).

## Staying fast as history grows

A peer-to-peer client keeps its own history, so "it got slower every month" is a design problem
rather than a hosting bill. The 2026-09 performance pass measured it and fixed the causes; the plan
and every measurement live in
[`docs/PERFORMANCE-SECURITY-HARDENING.md`](docs/PERFORMANCE-SECURITY-HARDENING.md).

| Cost, measured on one channel | Before | After |
|---|---|---|
| Materializing a 20k-message channel (release build) | 823 ms | 75 ms cold, 0 ms cached |
| The actor's per-network-event change check at 20k | one full materialization each time | 2 ms |
| Sealed snapshot after a send at 20k | 47 ms, every change | 54 ms, only when a document moved |
| Startup JavaScript chunk | 881 kB / 291 kB gzip | 709 kB / 230 kB gzip |

The changes behind those numbers: a cursor walk instead of an indexed read per row and field, a
per-document version cache, version-gated projections, **native paging** so the webview holds one
slice of a channel rather than all of it ([`docs/design-native-paging.md`](docs/design-native-paging.md)),
frontier-based catch-up so a reconnect costs the change instead of the history, and the
server-wide search scan moved into a worker.

## Repository layout

```text
apps/
  desktop                  Tauri 2 + Svelte 5 desktop app over the catcoms-app actor bridge
                           (its own cargo workspace; npm install && npm run tauri dev)
bins/
  catcomsctl               Dev CLI driving the whole stack (demo, serve, join, relay, rendezvous)
crates/
  catcoms-wire             Canonical, injective, length-prefixed wire encoding
  catcoms-rt               Core runtime seams: Clock + RNG + MeshTransport (+ in-memory test impls)
  catcoms-crypto           Identity (content-addressed ids, device-cert chains, revocation, roster)
                           and the unified key hierarchy / tiered key store
  catcoms-mls              MLS group core (openmls): one group per server, pinned ciphersuite,
                           ServerGroup lifecycle, per-channel key derivation
  catcoms-replication      Encrypted CRDT documents (automerge): inner-signed ops, per-epoch
                           sealing, snapshot catch-up, and the epoch-close/receipt core
  catcoms-storage          Content-addressed blob stores, per-file encryption (per-file wrap
                           nonce), and the retention engine (3-scope expiry, GC with decorrelated
                           eviction + holder probe, refetchable on eviction)
  catcoms-net              libp2p MeshService realizing the MeshTransport seam (gossipsub +
                           request/response over Noise+yamux)
  catcoms-sync             ChannelSync: replicate encrypted CRDT docs over any MeshTransport
                           (blinded ns_secret_L topics, live gossip, frontier catch-up, member
                           PEX, the pre-join join_ns, membership tags)
  catcoms-discovery        Pure eclipse-resistance: DiscoveryPolicy (ranked, bounded dial plan),
                           advisory eclipse detector, cross-session address cache (no I/O, no
                           ambient time/RNG)
  catcoms-app              The UI-facing product model: Server facade, canonical chat-message
                           schema, paged history queries, and the async event-stream actor
  catcoms-diagnostics      One canonical record of what the app did: a single event shape, keyed
                           redaction, a bounded timeline, and deterministic report rendering
  catcoms-log              tracing/diagnostics init for binaries and tests (RUST_LOG) +
                           toggleable debug-to-file (debug_log_<timestamp>.txt)
docs/                      Architecture, threat model, user guide, and design notes
```

The desktop application has its own Cargo workspace so Tauri's platform dependencies stay out of
the core protocol test cycle. The [architecture document](docs/ARCHITECTURE.md) explains the layer
boundaries and the test-gated implementation roadmap; [`docs/INTERFACES.md`](docs/INTERFACES.md)
holds the cross-crate contracts.

Diagnostics use the `tracing` facade; binaries and tests install a subscriber via
`catcoms_log::init()` / `init_test()`. Filter with e.g.
`RUST_LOG=catcoms_net=debug,catcoms_sync=trace`.

## Run the desktop app

Development requires Rust 1.89, Node.js/npm, and the platform dependencies required by Tauri.
Windows users also need the WebView2 runtime, which is included with Windows 11.

```sh
cd apps/desktop
npm install
npm run tauri dev
```

On first launch, enter a passphrase. On a new profile, that passphrase becomes the key used to seal
local data; on later launches, the same passphrase unlocks it.

To start talking:

1. Choose **Found a server** and give it a name.
2. Right-click the server icon, open **Server settings → Invites**, and copy or generate an invite.
3. Have another Mewtual instance choose **Join**, paste the invite, and connect.
4. Open `#general` and send a message.

See the [user guide](docs/USER_GUIDE.md) for the complete UI walkthrough, networking setup, file
sharing, wiki syntax, roles, and troubleshooting.

### Test two instances on one computer

`tauri dev` owns the Vite development port, so run the second instance from the already-built debug
binary while the first terminal remains open:

```sh
# Terminal 1
cd apps/desktop
npm run tauri dev

# Terminal 2
./apps/desktop/src-tauri/target/debug/mewtual-desktop
```

Use a blank reachable-address field when both instances are on the same machine.

### Connecting other people

The founder advertises one or more addresses in the invite:

| Where the joining member is | Setup |
|---|---|
| Same computer | Leave the reachable address blank |
| Same LAN/Wi-Fi | Enter the founder's LAN IP, such as `192.168.1.5` |
| Across the internet | Enter a public address and forward the selected TCP port |
| Behind NAT without port forwarding | Run a reachable relay and paste its multiaddress |

Relays forward encrypted bytes; they do not join the MLS group or receive its keys. When a
rendezvous address is configured, the desktop registers there, discovers the inviter from the
invite, and keeps re-registering after the join so existing members can find one another again.
Rendezvous is private reachability infrastructure, not a public server directory: joining still
requires a valid single-use invite.

## Build a release

Release builds embed the frontend inside the application. Do not distribute an executable from
`target/debug`: debug builds expect the local Vite development server to be running.

<details>
<summary><strong>Windows installer (NSIS)</strong></summary>

You need Rust 1.89, Node.js/npm, and the [Tauri prerequisites for
Windows](https://v2.tauri.app/start/prerequisites/#windows). From the repository root:

```powershell
cd apps/desktop
npm ci
npm test
npm run check
npm run tauri build -- --bundles nsis
```

The Tauri command runs the production frontend build automatically and then creates the NSIS
installer at:

```text
apps/desktop/src-tauri/target/release/bundle/nsis/
```

For the current alpha, the resulting file is `Mewtual_0.3.0-alpha.13_x64-setup.exe`. Before sharing
it, you can calculate a checksum from the repository root:

```powershell
Get-FileHash -Algorithm SHA256 `
  "apps/desktop/src-tauri/target/release/bundle/nsis/Mewtual_0.3.0-alpha.13_x64-setup.exe"
```

The alpha installer is not code-signed, so Windows SmartScreen may display a warning. A local
portable executable can be built with `npm run tauri build -- --no-bundle`; it is written to
`apps/desktop/src-tauri/target/release/mewtual-desktop.exe`.

</details>

<details>
<summary><strong>Linux packages (deb / AppImage), experimental</strong></summary>

Tauri links against WebKitGTK/GTK3 at build time, so this has to run on an actual Linux environment
(native, a VM, or WSL2) rather than being cross-compiled from a Windows host. Install the
[Tauri prerequisites for Linux](https://v2.tauri.app/start/prerequisites/#linux); on Debian/Ubuntu:

```sh
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Then, from the repository root:

```sh
cd apps/desktop
npm ci
npm test
npm run check
npm run tauri build -- --bundles deb,appimage
```

`--bundles nsis` only builds on Windows, so `deb,appimage` replaces it here;
`npm run tauri build -- --no-bundle` still works for a plain portable binary. The command produces:

```text
apps/desktop/src-tauri/target/release/bundle/deb/*.deb
apps/desktop/src-tauri/target/release/bundle/appimage/*.AppImage
```

As with a Windows source build, the result has no auto-update wiring: `tauri.official.conf.json`'s
updater endpoint and key are only merged in by the Windows `release.yml` workflow. CI runs the
frontend suite/check/build and the separate Tauri test/check workspace against Ubuntu's WebKitGTK
development libraries. That proves the Linux build surface, not graphical portal, PipeWire or
hardware-codec behaviour; see [Linux testing](docs/LINUX-TESTING.md) for the Docker lanes and the
real-desktop media boundary.

</details>

<details>
<summary><strong>Publish a GitHub alpha (maintainers)</strong></summary>

The manual **Release desktop alpha** workflow in `.github/workflows/release.yml` runs the frontend
checks, builds the Windows installer with the updater config merged in, and creates a **draft**
GitHub release:

1. Commit and push the release version and changelog to `main`.
2. Open the repository's **Actions** tab on GitHub.
3. Select **Release desktop alpha**, then **Run workflow**.
4. When it succeeds, open **Releases**, review the generated draft and installer, confirm
   **Set as the latest release** is ticked and **Set as a pre-release** is **not**, then publish.

A release marked pre-release (or left as a draft) is invisible to every installed copy, because
GitHub's `latest` pointer skips both: the in-app update check silently finds nothing. The version
in `apps/desktop/package.json`, `apps/desktop/src-tauri/Cargo.toml`, and
`apps/desktop/src-tauri/tauri.conf.json` must match before starting the workflow. The full
procedure, including the API path and its failure modes, is in
[docs/RELEASING.md](docs/RELEASING.md).

</details>

## Try the protocol from the CLI

The developer CLI can exercise the stack without the desktop UI:

```sh
# Alice founds a group, Bob redeems an invite, and encrypted chat converges in one process
cargo run -p catcomsctl -- demo

# Add protocol diagnostics
cargo run -p catcomsctl -- --debug --stats demo
```

For two real OS processes over TCP:

```sh
# Terminal 1: create a server and write its invite
cargo run -p catcomsctl -- serve --port 9000 --invite-file invite.txt

# Terminal 2: redeem the invite and join
cargo run -p catcomsctl -- join --invite-file invite.txt
```

For a relayed connection:

```sh
# Reachable host: start a relay and copy the printed multiaddress
cargo run -p catcomsctl -- relay --port 4000

# Founder: reserve a circuit through that relay
cargo run -p catcomsctl -- serve --relay /ip4/<relay-ip>/tcp/4000/p2p/<relay-id>
```

Set `RUST_LOG` for detailed console traces, for example
`RUST_LOG=catcoms_net=debug,catcoms_sync=trace`.

For automated two-client product checks, the real-TCP test, and the current manual desktop
procedure, see [Two-client testing](docs/TWO-CLIENT-TESTING.md). Implementation and verification
progress across the remaining roadmap is tracked in
[Development sweep](docs/DEVELOPMENT-SWEEP.md).

## Build and test

```sh
cargo build
cargo test --all --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
bash scripts/check-no-ambient.sh
```

Frontend checks run separately:

```sh
cd apps/desktop
npm test
npm run check
npm run build
```

That is 989 frontend tests and over a thousand Rust test functions across the two workspaces, plus
the gates that make them mean something: the ambient-dependency check keeps OS time and randomness
behind the runtime seams, so protocol behaviour stays deterministic under test, and the desktop's
Tauri command ledger (`src/tauri-command-security.ts`) is compared against both the Rust handler
list and the literal frontend invocations, so a new, removed or unclassified command fails the
suite. Wire-format fuzzing is available from `crates/catcoms-wire/fuzz` with nightly Rust.

## Licence

Mewtual is **source-available**, not OSI-approved open source. Noncommercial use, modification, and
redistribution are permitted only under the [Mewtual Combined Licence Terms](LICENSE), which
incorporate the PolyForm Noncommercial License 1.0.0 and additional no-AI and source-disclosure
conditions.

In short: commercial use is prohibited; the code may not be used for AI training, AI tooling, or
AI-based projects; and forks or network-deployed modifications must publish their complete
buildable source under the same terms. Third-party dependencies remain under their respective
licences. Read the licence document for the actual terms.

## Current status

**Working and covered by the test suites:** the encrypted group core, peer-to-peer transport,
direct/relayed/rendezvous joining, replication and catch-up, encrypted persistence, chunked file
transfer, post-join rediscovery, multi-device admission, voice and shared instruments, the
diagnostics plane, and the desktop product experience.

**In progress:** bounded history through owner-signed epoch closes and recovery snapshots (the
replication core has landed; checkpoint materializers, settlement, sync discovery and UI have not,
see [`docs/design-epoch-close.md`](docs/design-epoch-close.md)); the rest of the performance
sequence; last-copy-safe retention and disk quotas; automated restore; reachability diagnosis;
mobile support; voice hardening; and broader security review.

**Designed but not shipped:** the creative suite (shared drawing, flipnotes, sounding emoji, in-call
play), which depends on the epoch-close work above.
See [`docs/design-creative-suite.md`](docs/design-creative-suite.md).

This repository deliberately documents unfinished security properties instead of presenting them as
complete. Start with:

- [User guide](docs/USER_GUIDE.md): how to run and use the desktop app.
- [Architecture](docs/ARCHITECTURE.md): protocol layers, invariants, and roadmap.
- [Threat model](docs/THREAT-MODEL.md): assets, trust boundaries, attacks, and residual risk.
- [Interfaces](docs/INTERFACES.md): the main cross-crate contracts.
- [Handover notes](docs/HANDOVER.md): implementation state and engineering context.
</content>
</invoke>
