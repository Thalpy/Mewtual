# Desktop performance and security hardening plan

Status: active, started 2026-08-20. This is the implementation checklist for the chat-lag,
startup-splitting and desktop security work. A slice is complete only after focused tests,
antagonist review, documentation, and all repository test gates pass.

## Measured baseline and first result

The initial production build put `App.svelte`, QR generation and QR decoding into one startup
chunk: **881.49 kB minified / 291.20 kB gzip**. The main CSS was **209.59 kB / 39.08 kB gzip**.
These are transfer-format measurements rather than launch timings; Tauri reads them locally, but
the webview still has to parse, compile and initialize the JavaScript.

After lazy-loading the Feedback and Wiki Help components plus QR codecs, the App chunk is
**709.43 kB / 229.90 kB gzip**. QR generation is split across 25.79 kB and 31.43 kB chunks, QR
decoding is a 130.73 kB chunk, Feedback is 4.27 kB JS + 0.75 kB CSS, and Wiki Help is 4.80 kB JS
+ 0.50 kB CSS. Those chunks load only when their feature opens (Wiki Help is warmed on entering
the Wiki). The App chunk reduction is **172.06 kB minified / 61.30 kB gzip**. The build warning
remains honest: 709 kB is still large, so `chunkSizeWarningLimit` has not been raised.

## Lag-source inventory and treatment

