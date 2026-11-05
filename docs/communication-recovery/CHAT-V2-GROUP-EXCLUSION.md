# Negative V2 group exclusion before Current activation

This packet adds actual negative old-protocol consumers. It does not support ID3, create group
directories, issue Current, or implement the later activation/metadata/scope/node transaction.
The separate namespace codec now has the opaque physical opener/historical DATA reader;
pin/head/activation metadata remains test-only. Eleven new negative-fence regressions are authored
UNRUN under the user's compilation/test/build pause; source review is not execution evidence.
Design and final actual-diff adversarial static review are clear after the stale-pin race and
regression-reach corrections. Direct rustfmt, whitespace, ambient and raw-filesystem checks pass.

An existing ID2 mount now derives the fixed `g-<64 lower hex>` destination from its SAME held
vault/root and complete group, without signer input. Before the ordinary per-signer target check,
any group-object presence refuses the old path. Empty directories, ordinary files, dangling
links, unreadable objects and unknown partial metadata remain reserved; nothing is opened as
Current or removed. The actual held Root/marker identities and complete marker frame are checked
around the fixed lookup, so a missing/replaced parent cannot become false child absence.

This applies even when no ID3 descriptor is visible, preserving the group fence across an older
catalog-component replay. It is not a monotonic antirollback guarantee: replacing the complete
authenticated layout/root or externally altering a cooperative private vault directory remains
the existing threat-model residual. There is no lifetime directory scan or group collection.

Existing admission issuance, final save, positive resolution/storage and source-absent negative
recovery already consume the ordinary target predicate. Generic pre-await captures also check
the new group fence. Already-captured old owners cannot bypass it. Deliberate retirement and
same-original negative repair retain their existing semantics; unrelated groups still progress.
Numeric load/save and immutable pin capture now observe the group fence too. Actual old pin
mutations retain admission before pins; the nested pin validator holds pins through Root without
re-entering admission. The borrowed pin guard checks the SAME original preparation's mount,
nonce/revision/provenance/owner/negative after admission acquisition and again after pins/Root
waits before mutation/flush. A genuine founder regression retires that reservation before the
guard wins, then separately revokes it after admission: neither path creates/rewrites a pin.
If this exact initializer already saved its head but its Current scope parent-sync failed,
the constructor first resyncs the SAME outstanding table under admission, then requires clean
original custody before pins. It creates no row/nonce/owner and reruns no covering callback.
Two real initializer regressions pin automatic recovery and revoked-before-repair refusal.
This retains the existing synchronous scope-I/O obligation; it adds no new paid Scope grant.
Future physical group creation must use these same serializers and an
actual independently paid strong pending-debt registry before any mkdir. That registry and the
permanent directory receipt are NOT implemented by this packet.

Before the old ID2 constructor reaches any older gate/cleaner, it refuses ANY current-declaration
seed or `current-layout.bin` presence. The fixed orphan guard also refuses a seed before making
a new vault/DEK. Unknown bytes are preserved. Supporting those artifacts later requires the
complete independently paid ID3 inspector/declaration, not simply bypassing this refusal.

New V2 founder pre-row preparation/registration/effects also use the fixed-five JOIN inventory
by GROUP, across all signers. Current plus Store-admitted pending expected/next summaries are
checked while the actual admission writer is held, before pins/canonical/Root or RNG. A cold,
Paused, dropped-task or unwritten admitted JOIN still owns its original invite/KP even when
no admission row or pin/head exists. The old per-signer predicate remains appropriate for V1
same-identity operations; the group predicate grants no new admission or source authority.

The small selector uses fixed stack hash/hex buffers and serial checked root/child paths within
the existing <=4096-byte physical-open profile and mounted metadata allowance. It introduces no
large decoder, SDK call, file-content allocation, directory walk or host-budget claim. Full ID3
opener/index/resource and Current publishers remain separate reviewed implementation gates.
