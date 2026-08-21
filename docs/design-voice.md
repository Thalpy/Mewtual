# Design; E2E group voice

Real-time voice for a server's members, end-to-end encrypted, reusing what Mewtual already has.

## Architecture (Path A; WebRTC in the webview)

- **Media plane:** the Tauri webview's WebRTC (`getUserMedia(audio)` + a mesh of `RTCPeerConnection`s,
  one per other participant). Codec/jitter/AEC/PLC come free from the embedded libwebrtc. Mesh is fine
  for small groups (uplink = (n−1)×~32 kbit/s Opus); ceiling ~8.
- **E2E:** each audio frame is AEAD-encrypted (WebRTC Encoded Transform / `RTCRtpSender.transform`)
  with a **media key derived from the MLS group secret**; so even if media is ever relayed, only
  members can decrypt. The key is the same for all members (MLS exporter), rotates with the epoch.
- **Signalling:** SDP offers/answers + ICE candidates are relayed member-to-member over a new
  **authenticated push** request (`KIND_CALL_SIGNAL`, mirroring `KIND_DM_INVITE`); members-only,
  Ed25519-signed, freshness-bound, `from` = the verified signer. No signalling server.
- **NAT:** WebRTC ICE. MVP uses public STUN; a self-hostable TURN / the libp2p relay path is a later,
  ethos-consistent hardening (see the feasibility analysis).

## What's reused vs new

| Need | Reused | New |
|---|---|---|
| Media key (all members agree, per-epoch) | MLS `export_secret` (`catcoms-mls`) | `MEDIA_EXPORTER_LABEL`, `Group::media_secret(call_id)` → `Server::media_key` |
| Signalling transport | `build_authed_request`/`authenticate_request`, `peer_for_fingerprint`, the `KIND_*` dispatch | `KIND_CALL_SIGNAL`, `send_call_signal`/`serve_call_signal`, `pending_call_signals` |
| Actor plumbing | `AppCommand`/`AppEvent` + `ServerActor` reply pattern + the post-`sync_once` drain | `SendCallSignal`/`MediaKey` commands, `CallSignal` event |
| Bridge | `forward_events` + `#[tauri::command]` | `send_call_signal`/`media_key` commands, `call-signal` event |
| Media/codec/NAT | webview libwebrtc (CSP=null, WebView2) | the WebRTC mesh + Encoded-Transform crypto (frontend) |

## Phases

1. **Crypto + signalling foundation (Rust core).** `media_key` (MLS-derived) + `KIND_CALL_SIGNAL`
   push + actor/bridge plumbing + the `call-signal` event. Cargo-testable (members derive the same
   key; signal round-trips). _← this commit._
2. **Frontend mesh + E2E + call UI.** `getUserMedia`, a per-peer `RTCPeerConnection` mesh, signalling
   wired through the new event/command, Encoded-Transform AEAD with the media key, a call bar
   (join/leave/mute + participant list). Tested with real audio.
3. **Group join/leave + key rotation.** Participants join/leave mid-call; re-derive the media key on
   an MLS epoch change (using the bounded past-epoch window for in-flight frames). VAD/DTX.
4. **Ethos-consistent transport (later).** Move media onto the libp2p relay/DCUtR fabric; TURN opt-in.

## Notes

- The signal payload is **opaque** to the core (the frontend JSON-encodes `{callId, type, data}`); the
  core only relays it authenticated + members-only. No dedup (unlike DM invites; every ICE candidate
  must arrive); FIFO-bounded queue.
- The media key is derived locally by each member from MLS state; it is **never sent on the wire**.
- A "call" is identified by a random `callId` (u128) chosen by the initiator; the media key is
  `export_secret(MEDIA_LABEL, callId)`, so distinct calls have distinct keys.

## Transport findings (2026-08-21)

Recorded because two of the three obvious fixes are dead ends on the shipping target, and
both look plausible enough to be re-attempted otherwise.

### The two planes share nothing, and mostly cannot

`catcoms-net` performs a full NAT-traversal ladder (AutoNAT, UPnP, PCP/NAT-PMP, PCPv6
pinholes, relay circuits, DCUtR, signed address epochs). **None of it applies to a call.**
The mappings cover the libp2p transport socket; the webview's ICE agent binds its own
ephemeral UDP ports. A pinhole for the mesh port does nothing for media.

Nor can a peer's known-good mesh address be injected as an ICE candidate: the host is
right, but the port is the *mesh's* port, where no ICE agent is listening. `addIceCandidate`
would simply add a candidate that always fails.

What *does* transfer is the evidence, and `get_call_transport` now carries it: whether this
node is directly reachable, the AutoNAT verdict, its public IPv4/IPv6 literals, and which
members are offering to host.

### Pinning the WebRTC port range is not possible on WebView2

The appealing idea: constrain WebRTC to a known UDP range, then point the existing
PCP/PCPv6 code at that range so the committed pinhole work starts paying off for voice.

It cannot be done on Windows/WebView2:

- `WebRtcUdpPortRange` is a **Microsoft Edge browser policy**, not a command-line switch.
  It is registry/GPO-backed and has no Chromium switch equivalent, so it cannot be passed
  through Tauri's `additionalBrowserArguments`.
- Edge browser policies **do not apply to WebView2 by design**.
- The complete WebView2 policy set (Loader Override, Network settings, Additional) contains
  nothing WebRTC-related at all: no port range, no IP-handling policy.

CEF can do this because CEF exposes Chromium's preferences API directly. WebView2 does not.
Do not re-attempt this without first re-reading the WebView2 policy list.

Still worth testing separately: `--force-webrtc-ip-handling-policy` *is* a Chromium switch
and may fix LAN-only calls, where Chromium's default hides host candidates behind mDNS
`.local` names that WebView2 may not resolve. That is a different problem from port pinning.

### Consequences for the roadmap

1. A **member-elected relay** is no longer one option among several; it is the only viable
   fix for symmetric NAT on this target. Election reuses the switchboard's consent-gated
   offer pool (a member who already agreed to carry traffic is the right member to ask).
   The open question is the TURN implementation: the webrtc-rs `turn` crate pulls a large
   dependency tree into a project with `cargo-deny` supply-chain hygiene.
2. **Phase 4** (media over the libp2p fabric) gains value, because it sidesteps ICE
   entirely and inherits the traversal ladder that already works. Note the naive form does
   not work: a loopback TURN would publish a `127.0.0.1` relayed candidate that the remote
   peer cannot reach. It needs a synthetic per-peer address space routed over the mesh,
   which is a real overlay-network build, not a shim.
3. The CSP at `tauri.conf.json` sets `connect-src 'self' ipc: http://ipc.localhost` with no
   `stun:`/`turn:` schemes, while this document originally assumed `CSP=null`. Chromium is
   not believed to enforce CSP on ICE server URLs, so this is probably not a live fault, but
   it is a one-line thing to rule out.
