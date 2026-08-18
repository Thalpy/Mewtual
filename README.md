<p align="center">
  <img src="assets/mewtual-logo.svg" alt="Mewtual: a cat in a chat bubble with a heart lock inside a pastel security shield" width="180">
</p>

<h1 align="center">Mewtual</h1>

<p align="center">
  <strong>Private communities, owned by the people in them.</strong><br>
  Peer-to-peer · end-to-end encrypted · invite-only · no accounts
</p>

<p align="center">
  <a href="docs/USER_GUIDE.md">User guide</a> ·
  <a href="docs/ARCHITECTURE.md">Architecture</a> ·
  <a href="docs/THREAT-MODEL.md">Threat model</a>
</p>

> [!WARNING]
> Mewtual is an experimental, early-stage project. The desktop app works, but it has not
> received an independent security audit and should not yet be treated as production-ready.

## What is Mewtual?

Mewtual is a desktop app for private group communication without a central service holding
the community's messages, files, or membership database. Every Mewtual “server” is an
independent encrypted group stored and replicated by its members' devices.

The simplest mental model is **Signal-style group cryptography, a Discord-shaped community
space, and peer-to-peer networking**. Members can chat, share files, write wiki pages, post
updates, and move between several separate communities—all without creating an account.

| A conventional hosted platform | Mewtual |
|---|---|
| Stores history on a provider's server | Replicates encrypted history between members |
| Identifies you through an online account | Gives each device a cryptographic identity |
| Lets anyone request or discover an account | Requires a single-use, device-bound invite |
| Depends on the provider staying online | Connects peers directly or through a ciphertext-only relay |
| Gives the operator access to platform metadata and policy controls | Leaves each community in its members' hands |

## What can it do?

- **Group chat:** multiple channels, replies, reactions, pins, mentions, unread markers,
  message editing, rich search, Markdown, and custom emoji.
- **Shared knowledge:** collaborative Markdown/Wikitext wiki pages with links, backlinks,
  media embeds, redirects, and concurrent editing.
- **Files and media:** encrypted file sharing, folders, inline image/audio/video embeds,
  content-addressed fetching, and configurable circulation periods.
- **Community spaces:** member profiles, avatars, status posts, events, owner/admin badges,
  server themes, and multiple servers in one app.
- **Private calls:** peer-to-peer call signalling and media without a central call service.
- **Offline use:** read local history while disconnected and catch up with another member
  when a peer becomes reachable again.

## How it works

1. A founder creates a server, which creates a new MLS encrypted group and device identity.
2. The founder shares a single-use invite. The invite is bound to the joining device when it
   is redeemed, so it cannot be reused as a permanent bearer credential.
3. Members connect over `rust-libp2p`, either directly over TCP or through a relay when direct
   connectivity is unavailable. Transport connections use Noise; application content remains
   encrypted end to end.
4. Chat, wiki, status, calendar, profile, and file-index data are replicated as encrypted CRDT
   documents. Concurrent and offline changes converge when devices reconnect.
5. Files use content addressing and encrypted peer-to-peer fetching. A device can discard an
   old cached copy and fetch it again while another member still holds it.

There is no always-on Mewtual application server. A community does still need at least one
reachable member to join or synchronize, and internet users behind restrictive NAT may need a
relay or port forwarding.

## Security model

Mewtual is designed so that infrastructure can move encrypted traffic without being trusted
with its contents:

- Every server is a separate **MLS group (RFC 9420)** with its own key schedule, forward
  secrecy, and post-compromise security properties.
- Messages and replicated operations are authenticated and encrypted before entering the
  network. Files are encrypted under group-derived keys.
- Relay and rendezvous nodes route or help locate peers but do not receive plaintext content.
- Local group state, messages, files, and identity keys are sealed under the user's passphrase
  using **Argon2id** and **XChaCha20-Poly1305**.
- Membership is invite-only, and removal rotates the secrets used for group traffic and
  discovery.

Important boundaries remain:

