# Six-client native recovery regression

The fixture in `apps/desktop/src-tauri/src/six_client_recovery.rs` runs six independent
identities and encrypted stores over real loopback TCP/libp2p transports in one test process.
Both directions of **2–3** and **5–6** are blocked in each swarm before it starts, including
every restored generation. It records actual connections and received/completed requests and
rejects any forbidden edge. Forwarding through another current member remains allowed.

## Scenario

Membership stays fixed after the original six admissions. Every history assertion compares the
complete map of native-accepted message IDs to original author fingerprints and exact text;
duplicate IDs are rejected separately.

1. Admit all six, exchange a common conversation, then close every client.
2. Reopen all six from their own stores, verify the first history, exchange another common batch,
   and verify convergence.
3. Close 4–6. Clients 1–3 author and converge through 1, since 2 and 3 cannot connect directly.
   Close 1–3.
4. Reopen only 4, durably accept a solo message, verify its local history, and close it.
5. Reopen only 5 and 6. Each durably accepts its own message. After a full periodic discovery
   pass, each still has only the common history plus its own new message: neither the other
   isolated message, 4's solo message, nor the 1–3 batch is present.
6. Reopen 4. Clients 4–6 must converge to the common history plus 4's solo message and both
   isolated messages, without acquiring the still-offline 1–3 batch.
7. Reopen 1–3. All six must converge to the complete union with the same IDs and authors.
8. Close all six. Reopen each client alone, one at a time, and require the entire union from that
   client's own encrypted store before closing it again.

While 4 is alone, and while 5 and 6 remain mutually isolated after the periodic discovery pass,
the fixture queries each actor's delivery snapshot for exactly its new native-accepted IDs.
Each ID must be present once with `delivered == 0` and `any_peer == false`. Local durable
acceptance therefore cannot be mistaken for remote delivery proof. These negative assertions
do not apply to old common-history IDs, which may have legitimate prior delivery evidence.

## Native paths and bootstrap boundary

Sends call `durable_chat::send` with a stable token and original authoring context. The fixture
requires durable acceptance and retains that returned message ID. A busy native serializer may
be retried only with the same token/context. Close calls `shutdown::freeze_servers` while the
persistence, capture and discovery workers remain active, then commits stop and releases them.
An existing driver-owned watch channel must close before that client is considered shut down.

Reopen mounts a fresh `AppState` and `ServerStore`, reserves the saved transport sequence through
`load_or_init_server_net`, and calls `restore_server_actor`. It verifies the original transport,
group and device identities. Persistence uses `note_event_for_persistence` and
`run_persistence_worker`; route capture uses `member_reconnect::run_capture_worker` and the native
capture wrapper.

Only original admission permits explicit fixture dials. Additional allowed connections run from
higher-numbered clients to lower-numbered listeners so outbound Noise evidence can establish
the necessary saved directions, including 2→1, 3→1, 5→4 and 6→4. The fixture waits for production
workers to save these routes under authenticated member-mesh policy. Signed public addresses,
relay configuration and rendezvous configuration remain empty. After the first collective close,
there are no fixture dials, manual catch-up calls, injected cache entries, or inserted histories:
native restore and discovery must use the locally proven, sealed routes.

## Timing and hostile variant

The native host and fixture share `discovery_timer::run`, including the initial `[0, 5)` second
spread and subsequent `[45, 75)` second interval. The fixture chooses different deterministic
points inside those ranges for each client. A shared injected `ManualClock` advances 100 ms per
100 ms of `SystemClock` time during admission, then 500 ms per 100 ms afterward. Thus logical
discovery periods take approximately 9–15 real seconds; this acceleration also applies to actor
deadlines, rather than replacing discovery with a test-only fast loop. Outer operation/phase/test
bounds and native storage-worker sleeps use `SystemClock`.

`six_client_native_restart_and_partition_recovery` uses the normal order. The second test,
`six_client_native_reversed_reopen_gossip_loss_and_command_load`, reverses each multi-client
reopen, discards incoming gossip after all admissions, and delays completed request replies by
125 real milliseconds during the bridge/reunion phases. The delay also covers cancellable
requests without replacing the real transport's cancellation ownership. Repeated actor reads
run every 5 ms, beginning on 5 and 6 before 4 returns. The exact-union assertion must succeed
while load remains active; counters require actual reads, discarded gossip and delayed replies.

## Limits and execution ledger

This is a headless native-adapter test, not six operating-system processes or renderer/window
automation. It does not emulate physical NAT, routers, relay infrastructure, platform interface
sampling, or platform address-cache collection. Fixed membership, ordinary close, one existing
chat channel and bounded synthetic messages do not establish crash/power-loss durability,
membership-change correctness, arbitrary archive performance, or adversarial-peer security.
Separate protocol, authority, storage-failure and cancellation tests remain necessary.

Execution is pending the root agent's serial native build and test run. Static API inspection,
formatting and whitespace checks completed; independent review found no remaining blocker/high
after correcting worker lifetime during close and replacing an exclusive socket-bind teardown
probe with the real driver lifetime signal. Record actual command, revision, selected counts,
failures and final results here after execution; this document does not claim either test passed.
