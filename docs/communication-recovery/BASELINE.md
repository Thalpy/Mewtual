# Communication recovery baseline

Inspected commit: `48f906962b37ed71a967c04ef4978d019ff43586`.
Local branch: `Chat-method-redesign`, clean before this work. The supplied plan names
`gate4-agent1-runtime`; both refer to the inspected commit, but this work remains on the user's
checked-out branch. Current changes are uncommitted for review.

Platform: Windows, PowerShell, repository Rust 1.89.0, existing desktop Node dependencies.
No feature flag, wire tag, storage format, cryptographic primitive or network permission is added.
The new native `send_message` return value requires the matching frontend from this patch.
ServerNet writes version 4 and reads versions 1-4; request/response uses `/catcoms/rr/1`.
Those versions remain unchanged. This is a focused baseline, not the complete CR00 inventory.

| Classification | Observation | Production seam |
| --- | --- | --- |
| Source-confirmed | Reply/helper/switchboard socket reuse policy differs from direct named-inviter admission | `reconnect_policy_after_admission`, `join_server_inner` |
| Source-confirmed | Early member PEX was only performed when direct reconnect was authorized | `join_server_inner` |
| Source-confirmed | `persist_server` discarded failure outcomes while send logged persisted | `persist_captured`, `send_message` |
| Existing invariant | Snapshot tickets retire only after a successful write; server slot reuse is incarnation-fenced | `PersistCounters`, `persist_captured` |
| Source-confirmed | Mounted unlock authenticates and projects existing actors without an immediate discovery wake | `authenticate_mounted_store`, `finalize_unlock_session` |
| Existing invariant | Explicit UI lock leaves actors mounted and suppresses plaintext native events | `lock_session_inner`, `forward_events` |
| Source-confirmed | Previously checked sources can become useful after learning from a third peer without changing the local socket/document | catch-up checked-source tracking, `DriveDiscovery` |
| Existing invariant | Original signed operations are retained and served through paginated catch-up | `EncryptedDoc`, `serve_catchup_since`, `request_catchup_since` |
| Existing invariant | Actor opens shared channel index and listed channels independently of the displayed tab | `spawn_server`, discovered-channel integration |
| Hypothesis / unrun | Original direct-fails/reply-succeeds immediate restart across two actual machines | Requires asymmetric transport and independently sealed stores |
| Source-confirmed gap | Native snapshot follows actor publication; no commit-before-publication transaction is added here | actor send, `ChannelSync::post`, native send |
| Source-confirmed gap, targeted by this patch | Receive-only chat emits UI updates but does not request a snapshot; normal UI close only saves continuity | actor receive/catch-up, `forward_events`, `lock_session_with_generation_inner` |
| Source-confirmed gap, targeted by this patch | Projection invalidation misses signed operations that do not change displayed rows and epoch changes with unchanged member count | actor `DocVersions` and rendered-change checks |
| Unrun | True account teardown, sleep/resume, long-absence control recovery and bridge process restart | Full CR03/CR07/CR08 acceptance |

## Limits used by this implementation

These are checked current source constants/defaults, including the explicitly identified additions
below. They are not a new calibrated resource profile or measured latency guarantees.

| Boundary | Value / source |
| --- | --- |
| Discovery cadence | 60 seconds with +/-15-second jitter, first pass spread over 5 seconds |
| Early admission PEX | 3 seconds, existing connection only |
| PEX | 64 records; 8 addresses/record; 256 bytes/address; 512 KiB response; 4 peers per discovery pass |
| Catch-up queue | `SyncConfig::max_catchup_queue`, default 256 |
| Known peers | `SyncConfig::max_known_peers`, default 64 |
| Catch-up frontier | 512 heads/parents |
| Export chunk / accepted response | 256 KiB / 16 MiB |
| Nonprogressing source / tracking entries | 8 rounds / 256 entries |
| Failed-source cooldown | Minimum 30 seconds; this patch adds 1 second in one peer-order direction |
| Periodic reconciliation pacing | At most one new sweep per 30 seconds; existing pending work retained |
| New received-state snapshot worker | One watch-coalesced worker per installed server; startup snapshot then fixed 250-ms batching windows |
| New raw snapshot invalidation | O(current open documents) operation-count map comparison per actor turn plus MLS epoch; no body scan |
| Retained past epochs / control log | Defaults 8 / 256; not expanded |

Reconciliation uses the existing signed-operation authorization and endpoint scheduler. It does
not add opaque forwarding or promote an inbound socket to a dialable listener. Queue saturation
by permanently unfinished tasks and outbound waits inside the sole actor remain broader design
limits. The simultaneous reciprocal regression tests recovery from one specific wait cycle; it
is not evidence that inbound work is always serviced while every outbound request is pending.

Receive-only history now requests persistence through `SnapshotNeeded` independently of rendered
changes. Native handling marks the current server incarnation dirty before checking UI lock and
saves through the existing snapshot ticket/write lock. The worker's startup save covers state
created before registration; Closed, leave, stream exhaustion and replacement retire only the
matching signal. Saving remains asynchronous. Process/actor failure before a covering write can
lose accepted in-memory work, and a wake is not a durability or custody acknowledgement. The new
tracker covers open legacy-document identities/counts and MLS epoch, not all snapshot fields;
Studio's existing separate persistence barriers are unchanged.

## Evidence discipline

Test commands/results and review findings belong in [STATUS.md](STATUS.md). A helper or
deterministic transport test is not a running-app reproduction. No physical-device, NAT,
Linux/macOS or CI result is claimed. The initial Windows debug builds exhausted disk and failed
linking; verification is retried with debug symbols and incremental compilation disabled, after
removing only regenerable Rust build artifacts within this repository.
