# Local browser voice memory policy

This inactive V2 continuation keeps the original 64 KiB device-configuration payment,
copying rules and seventeen metadata-loan bound. It raises the browser's default local
group budget from 256 KiB to 512 KiB. The separate process aggregate remains 2 MiB.
Native Root, wire/body, peer/action, queue, execution and SDK limits do not change.
The individual generic crypto-job maximum remains 256 KiB, including when a local
group budget is 512 or 1024 KiB; only separately bounded work may fill the aggregate.
These are tracked application-owned buffers and metadata, not total WebView/RTC/OS RAM.

`voiceBrowserBudget` is a local-device policy seam, with integer KiB selections from
256 through 1024. No settings UI or persistence is included. Its immutable primitive
fields are captured once before creating the original broker ledger; caller mutation
cannot change that ledger. A future settings adapter must obtain this selection from
the local device and propagate it to the retained broker factory, never use server,
invite or incoming peer fields as the source. A changed selection requires a reload.

Broker ABI50 uses the existing retained window symbol. An older broker or a different
requested policy refuses reuse without replacing the original broker or its ledger.
Original cleanup stays callable. Both raising and lowering a requested limit leave all
existing work and debt under the original policy, even after that work returns.

## Measured composition

The actual authenticated endpoint fixture now exercises a distinct inactive
`nativeConfiguredAdmissionAppCalls` constructor. It fixes admission mode before channel
construction, performs mode29 plus the noncreating30 capability probe before ACK, and
starts one original 128 KiB capacity preclaim. Pilot factories retain their original
mode. Neither constructor can upgrade the other's retained App session.

The measured original group charge after pairing is **236 KiB**:

| Owner | KiB |
| --- | ---: |
| Retained App session | 8 |
| Device RTC configuration | 64 |
| Selected join key | 4 |
| Physical Control codec | 8 |
| Physical work/cleanup metadata | 16 |
| Separate admission coordinator | 8 |
| Original browser preclaim | 128 |

The 8 KiB setup phase has actually returned before that preclaim, so it is excluded.
The actual separately implemented capture pool (4 KiB) and permission/master capsule
(8 KiB), paid through the SAME original broker ledger in the test, bring this to
**248 KiB**. The capture helper still needs production call/media wiring; the test uses
its receipt-boundary fixture and does not manufacture native media authorization.

Sixteen indexed peers would add sixteen existing 8 KiB monitors plus sixteen 2 KiB
compact markers: **408 KiB**, leaving **104 KiB** at the default. This last figure is
a structural inventory, not actual sixteen-peer admission/RTC acceptance. The existing
actual two-peer pilot RTC tests remain part of focused validation. Seventeen configuration
loans are metadata headroom for sixteen indexed peers plus one in-progress negotiation;
they do not authorize seventeen peers or extra native execution.

The lower 256 KiB selection is still enforced exactly. The test fills its remaining
8 KiB with an unrelated retained payment and proves that another byte refuses. It does
not silently raise that policy to admit additional work.

## Actual cleanup boundary

The combined regression holds real capture-helper `getUserMedia` work while End closes
the App and the original native capacity cancellation is still uncertain. The 128 KiB
preclaim remains until authenticated exact native return and local framing return.
After App closure, the independent old capture keeps its original 12 KiB group charge;
another scope cannot start a replacement permission request. Only actual late SDK return,
track disable/stop and original finalizer return release that old capsule and binding.
The fixed 4 KiB process pool remains paid. Existing failed-stop and reentrant SDK tests
continue to exercise the same helper.

V2 stays OFF. App `ready` still means authenticated bootstrap/local lifecycle only.
Native bilateral Ready, operational admission-mode SDP/RTC, microphone/playback wiring,
media relay and combined/physical acceptance remain outside this checkpoint. The primary
agent owns the complete integrated suites and final global handover.

## Focused verification

The ten relevant files pass **574/574** focused Node tests, including the existing
actual two-peer pilot RTC cases and capture/funding/configuration cleanup cases.
`target/browser-budget-focused-final.log` and `.exit` retain the result. The three
new composition/mode cases also pass separately after the inert mock's type annotation.
Strict targeted TypeScript for the shipped changed modules and the new policy test
passes (`target/browser-budget-types-final.log` and `.exit`); diff checking passes.
The primary will run the full suites, static/build, flows and startup after integration.

The subsequent complete hosted frontend run at `153c573f` has 3,647 passes and one
failure outside that focused subset: an older confirmation-pressure unit assumes the
default is always 256 KiB. Its test-only correction explicitly selects the supported
256 KiB policy, preserving the exact original cap, pre-admission refusal and late-engine
return assertions. Other fixtures keep the new default. The confirmation/policy subset
passes 36/36 afterward and read-only review has no findings; complete validation is pending.

Initial focused checks exposed stale 200/256 KiB pressure fillers; the fixtures now
fill the selected aggregate with individually bounded jobs or original retained payments.
All exact refusal and full-queue assertions remain. A new exact-job-boundary unit also
needed to join the actual job return before checking its refund; production refund
ordering was not changed. An exploratory strict compile including the large legacy IPC
test file reports BufferSource/remount/control-flow typing errors outside the repository's
configured shipped-source check; those Node tests are excluded by its tsconfig.

The implementation's own read-only adversarial review and re-review report no remaining
blocker/high/medium. Its confirmed single-job-limit drift was corrected before handoff.
The low-priority global documentation update remains with the primary: current ABI50,
512 KiB default and resolved local budget obstacle must be recorded above older ABI49/
256 KiB checkpoint sections in architecture, interfaces, threat model and handover docs.