- Encryption at rest protects a copied disk, not a running device compromised by malware.
- Peers and relays may observe metadata such as IP addresses, timing, and traffic volume.
- A forgotten local passphrase cannot be recovered; Mewtual has no account or reset service.
- Display names are not identities. The stable identity is the device fingerprint shown by
  the app.
- Owner and admin roles are authenticated labels, but they are not yet fine-grained content
  access controls: members of a server can read that server's shared content.

For the assumptions, attacker capabilities, mitigations, and remaining risks, read the
[threat model](docs/THREAT-MODEL.md).

## Layout

```
crates/
  catcoms-wire    canonical, injective, length-prefixed wire encoding
  catcoms-rt      core runtime seams: Clock + RNG + MeshTransport (+ in-memory test impls)
  catcoms-crypto  identity (content-addressed ids, device-cert chains, revocation,
                  roster), and the unified key hierarchy / tiered key store
  catcoms-mls     MLS group core (openmls): one group per server, pinned ciphersuite,
                  ServerGroup lifecycle, and per-channel key derivation
  catcoms-replication  encrypted CRDT documents (automerge): inner-signed ops,
                  per-epoch sealing, and snapshot catch-up for late members
  catcoms-storage  content-addressed blob stores, per-file encryption (per-file wrap
                  nonce), and the retention engine (3-scope expiry, GC with
                  decorrelated eviction + holder probe, refetchable on eviction)
  catcoms-net     libp2p MeshService realizing the MeshTransport seam (gossipsub +
                  request/response over Noise+yamux)
  catcoms-sync    ChannelSync: replicate encrypted CRDT docs over any MeshTransport
                  (blinded ns_secret_L topics, live gossip, request/response catch-up,
                  member PEX, the pre-join join_ns, membership tags)
  catcoms-discovery  pure eclipse-resistance: DiscoveryPolicy (ranked, bounded dial
                  plan; the only thing that decides what to dial), advisory eclipse
                  detector, cross-session address cache (no I/O, no ambient time/RNG)
  catcoms-app     the UI-facing product model: Server facade, canonical chat-message
                  schema, and the async event-stream actor (commands in / events out)
  catcoms-log     tracing/diagnostics init for binaries and tests (RUST_LOG) +
                  toggleable debug-to-file (debug_log_<timestamp>.txt)
bins/
  catcomsctl      dev CLI driving the whole stack (`demo`), with --debug file logs
apps/
  desktop         Tauri 2 + Svelte 5 desktop app over the catcoms-app actor bridge
                  (its own cargo workspace; `npm install && npm run tauri dev`)
```
More crates (`catcoms-core`, `catcoms-harness`) land with their phases.

Diagnostics use the `tracing` facade; binaries/tests install a subscriber via
`catcoms_log::init()` / `init_test()`. Filter with e.g.
`RUST_LOG=catcoms_net=debug,catcoms_sync=trace`.

## Run the desktop app

The current user-facing target is the Tauri 2 desktop app. Development requires Rust 1.89,
Node.js/npm, and the platform dependencies required by Tauri. Windows users also need the
WebView2 runtime, which is included with Windows 11.

```sh
cd apps/desktop
npm install
npm run tauri dev
```

On first launch, enter a passphrase. On a new profile, that passphrase becomes the key used to
seal local data; on later launches, the same passphrase unlocks it.

To start talking:

1. Choose **Found a server** and give it a name.
2. Open **Settings → Invite someone** and copy the generated invite.
3. Have another Mewtual instance choose **Join**, paste the invite, and connect.
4. Open `#general` and send a message.

See the [user guide](docs/USER_GUIDE.md) for the complete UI walkthrough, networking setup,
file sharing, wiki syntax, roles, and troubleshooting.

### Test two instances on one computer

`tauri dev` owns the Vite development port, so run the second instance from the already-built
debug binary while the first terminal remains open:

```sh
# Terminal 1
cd apps/desktop
npm run tauri dev

# Terminal 2
./apps/desktop/src-tauri/target/debug/mewtual-desktop
```