| Source | Why it costs | Treatment | Status |
|---|---|---|---|
| Live profile message frames | Per-row layered paint, animation and scan observers | Keep saved configuration/studio; gate live frames off without deleting code | Done |
| Unbounded message DOM | Every historical row participates in Svelte updates, style, layout and accessibility | Start with the latest 320 rows; reveal 200 at an edge; preserve scroll anchors; make jumps mount a bounded target window | Done, frontend bound; native paging remains |
| Re-rendering rich message HTML | Markdown/wikitext sanitization repeated for unchanged rows | Bounded memory-only cache keyed by server/channel/message revision and mention name; clear on lock/context change | Done |
| Whole-log placeholder scans | Every arrival re-queried all message media, remote images, emoji and reference cards | Resolve the mounted/updated row; retain a coarse retry only when resource indexes change | Done |
| Forced bottom scroll | Every snapshot wrote `scrollTop`, causing layout and yanking readers from history | Track near-bottom intent; scroll only at the tail; preserve the viewport when prepending history | Done |
| Event bursts/full snapshots | Multiple `channel-updated` events could race full `get_messages` payloads | Serialize/coalesce to one active refresh + one merged follow-up; reject stale scope/revision responses | Done; delta/native paging remains |
| Cross-server inbox scans | Each channel event can scan every server/channel | Debounce, run during browser idle time with a deadline, cancel on lock/unmount | Done; incremental native index remains |
| Large startup component | One very large Svelte component and eager optional libraries increase parse/initialization work | Real dynamic imports; move component markup, behavior and feature CSS together | Feedback + Wiki Help + QR done; more views queued |
| Server-wide search | Builds an all-channel corpus and runs filters/sorts on the UI thread | Load on explicit search only; the scan (the part that reads every message) moved to a worker (`search-index.ts` pure + `search-worker.ts`), with an inline fallback if a worker cannot start. Ordering stays on the main thread because it needs display names, and sorting matches is cheap next to finding them | Done (2026-09-03); paging the corpus itself is still queued |
| Full native history materialization | `read_messages` walks every Automerge list item and IPC serializes the full vector | Walk with one sequential list/map cursor instead of an indexed lookup per row and field; cache the materialized list per channel under its document version (`Server::with_messages`) so one walk serves every read between two changes. Then an actor-owned page query around durable id anchors with per-row reply context and a whole-channel unread summary, so the webview holds one slice (`docs/design-native-paging.md`) | Cursor walk + version cache done (2026-09-03); native paging done on `perf-native-paging` (2026-09-03): `get_message_page` / `get_pinned_messages`, webview converted, `get_messages` kept only for search corpus and moderation timeline |
| Per-event native projection sweep | Every network event (gossip frame, presence blip, receipt) re-materialized every open channel plus status, wiki, roles and Ed25519-verified moderation records to detect changes | Gate every projection on `Server::doc_version` (ops applied per document, O(1)) plus epoch/member-count for membership-derived ones. Delivery dirtying is left ungated on purpose: its one-second timer is also the wake that cancels a sync tick blocked in an outbound request (see INTERFACES § 10); gating it made `process_recovery_e2e` deadlock for the full request timeout | Done (2026-09-03) |
| Actor blocks on outbound requests | A sync tick that awaits a catch-up/PEX response cannot serve the peer's inbound request; two members requesting each other at once wait for each other until a timeout. Only the `select!` cancellation from the delivery timer or a command broke it, which made an unrelated feature load-bearing | Bounded every outbound catch-up with `CATCHUP_REQUEST_MS` (2 s, on the injected clock): the tick returns, the inbound request is served, the catch-up is re-queued. More permissive than the ~1 s cancellation it replaces, and it let the delivery gating land. The real fix, serving inbound requests while an outbound one is in flight, needs the transport shareable rather than borrowed by the tick | Bounded 2026-09-03; the restructure is queued and needs adversarial review |
| Catch-up carries the whole history | A reconnecting member re-received and re-verified every op of a document it mostly held, so every reconnect cost more than the last | `KIND_CATCHUP_SINCE`: the requester names its frontier plus its immediate ancestors, the server sends only what is behind its own and not behind that. Falls back to the full serve for a peer that does not know the kind | Done (2026-09-03); protocol-adjacent, so it wants the adversarial-review pass before release |
| Background-channel arrival fetch | The ticker fetched a whole channel's history for every arrival in a channel not on screen, to read its last row and scan for mentions | `get_message_tail(limit)` with a native `targets_me` per row (reply parents resolved against the whole channel) | Done (2026-09-03) |
| Frontend full-array reactivity | `messages` was deep `$state`, proxying every row and field on every refresh while eight deriveds walk the whole array | `$state.raw` (the array is only ever replaced wholesale); keep the sanitized-HTML cache across channel hops; in-place ticker receipt set (bounded) and media-url record instead of copy-per-add | Done (2026-09-03) |
| Remote image embeds | Network, decode, memory and layout work; also discloses the viewer IP to the image host | Keep URL/referrer validation; add click-to-load or per-server trust and fixed intrinsic placeholders | Queued |
| Voice/video/instruments | WebRTC peers, meters, rAF/timers and media decode are inherently expensive | Keep inactive code lazy, stop every track/timer on leave/lock, profile peer-count scaling | Queued |
| Global timers/animated chrome | Presence, transfers, mascot, ticker and visual effects keep waking the webview | Pause hidden/locked work, consolidate clocks, respect motion-off | Queued |
| Large settings/operations markup | Infrequent views still compile and initialize with chat | Extract Settings, Server Settings, Server Space, moderation/storage/connectivity, wiki help and recovery as typed lazy components | Queued in that order of coupling/risk |
| CSS size/style invalidation | One 200+ kB global stylesheet is parsed up front and broad selectors can invalidate widely | Move feature CSS with extracted components; audit selectors before changing shared theme tokens | Queued |
| Persistence/crypto work on mutations | Snapshots and integrity work can overlap UI-visible activity | Measure actor latency first; batch only where durability contracts permit; never weaken seals/hashes | Measured 2026-09-03 (release, one channel): full `Server::snapshot` is 2.7 ms / 393 KiB at 1k messages, 12 ms / 1.9 MiB at 5k, 47 ms / 7.6 MiB at 20k, and `persist_server` runs it (plus seal, full-file rewrite and fsync, inside the actor mailbox) after every send/edit/reaction. Not the dominant cost, but linear in history; a debounce would change the "persisted before reported" contract and needs its own decision. Addressed 2026-09-03 without touching that contract: `EncryptedDoc::snapshot` no longer re-encodes a document whose heads and op-log length have not moved (documents over 1 MiB are still re-encoded rather than duplicated in memory), and `persist_server` coalesces concurrent requests so a burst costs the writes it needs, with each request still returning only after a write whose snapshot was taken after its own change |
| Per-render image URLs and file lookups | Every avatar, banner and icon rebuilt a base64 data URL on every render of every row showing it, and each embed resolved its content address by scanning the whole file index | Memoize the URL by its bytes (`image-src.ts`, bounded, cleared at lock); index the file list by content address once | Done (2026-09-03) |

`manualChunks` is deliberately not the plan: without dynamic imports it mostly renames the same
startup payload. Raising `chunkSizeWarningLimit` would only suppress the signal.

### Measured history scaling (2026-09-03)

