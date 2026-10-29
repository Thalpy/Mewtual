# Normal Actor mailbox retirement

29 October 2026. Isolated custody correction from validated checkpoint `fbe53dab`.
The normal-terminal correction and focused lifecycle regressions pass. This does not
activate V2, change a wire format, add authority or change a resource ceiling.

## Confirmed inherited gap

`try_queue_voice_admission` reserves an actual bounded mailbox position, creates its paid
Work, invokes `VoiceAdmissionPublisher::accepted`, then sends with that SAME Permit. A
native accepted callback can pause between reservation and send. The ordinary asynchronous
command sender and SDP input/engine/media/delivery/result paths have narrower scheduling
windows between their own reservation and send too.

Normal terminal Actor branches previously dropped the command Receiver before awaiting
`AppEvent::Closed`. Installed Tokio1.53.3 `Permit::send` unconditionally publishes even
after Receiver drop. If an ordinary `ServerActor` Sender survives, a late command can stay
unread in the channel allocation after the Actor task has returned. Its original
Arguments/publisher/Work/payment then cannot retire until the last Sender disappears.
Root's earlier first-effects retirement does not destroy that command allocation.

## Narrow normal-completion correction

Every normal terminal branch first preserves existing file/Root retirement, closes the
SAME Receiver and synchronously drops already queued commands. It retains that Receiver
through `Closed` backpressure. After the normal loop exits, `recv().await` drains and drops
late commands until None. Tokio's closed receive waits for all pre-close permits to send
or be destroyed, independently of ordinary Sender handle liveness. Close prevents new
reservations; the existing64-position channel bounds remaining obligations.

The final loop never dispatches a command or enters a Root factory. Existing ordered
destructors/refusal callbacks run outside all native/Group/Policy guards, preserving their
actual callback-return and paid-last ordering. No Sender rewrite, replacement Receiver,
actor-wide registry, detached cleanup task or new allocation is introduced.

Stop ACK remains before event delivery/final drainage. It acknowledges first-effects
closure, not physical command return. `Closed` is also not a task/Work return certificate.
Event backpressure and a trusted producer holding a permit can delay normal task return;
there is no timeout-based refund or replacement permission.

## Deterministic regressions

The actual Existing grant, original AdmissionRpcPermit and real VoiceAdmissionPublisher
store the SAME accepted Work, signal a oneshot, and pause on a std-channel receiver in a
detached producer thread. The release Sender is RAII cleanup on assertion failure. Sink
guards are released before the pause. Protocol state retains ManualClock; the sanctioned
SystemClock supplies harness watchdogs only. There are no ambient sleeps or blocking joins.

Both cases retain an ordinary Actor Sender and saturate the actual event sink:

- Release the original producer after Stop ACK while `Closed` is blocked, then drain the
  event/task and require exact Stopped refusal plus immediate Work retirement. Original
  Work observers retain the full4KiB payment through actual command return.
- Keep the producer held through observed `Closed`, then give the actual Actor task a
  bounded opportunity to finish. It must remain pending until that original permit sends;
  release it, join actual task return, and require the same negative Work retirement.

The unchanged production `actor.rs` first failed BOTH regressions:0 passed,2 failed,
1341 filtered,0.03s. The first failed immediate Work retirement after actual task return;
the second failed because actual task completion preceded the held permit's return.
After the fix, the focused voice-endpoint module passes35/35, no failures/ignores,
1308 filtered,10.08s. The second regression intentionally holds the task through the
existing10-second sanctioned harness watchdog. Existing stop/freeze/cancellation, unwind
classification and payment/layout regressions also pass in that module.

Scoped formatting and strict all-target/all-feature App Clippy pass. Independent read-only design
and actual-diff adversarial reviews have no remaining findings. Commands/evidence:

```text
cargo test -p catcoms-app --all-features --lib normal_stop_ -j2 -- --nocapture
  # expected pre-fix failures; target/mailbox-retirement-before.log
cargo test -p catcoms-app --all-features --lib actor::voice_endpoint::tests -j2 -- --nocapture
  # 35 passed; target/mailbox-retirement-after.log
cargo clippy -p catcoms-app --all-targets --all-features -j2 -- -D warnings
  # passed; target/mailbox-retirement-clippy.log
rustfmt --edition 2021 --config skip_children=true --check crates/catcoms-app/src/actor.rs crates/catcoms-app/src/actor/voice_endpoint/tests/admission.rs
git diff --check HEAD
```

The primary owns complete backend/native/frontend integration suites. No full suite,
native/Tauri build, startup or frontend command ran in this isolated leaf, and no such
evidence is claimed here. The new late-permit regressions directly exercise admission;
other producers share the corrected Receiver but have no new per-producer pause hook.

## Explicit inherited limits

This joins normal terminal exits only: StopAndWait, explicit Shutdown, successful saved
freeze, command-channel exhaustion and normal transport closure. Explicit task abort,
unwind, panic-derived Studio/maintenance early returns, and a panic/block inside a command
destructor do not gain a draining continuation. They require a separately owned recovery
design if that stronger guarantee is needed. The fix does not prove publication absence,
terminal result collection, local native refund, SDK/IO drainage or a safe remint merely
because first effects are closed or the Actor's ordinary task returned.
