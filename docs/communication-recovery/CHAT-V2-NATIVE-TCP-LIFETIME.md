# Native TCP task lifetime (in progress; not integrated)

Chat V2 remains disabled. The Root authoring producer now uses this
separate Net constructor under focused validation; native command integration
is still absent. Existing transport constructors keep
their current behavior. No native, physical-vault or remote acceptance is claimed.

`NativeTcpPreparation` is cached before key-bearing construction. Its original
Actor token is armed before the Swarm is built. The same token moves into the
actual Actor task. The supplied Swarm Executor registers every TCP/Noise/yamux
handshake and connection future before scheduling it. A closed or full tracker
closes this original transport before dropping the rejected future while its
Actor/parent remains counted. A lone dropped pending future would otherwise
strand Swarm bookkeeping on its shared event channel. Construction
failure or unwind closes the original negative; an attempted preparation cannot
be used to build another transport.

The affine service owner closes independently of the ordinary command queue.
The task wrapper checks stop before polling its inner future; the Actor also
checks it before selects and queued command batches. Actual future destruction,
passive task custody and its counter End retire in that order, including Ready,
panic and a completely unpolled task. Completion means **closed plus zero**,
never merely idle. A retained command handle cannot delay that closure.

The tracker state and drain observers contain fee-only accounting. Actual tasks
and Executor/owner allocations separately retain passive original task custody.
Root supplies its original `SourceWork` and actual payment, without a runtime,
SDK or Source-owner backreference. Each construction attempt has its own real
metadata debit. A retained old drain therefore stays charged after key-task
completion, while another permitted attempt pays separately. A surviving Executor may retain custody
after count zero; the drain therefore establishes tracked-task completion, not
physical release or SDK drainage. Canonical Root/native retirement must keep the
same runtimes and handle interrupted retirement tasks; that integration is open.

This initial constructor supports direct TCP/Noise/yamux and member-relay
transport. DNS, WebSocket and QUIC are absent. In the locked dependency versions,
TCP/Noise/yamux futures use Swarm's Executor. Quinn's endpoint driver spawns
through its own runtime, so a Swarm-only tracker cannot claim its TLS/key work.
No existing QUIC or WebSocket path is changed or credited by these checks.

Ordinary construction leaves automatic router mapping disabled. The explicit
product variant can enable it. Mapping helpers hold public address/port
configuration and nonces, not transport keys or Root custody; their cleanup is
best effort. This contract promises no joined mapper subtree, immediate router
lease removal or complete cessation of public-address helper work.

Ten focused Net cases passed in 0.03 seconds. The complete affected Net package
passed 208 unit/integration cases, and strict all-target/all-feature Net Clippy
passed. Design/current-diff review found
two medium issues (capacity refusal and the destructor test harness); both are
fixed and re-reviewed without remaining blocker/high/medium. They cover actual
task/destructor ordering, Ready/unwind/unpolled cases, count bounds, failed QUIC
construction, a full command queue with retained handles, real TCP connection
shutdown, real executor exhaustion, lease revocation, closed snapshot refusal
and unchanged legacy handle ownership. The first 36-case Root source cohort passed after updating cold tests to wait
for their actual original transport teardown. Its new cases cover:
actual vault exclusion until the unpolled original Actor dies; a concrete local
constructor error retried only after its same drain with separate payment; and
a real constructor unwind that remains terminal despite a drained counter.
The returned-Mesh case never permits replacement. Canonical native retirement,
SDK/worker drainage, native caller integration and physical acceptance remain
open. Complete current-source suites/static checks remain pending.


A further review found that the RT Device cut does not cover independent
mount/index negatives. Root now repeats cached scope/UI/physical checks AFTER
Device entry and BEFORE key/transport construction. The entered End remains
owned on refusal. The new deterministic counter-edge regression closes only
the actual index, keeps Host/UI/source checks positive and requires zero
constructor calls with the same frozen seed/frame. The resulting 37-case Root
cohort passed (140.23 seconds), with the two separate predecessor fixture
generators unchanged. Static re-review found no remaining blocker/high/medium
in this late-admission correction.

A concrete returned constructor error is distinct from unwind. Root caches
ObservedErr before late negatives. Only that result, its SAME closed-and-zero
drain, fresh original UI/source admission and a separate new attempt debit
permit another transport attempt. Device, seed, port, token and ciphertext stay
unchanged. Missing cached Mesh or drain completion alone never establish this
permission; a returned Mesh and consuming SDK entry remain permanent boundaries.
Old drain observers retain their original actual debit, so retry cannot reuse
old metadata credit. The logical policy quotes two attempt overlaps per entry;
further retained metadata causes Busy instead of silently exceeding payment.
