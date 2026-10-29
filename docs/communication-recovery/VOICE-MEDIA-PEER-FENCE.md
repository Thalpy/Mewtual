# Individual media closure before native submission

This inactive V2 correction fences a positive native media request against the exact
browser peer that owns it. A live shared IPC Session does not make a closed peer current.

Before queueing, `mediaContext` snapshots the original private peer record and driver.
The existing framing checks validate that same registry allocation, driver and negative
state after the owner callback, after each SDK/transport await, and before first fetch.
No current-negotiation generation is required: an indexed sibling must survive another
negotiation's retirement or replacement. Individual closure does not abort the shared
controller or close another peer.

A request refused while queued spends no nonce. Closure during encryption keeps the
already spent nonce; it cannot be refunded or reused. An ambiguous transport completion
retains the original cleanup obligations. Negative Close, Release and history recovery
use their existing authenticated route after closure. No native grant, retry allowance,
wire format, deadline or resource ceiling changes.

The independently paid 8 KiB media graph includes an additional conservative 128 bytes
for one fixed two-reference request snapshot and queued/frame/check references. It retains
no old negotiation or SDK graph. Positive-flight return precedes negative cleanup requests.
The logical application-owned inventory is setup 7,728, adopted 6,272, closing-wait 6,976
and closing-joined 6,080 bytes. This is not a bound on opaque browser/RTC heap or RSS.
Native Rust Control layouts and browser physical buffer limits are unchanged.

Retained broker ABI51 requires reload for older brokers, including ABI50. The original
window slot and its cleanup stay available; a new module cannot reuse the unfenced old
class or create another pool alongside its unresolved work.

Six actual retained-channel regressions cover Admit, Current and Detach at shared-lane
and encryption barriers. Each failed before the fix by first-submitting after individual
Close. They now preserve the indexed sibling, keep original work paid, complete original
negative cleanup, and prove increasing Session nonces and eventual payment return.
The native stand-in also models a negative claim before any Admit, without inventing
admission. The IPC/media/status subset passes 453 tests after the ABI51 update; the final
six strengthened nonce barriers plus old-ABI50 refusal pass 7/7. Strict shipped-source
TypeScript checking passes. Read-only adversarial review/re-review has no remaining
findings. Native actor/TCP, complete-suite and physical evidence are separate gates.

V2 remains disabled. This correction does not establish operational bilateral admission,
production audio, media relay, or physical network acceptance.
