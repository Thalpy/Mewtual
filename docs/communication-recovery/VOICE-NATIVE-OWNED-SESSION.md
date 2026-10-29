# Native channel allocation custody prerequisite

29 October 2026. This is an inactive native ownership prerequisite, not an
operational V2 milestone. V2 remains off. The new code and Rust regressions are
**uncompiled and unrun** at this checkpoint. The primary agent owns coherent
integration, measurements and all full validation gates.

The admitted physical collection proposal needs evidence that the actual
Session and public-output allocations returned. A weak pointer's strong count
reaches zero before the last destructor finishes; it cannot provide that
evidence. Likewise, a credit stored inside an Arc or Box body can refund before
the enclosing allocation is freed.

The private `SessionRef`, `SessionWeak`, `OwnedArc`, `OwnedWeak` and `OwnedBox`
wrappers retain the same original payment outside the actual body/allocation
drop. Their dereference target is the body, never `Arc<T>`, and no production
API exposes the underlying raw Arc/Weak. The plain shared funding owner uses
`Arc::into_inner` on every alias, so the final allocation is deallocated before
the extracted credit is released, including concurrent final aliases.

Session strong owners retain its original control, bootstrap and browser-control
payments. Private weak Session witnesses retain the same original bootstrap
payment after the live body and control/browser payments return. A successful
weak upgrade takes the existing body's funding; it creates no new payment,
Session, key, identity, counter or permission. The process index likewise drops
its original Box before its original process credit. The Row and public output
retain their metadata payment outside each actual allocation.

An admitted closed Row keeps independent strong custodians for Session and
public output. A bounded sweep takes one custodian only when it is the sole
strong owner and has no weak owners. It marks that original owner `Returning`
under the Row lock, then destroys it outside Index, Row and Coordinator guards.
Only the final outside marker publishes `Returned`, after the actual body and
allocation are gone. A concurrent or reentrant collector must keep an in-flight
Row. Session and output can return independently: a retained public metadata
output does not hold Session Crypto capacity after the real Session owners
return, even while the overall call remains live. Final Row collection still
requires the original dead lease and every existing admitted-stage/Slot drain
predicate. An empty Existing Slot is not a substitute for future admitted debt.

Setup Ticket fields place original preparation before the last Row alias. The
allocation-free reaper runs before admitted-key Crypto admission and before its
UI/Coordinator locks; it does not invoke callbacks from SessionRef destruction.
The physical-history leaf must also run it before negative history observation.

No fee or aggregate cap was increased here. Existing production guards retain
their previous terms and add complete wrapper/funding/reaper terms. The original
2 KiB bootstrap, 10 KiB Row, 4 KiB index and 12 KiB control budgets still require
actual compilation and measurement. A failed guard is a refusal to investigate,
not permission to remove invoice terms. Identity, key material, wire bytes,
counters, executor/profile fences and the shared 64-row index remain unchanged.

## Focused regressions drafted

All eight new Rust regressions below are **unrun**:

- Metadata body destruction with a genuinely retained weak allocation.
- Plain funding deallocation before refund, with an actual return barrier.
- Concurrent final funding aliases with one extraction/refund.
- Final Session destructor paused while strong count is already zero and the
  real original payments remain held.
- Concurrent final Session aliases through the single actual body destructor.
- Admitted Session destructor paused while a second collector and final public
  output return cannot produce collection.
- Closed physical channel reaping while the whole call is live, with a retained
  output and full Crypto pressure; another group can progress after Session
  actually returns.
- Abrupt Registry destruction with independently retained native projection,
  output, header and weak allocation owners. This proves allocation custody only.

Scoped formatting and `git diff --check` passed. No Cargo, native, frontend or
full-suite result is claimed for this prerequisite. No browser runtime was
changed by this leaf.

## Explicit remaining limits

The Existing profile still uses its inherited weak-strong-zero retirement
classifier. Its weak allocations are now paid, but that classifier still does
not prove the final destructor returned. Admitted actual-return receipts must
never consume Existing `CollectedChannel` as equivalent evidence.

Abrupt destruction of the entire Registry still drops its inherited
`browser_setup` escrow with Row, without a real browser executor/SDK terminal
acknowledgement. The abrupt-destruction test does not certify SDK return.
Registry destruction or a missing Row must never mint an admitted collection
receipt. A real executor/window shutdown contract is required before activation;
this minimum leaf deliberately does not invent it.

The physical-intent/history leaf, its exact terminal receipt and deferred parent
ACK, the admitted Slot/31 consumer, operational bilateral admission, Ready,
SDP/RTC, media relay and physical acceptance are separate work. This ownership
change grants none of those permissions.
