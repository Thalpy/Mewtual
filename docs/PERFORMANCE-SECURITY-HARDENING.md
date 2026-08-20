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
| Server-wide search | Builds an all-channel corpus and runs filters/sorts on the UI thread | Load on explicit search only; next move indexing/filtering to a worker and page results | Queued |
| Full native history materialization | `read_messages` walks every Automerge list item and IPC serializes the full vector | Add an actor-owned paged/window query with total/anchors; preserve exact search/unread semantics | Queued; compatibility/security design required |
| Remote image embeds | Network, decode, memory and layout work; also discloses the viewer IP to the image host | Keep URL/referrer validation; add click-to-load or per-server trust and fixed intrinsic placeholders | Queued |
| Voice/video/instruments | WebRTC peers, meters, rAF/timers and media decode are inherently expensive | Keep inactive code lazy, stop every track/timer on leave/lock, profile peer-count scaling | Queued |
| Global timers/animated chrome | Presence, transfers, mascot, ticker and visual effects keep waking the webview | Pause hidden/locked work, consolidate clocks, respect motion-off | Queued |
| Large settings/operations markup | Infrequent views still compile and initialize with chat | Extract Settings, Server Settings, Server Space, moderation/storage/connectivity, wiki help and recovery as typed lazy components | Queued in that order of coupling/risk |
| CSS size/style invalidation | One 200+ kB global stylesheet is parsed up front and broad selectors can invalidate widely | Move feature CSS with extracted components; audit selectors before changing shared theme tokens | Queued |
| Persistence/crypto work on mutations | Snapshots and integrity work can overlap UI-visible activity | Measure actor latency first; batch only where durability contracts permit; never weaken seals/hashes | Measurement queued |

`manualChunks` is deliberately not the plan: without dynamic imports it mostly renames the same
startup payload. Raising `chunkSizeWarningLimit` would only suppress the signal.

## Security review and command tracking

The desktop exposes **100** custom Tauri commands. `src/tauri-command-security.ts` classifies every
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
3. Design actor-owned paged history: stable message-id anchors, bounded limits, edits/deletes,
   unread/search behavior, and legacy id-less rows. Review before changing the bridge contract.
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
