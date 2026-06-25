# Design — E2E group voice

Real-time voice for a server's members, end-to-end encrypted, reusing what CatComs already has.

## Architecture (Path A — WebRTC in the webview)

- **Media plane:** the Tauri webview's WebRTC (`getUserMedia(audio)` + a mesh of `RTCPeerConnection`s,
  one per other participant). Codec/jitter/AEC/PLC come free from the embedded libwebrtc. Mesh is fine
  for small groups (uplink = (n−1)×~32 kbit/s Opus); ceiling ~8.
- **E2E:** each audio frame is AEAD-encrypted (WebRTC Encoded Transform / `RTCRtpSender.transform`)
  with a **media key derived from the MLS group secret** — so even if media is ever relayed, only
  members can decrypt. The key is the same for all members (MLS exporter), rotates with the epoch.
- **Signalling:** SDP offers/answers + ICE candidates are relayed member-to-member over a new
  **authenticated push** request (`KIND_CALL_SIGNAL`, mirroring `KIND_DM_INVITE`) — members-only,
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
  core only relays it authenticated + members-only. No dedup (unlike DM invites — every ICE candidate
  must arrive); FIFO-bounded queue.
- The media key is derived locally by each member from MLS state — it is **never sent on the wire**.
- A "call" is identified by a random `callId` (u128) chosen by the initiator; the media key is
  `export_secret(MEDIA_LABEL, callId)`, so distinct calls have distinct keys.
