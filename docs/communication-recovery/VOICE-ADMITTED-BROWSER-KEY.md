# Retained admitted browser key

This inactive browser leaf adds original-key custody for the native admitted call
profile. It does not activate App V2 or implement admitted KDF, agreement, Ready,
SDP, RTC or relay. The native commands are supplied by the separate admitted-key
leaf; WebCrypto fixture tests are not native, audible or physical-network evidence.

`VoiceCryptoBroker.selectAdmittedJoinKey` requires the exact frozen adapter from
`voiceAdmittedJoinKeyNative`. Its factory brand prevents accidental adapter
mixing, not hostile JavaScript in the same realm. Native remains responsible for
the original issuer profile, activation, UI generation, grant and epoch.
The same-window versioned weak registry survives adapter-module reloads without
owning adapters or work strongly. Unknown registry storage fails closed. A new
factory instance may serve a new owner; it cannot replace an existing selection's
original adapter, and copied/unregistered adapters remain refused.

The key and selected record have immutable profiles. Both profiles share the same
byte ledger,128-key/16-per-owner bounds, two provider positions and32-job queue.
The admitted opaque handle offers `publicPoint()` and has its own private WeakMap.
It never enters the Existing `joinKeys` map, bind/derive/pilot/channel factories or
the old App-session factory. No private key is exported or passed over IPC. A
future admitted derivation must use this original key with its new transcript and
domains; `computeVoicePair` is not that operation.

There is one nonextractable P-256 generation and one public export. Lost admission
or publication replies retry only the same native owner/executor/permit/point.
Once publication might have entered, End continues exact negative history recovery
without relying on the live browser-owner predicate. Exceptions are unknown; closed
terminal facts come only from the four closed native reply variants:

- `accepted`: exact historical Root publication. The browser still checks current
  ownership before exposing a key. A historical receipt grants no current RTC use.
- `rejected`: known Root refusal or proven local unstarted callback.
- `closed`: ended or unconfirmed exact native terminal cleanup, never Root rejection.
- `not-started`: native's narrow closed-generation proof that no publication began.

For known terminal history the broker explicitly acknowledges **before** sending
SDK settlement. The actual SDK promise has returned, but its native engine charge
keeps the same canonical terminal cell present while an ACK reply is lost. Only a
successful ACK response permits SDK settlement. `not-started` needs no publication
ACK. A provider failure before publication also settles its original generation
without inventing a terminal publication. Unknown/malformed responses retain paid
cleanup and wait for an explicit availability/End wake; they do not poll or regenerate.

A known result may reach its observer while ACK/settlement is pending. That does
not signal cleanup completion. `retireNativeOwner` joins the actual original job.
Escaped admitted handles and retirement callbacks capture detachable cells; private
targets are removed only after SDK/native cleanup, queue-job return and every entered
synchronous dispatch. Then the private key is dropped, point zeroed, maps removed
and original fee returned. Inert caller-owned cells retain no original broker, key
or native cleanup graph.

Broker ABI52 keeps the original window symbol. ABI51 and older are refused while
their cleanup remains callable; no second pool replaces an unresolved broker.

The separate admitted metadata fee is8KiB; Existing metadata stays4KiB. Both share
the unchanged128-byte execution charge and existing local aggregate policy
(default512KiB/group, fixed2MiB/process). The conservative logical inventory is
6,216 bytes, including records/cells/closures, retained UTF-16 public strings,
terminal projections, queue/async/return frames, adapter references and point/hex
construction. A fixed130-byte ASCII encoder removes65 boxed-number/string arrays.
This inventory bounds application-owned logical storage, not opaque WebCrypto,
JavaScript engine/GC or framework serialization memory.

Focused evidence after review corrections:88 tests pass across admitted-key,
Existing selected-key and generic crypto files; the funded ABI51 refusal passes.
Targeted strict TypeScript passes. Review found and corrected malformed admission
accessors being read as negative proof, a shadowable encoder length, and the
adapter-module registry being lost during HMR. Closed
native projections now use own data snapshots; the encoder uses the intrinsic
backing width and literal65 bound. Integrated frontend/native/full gates remain
with the primary integration owner. No full suites or Rust commands were run for
this isolated browser leaf.
