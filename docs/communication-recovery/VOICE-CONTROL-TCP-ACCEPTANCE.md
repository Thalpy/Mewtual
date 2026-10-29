# Admitted Control7 Actor bridge and TCP acceptance

29 October 2026. Implementation and tests are **UNRUN**. Earlier twelve in-memory
Query/Support passes do not validate this new Actor command or real TCP scenario.
No Cargo command has run on this leaf; the primary owns the shared Rust target
and complete integration gates. Narrow formatting and static diff checks only.

The trusted native bridge has six named endpoint/direct-helper ensure, refresh
and observe operations plus the existing paid admitted Presence peer snapshot.
It uses the SAME sealed VoiceActivationControl, original activation/grant and
ActivationRpcPermit. No arbitrary Root closure, renderer command, wire change,
Existing participant conversion or generic signing entry is added. Endpoint and
direct-helper outputs remain separately typed in a dedicated result/publisher/Work.
Old selection and Presence result enums stay unchanged.

The command has one boxed body under a small private arm. Its outer Arguments
explicitly frees the Box before its retirement marker; SAME Work/payment remains
outside the Box and last. Identity/payment/NativeActive/grant checks and the full
invoice guard precede body/Work allocation. The final actual Actor send fence
contains only the open check and Permit.send. Accepted/refused/completed callbacks
run outside it and outside Root/Group/Policy guards. Taking the publisher before
Root entry preserves entered uncertainty. Normal shutdown and abort close this
original control before a paused producer can send.

The production invoice includes command/envelope, body/Box publication overlap,
retirement/Work and permit headers, results, control aliases and the actual epoch
wake allowance. Its existing4KiB payment is provisional until compiled measurement;
no term may be removed to fit. Concrete native publisher cells have their own
independent payment. The TCP fixture's cell wrappers preserve that sponsor through
actual body AND funding-header deallocation, including lost observing futures.

Support Work also pairs its no-credit invocation Arc with the SAME outside
FundingOwner. Both the Work and retirement-marker aliases carry that sponsor and
drop the actual invocation allocation first. Every funding alias uses Arc::into_inner
so the final funding allocation/header frees before its plain permit/Credit. The
invoice preserves the previous larger invocation allowance and adds all new owner/
marker terms. A sixth regression holds the exact RPC fee through retired outputs
and concurrent escaped Work aliases, then requires one actual final refund.
Older Activation/Presence and the separate AdmittedAdmission generic Work pattern
still need their own allocation-tail correction; this leaf claims Support only.

The complete ACTOR transport invoice is not a complete Root-operation accounting
claim. Its existing reconciliation and paid peer-snapshot paths materialize full
MLS member vectors and View BTreeSet/Presence HashSet scratch; OpenMLS members()
also clones credentials/keys. The Group4096/member128 fees do not explicitly invoice
that transient work. This inherited large-roster accounting/work gap remains a
separate prerequisite, with the existing512-member cap and wire unchanged. Do not
hide it by reducing that cap, cutting invoice terms, or moving Clock callbacks
under Group/Policy guards. The disclosed gap does not create Support authority,
and the four-member TCP scenario cannot certify its large-roster behavior.

SupportQueryIdentity adds only pure original-allocation comparison and an immutable
historical metadata copy: query ID, complete signed-object BLAKE3 hash, signed wall
issued/expiry, LOCAL original monotonic deadline and ordinal. These values expose
no signature/key/packet bytes and confer no present authority. No Root factory
accepts the projection. A superseded query may retain metadata while actual Root
observation refuses it; the existing q2 regression now pins that distinction.

The new scenario reuses six_client_recovery's real MeshService/TCP/Noise driver,
not a new transport implementation. It admits exactly four actual members.
The existing Swarm blocked_peers rule forbids endpoint indices1↔2 before spawn;
connection/request observations independently reject any such shortcut. Helpers
are indices0/3, labelled B=max(DeviceId), D=min(DeviceId). Their Support probes are
inserted B BEFORE D at both endpoints: table insertion order determines first use.
A/C use actual native admitted activation and original point publication/ACK/
settlement with a deterministic on-curve public point. This is no SDK key-generation
or WebRTC acceptance claim. B/D only attach/unlock native actors and enable the
trusted forwarding switch; no helper controller, call, watcher or key is created.

Before starting Query, the fixture obtains exact paid admitted FreshPeer snapshots
and all four authenticated direct-helper feature1 observations. It then retains
all four genuine B/group Forward credits through final endpoint observation.
One original A→C query must use A→B, then A→D→C, with the SAME signed hash, opaque
identity, issued/expiry, local deadline and ordinal. The signed endpoint reply must
return C→D→A; actual source submissions remain at most three. The observation wait
uses the query's original local deadline, with no refresh, remint or timeout increase.

The transport's bounded64-row hash ring is an UNVERIFIED framing diagnostic. It
retains no payload/Arc/route owner and records overflow explicitly. It cannot prove
signature acceptance, support or endpoint consent. The sole positive oracle is
the actual Root EndpointSupportObservation::Observed{features:1} for that original,
which requires C's authenticated signed Support and exact query/epoch/member/binding
checks. Helper queue ACK or the transport trace cannot satisfy the assertion.

Native scalar test diagnostics pin no controller/call/activation/key generations
at B/D before and after. Brief acquisition of all eight real Watcher positions
additionally proves no charged watcher at those observations. That instantaneous
oracle alone does not prove all historical behavior; the setup never calls a
helper join/watch factory. All pressure, query, snapshot and publisher owners
return through their actual paths before native End and Actor/TCP shutdown.

Six new Actor tests cover the complete provisional invoice, foreign control/
payment/grant before Work creation, whole Root refusal, entered callback panic
ambiguity, and an accepted producer paused across actual Actor abort while an
ordinary Sender survives, plus concurrent last-Work payment return. Tests use sanctioned Clock watchdogs, RAII release and
detached producer completion, with no ambient sleeps or blocking joins.

Planned focused execution, serialized in the coherent primary checkout:

```text
cargo test -p catcoms-app --all-features --lib admitted_support -j1 -- --test-threads=1
cargo test -p catcoms-sync --all-features --lib admitted_control -j1 -- --test-threads=1
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib admitted_control_signed_endpoint_support_uses_blocked_tcp_and_same_query_replacement -j1 -- --test-threads=1 --nocapture
```

Actual-diff adversarial review, those executions, strict lint and complete backend/
frontend suites remain pending. V2 stays OFF. Availability, bilateral admission,
admitted agreement/KDF/Ready and operational SDP/ICE/RTC/capture/audio composition
remain separate unfinished boundaries; TCP protocol observations cannot certify them.
