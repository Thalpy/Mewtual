# Admitted key publication and shared discovery

This isolated Root slice is inactive. Its implementation and focused regressions are
drafted; Rust compilation, execution, measured inventories and strict lint are pending.
Production V2 remains OFF. A successful key publication is not agreement, Ready, SDP,
ICE, RTC, capture consent or relay permission.

`start_admitted_voice_presence(grant, original, join, point)` requires the SAME actual
Root instance/resources, directory-issued child, selected residence and NativeActive
original. Native supplies the canonical Owner's join and already on-curve validated
frozen point; the raw trusted Rust argument is not independent curve-validation proof.
Every positive Root turn revalidates current membership, epoch, directory and original.
Exact retries preserve the first tuple without another owner or RNG use. Changed tuples
refuse. The opaque receipt matches the exact original/Weak grant/tuple and records only
historical installation. Native must independently recheck its original current Handle
before returning positive success; End does not erase accepted history.

The dedicated `AdmittedPresence` contains no legacy LocalJoin, PacketLedger,
Confirmations or ActionQueue. Existing key/pair/derive/packet/Ready constructors remain
profile-rejecting. The unique RootResidence owns the implicit Watcher and separately
paid Presence Box. Closing the original precedes actual Presence/Watcher destruction;
the observer is then destroyed, and the residence ReturnMarker remains last. Original
native task, Actor Work, escaping receipt, callback and lower-flight custody are separate
owners and must keep their own original payments until actual return.

The private watcher child is unregistered and cannot become an Actor/directory call
grant. It has an independent one-shot watcher claim, the exact admitted parent, and
the SAME dedicated Presence Funding Arc. Parent supplies actual issuer/room/epoch
closure. Escaping FreshPeer/WatchChallenge Source aliases keep that payment through
the child's final lease/watch allocation destruction. Root residence return alone
therefore does not certify the complete Presence payment has returned. Native peer
snapshots use the existing paid PeerSnapshotPermit/output; no public bare Vec escapes.

Shared Announcement1, WatchChallenge2 and JoinProof3 bytes/domains are unchanged.
They authenticate room/join/key liveness. FreshPeer intentionally carries no admitted
profile/support claim. Compatibility must later come from current authenticated new
controls and the committed agreement. Ingress captures the original admitted watch
lease/serial; retained old work cannot adopt a replacement admitted watcher or produce
a replacement ACK. Existing observer candidate behavior is unchanged.

An opaque discovery Source seal permits only Announcement/JoinProof with no packet
authority, through a dedicated private Root/Source entry. SourceActivity storage and
existing Body/SDP/Source enums stay unchanged. Ordinary Source admission still refuses
admitted joined producers. Existing-source retirement skips admitted discovery. Final
publication and lower FlightAuthority check the SAME admitted lease's NativeActive
phase as well as its live bit, covering invalidate's CLOSED-before-live=false interval.
Current Actor/member/room/policy and existing origin/rate/attempt fences still apply.

Presence reserves an explicit8KiB within unchanged512KiB/group and2MiB/process limits.
Its plain Credit stays outside the Box through actual deallocation and includes the
fixed32-slot proof-cache Vec backing, watcher lease allocation and response/receipt
overlap. Each proof reserves the existing1536-byte fee before its Box allocation and
keeps that plain Credit outside its Box through deallocation. Cache hits preserve the
same signed result/deadline and do not charge another record. Existing Watcher caches
retain their existing independent fees/caps. New compiled growth guards must pass;
the source formulas alone are not execution evidence or allocator/RSS bounds.

Draft regressions cover pre-NativeActive/foreign-Root/counterfeit-grant refusal, immutable
first tuple and exact retry, old constructor fences, actual shared discovery Source and
private ingress exchange between Existing/admitted owners, profile-neutral liveness,
stale recipients, final Source closure, retained Lower payment, Root-clear funding and
the complete new retained/temporary graph. The deterministic ingress exchange starts
after real route establishment; it does not claim live forwarding or audio.

Read-only design and actual-diff review found and resolved wrong-room recipient
selection, bare peer-vector output and watcher-child payer-tail issues. The final
static re-review has no remaining findings. Nine drafted tests and all compiled
measurements/lint remain unrun; narrow rustfmt and diff whitespace checks pass.

Operational blockers remain sticky operation31 selection, complete neutral6/6 consume,
native parent-row consume, immutable RoundGate, new control/encrypted/Ready producers
and ingress, exact A/R/KDF agreement, SDP/ICE/RTC integration and production relay/audio.
No FreshPeer or publication receipt substitutes for any of those gates. The primary
owns combined root/native/frontend suites and application startup after integration.