`cargo test -p catcoms-app --release --lib scale_probe -- --ignored --nocapture` fills one channel and
times the native costs that grow with it. Before the cursor walk, one materialization of the channel
(`Server::messages`, which the actor's change check also ran for every open channel on every network
event) cost 34 ms at 1k messages, 194 ms at 5k and 823 ms at 20k: the reason a busy old room got slower
to open and to keep up with. After the change the walk (plus the clone `messages()` hands out) is
2.8 ms / 17 ms / 72 ms, and the actor's change check between two changes, which now only hashes the
cached list, is 0.08 ms / 0.38 ms / 1.6 ms instead of repeating the walk. The remaining per-arrival
cost of a 20k-row channel is therefore one ~70 ms materialization plus the full-vector IPC to the
webview, which is what native paging is for.

## Security review and command tracking

The desktop exposes **103** custom Tauri commands. `src/tauri-command-security.ts` classifies every
one by boundary, and `tauri-command-security.test.ts` compares that ledger with both the Rust
`generate_handler!` list and literal frontend invocations. A new, removed, duplicated, dynamically
named or unclassified command fails the frontend suite.

The current review follows each path through these layers:

1. The main-window capability and CSP limit which webview code can reach IPC.
2. The native session gate denies every non-bootstrap command while explicitly locked. Networking
   actors and native persistence may continue, but plaintext projections are no longer callable;
   actor events and file-transfer progress are dropped at the native boundary while locked.
3. Tauri handlers validate sizes, shapes, ids, paths and URL destinations before local side effects.
4. Server operations go through the registered `catcoms-app` actor; UI role visibility is never
   treated as authorization.
5. Protocol/crypto enforcement and honest-client residuals remain exactly as documented in
   `THREAT-MODEL.md` (notably R2, R6 and R7).

### Findings in this slice

| Finding | Severity | Resolution |
|---|---|---|
| Explicit lock cleared the UI but left actor-backed IPC/events callable | High local boundary | Closed: native session gate on every non-bootstrap command, native event suppression, and long-download rechecks; focused Rust test proves a mounted vault remains available to native background work while IPC is refused |
| Slow history response could populate a newly selected channel | Medium integrity/confidentiality-in-UI | Closed: capture server/channel plus monotonic request revision and discard stale responses |
| Feedback launcher accepted an unbounded prefix-matching string | Low OS-boundary DoS/hardening | Closed: 6,000-byte/control-character native bound plus structural frontend origin/path check and tests |
| Cached rendered plaintext could outlive its useful context | Medium local exposure | Closed: cache is memory-only, capacity 640, revision-aware, and cleared on server/channel change and lock |
| Command surface could drift without review | Medium maintenance risk | Closed: exhaustive executable ledger for all 100 commands |
| Tauri core window, signed updater and process-restart plugin permissions are capability-scoped, not session-gated | Low availability | Accepted for now: they cannot read vault projections or install unsigned code, but compromised webview code could close/restart the app; revisit with separate locked/unlocked windows or narrower updater orchestration |

Open inherited protocol residuals R2, R6 and R7 are not relabelled or weakened by this work.
Remote-image privacy and native paged-history semantics need their own antagonist review before
implementation.

## Remaining implementation sequence

1. Extract the low-coupling overlays and operations views; keep feature CSS and optional libraries
   behind the same dynamic boundary. Add typed props and explicit loading/error fallbacks.
2. Extract Settings and Server Settings by page, preserving drafts across chunk loads and keeping
   every mutation in a typed native service wrapper.
3. ~~Design actor-owned paged history: stable message-id anchors, bounded limits, edits/deletes,
   unread/search behavior, and legacy id-less rows. Review before changing the bridge contract.~~
   Done (`perf-native-paging`, `docs/design-native-paging.md`).
4. Move server-wide search/filter/sort to an on-demand worker over paged data.
5. Make remote media consent-aware and fixed-size before fetch; then profile voice/video and pause
   nonessential timers while hidden or locked.
6. Run an adversarial pass over each changed boundary, update this document and the threat model,
   and run every mandatory frontend/root/Tauri suite plus production build checks.

## Acceptance measurements

- Report App and feature chunk minified/gzip sizes after every extraction; do not hide warnings.
- Add a deterministic 10,000-message frontend fixture: initial DOM rows must remain at or below
  the configured bound, history jumps must land, and reading position must survive prepend/append.
- Record unlock-to-interactive, channel-open, message-arrival and scroll-frame timings on the same
  release build/hardware before and after native paging.
- Confirm lock rejects representative read, mutation, OS-launch and pairing commands while actors
  continue native background work.
- Keep the 100-command ledger test, CSP, full suites and documented R2/R6/R7 residuals green.