Use a blank reachable-address field when both instances are on the same machine.

## Connecting other people

The founder advertises one or more addresses in the invite:

| Where the joining member is | Setup |
|---|---|
| Same computer | Leave the reachable address blank |
| Same LAN/Wi-Fi | Enter the founder's LAN IP, such as `192.168.1.5` |
| Across the internet | Enter a public address and forward the selected TCP port |
| Behind NAT without port forwarding | Run a reachable relay and paste its multiaddress |

Relays forward encrypted bytes; they do not join the MLS group or receive its keys. Automatic
rendezvous discovery exists in the core/CLI but is not yet fully wired into the desktop flow.

## Build a distributable app

Do not share a debug executable: it expects a local Vite server. Build a self-contained release
executable instead:

```sh
cd apps/desktop
npm install
npm run build
npm run tauri build -- --no-bundle
```

The result is written to `apps/desktop/src-tauri/target/release/` (`.exe` on Windows). The
current Tauri configuration produces the executable without an installer bundle or code
signature.

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

## Project layout

```text
apps/desktop             Tauri 2 + Svelte 5 desktop application
bins/catcomsctl          Development CLI for demos, peers, relay, and rendezvous
crates/catcoms-app       UI-facing product model and event-stream actor
crates/catcoms-crypto    Device identity, key hierarchy, and encrypted key storage
crates/catcoms-discovery Bounded peer discovery and eclipse-resistance policy
crates/catcoms-mls       MLS group lifecycle, invites, membership, and persistence
crates/catcoms-net       libp2p mesh, Noise transport, relay, and rendezvous
crates/catcoms-replication Encrypted, authenticated CRDT documents
crates/catcoms-rt        Runtime seams for clocks, randomness, storage, and transport
crates/catcoms-storage   Encrypted content-addressed blobs and retention
crates/catcoms-sync      Live replication, catch-up, peer exchange, and blob fetching
crates/catcoms-wire      Canonical length-prefixed wire encoding
docs/                    Architecture, threat model, user guide, and design notes
```

The desktop application has its own Cargo workspace so Tauri's platform dependencies stay out
of the core protocol test cycle. The [architecture document](docs/ARCHITECTURE.md) explains the
layer boundaries and the test-gated implementation roadmap.

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

The ambient-dependency gate keeps OS time and randomness behind the runtime seams, which makes
protocol behaviour deterministic under test. Wire-format fuzzing is also available from
`crates/catcoms-wire/fuzz` with nightly Rust.

## Licence

Mewtual is **source-available**, not OSI-approved open source. Noncommercial use, modification,
and redistribution are permitted only under the [Mewtual Combined Licence Terms](LICENSE),
which incorporate the PolyForm Noncommercial License 1.0.0 and additional no-AI and
source-disclosure conditions.

In short: commercial use is prohibited; the code may not be used for AI training, AI tooling,
or AI-based projects; and forks or network-deployed modifications must publish their complete
buildable source under the same terms. Third-party dependencies remain under their respective
licences. Read the licence document for the actual terms.

## Current status

The encrypted group core, peer-to-peer transport, direct and relayed joining, replication,
encrypted persistence, file transfer, and desktop product experience are implemented and
covered by the repository's test suites. Work is still ongoing around easier automatic
discovery in the desktop UI, chunked large-file transfer, retention hardening, packaging,
mobile support, and broader security review.

This repository deliberately documents unfinished security properties instead of presenting
them as complete. Start with:

- [User guide](docs/USER_GUIDE.md)—how to run and use the desktop app.
- [Architecture](docs/ARCHITECTURE.md)—protocol layers, invariants, and roadmap.
- [Threat model](docs/THREAT-MODEL.md)—assets, trust boundaries, attacks, and residual risk.
- [Interfaces](docs/INTERFACES.md)—the main cross-crate contracts.
- [Handover notes](docs/HANDOVER.md)—implementation state and engineering context.
