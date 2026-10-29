# Admitted neutral Root preparation

29 October 2026. This factory is drafted and **uncompiled/unrun**. V2 remains off.
It supplies inert Root capacity, not an operational agreement or Ready result.

`prepare_admitted_voice_admission_step` checks the actual selected NativeActive
Root residence and original grant, current actor/epoch/policy, published Presence,
and the original RPC payment. Its input contains no peer or role. A separate
`AdmittedAdmissionSession` records the actual local DeviceId and unpredictable
public incarnation. It has no conversion into the Existing admission session or
HIGH-only preclaim.

Presence lazily creates one independently paid pool set: the existing 32 Action
positions and separate 32-position incoming/outgoing packet tables. Every
registration reuses those same pools. Registration and preclaim observers retain
the same outside pool payment after Presence or Root destruction. Raw pool Arcs
are not exposed by the public API. Pool and registration body allocations precede
their funding owners in destruction order. Final funding extraction uses
`Arc::into_inner`, so its own allocation returns before the credit does.

The neutral `ChainCapacity` reserves incoming6/outgoing6, twelve Action positions,
the existing bytes/lanes and one progression claim. Thus an otherwise empty call
has 26 incoming, 26 outgoing and 20 ordinary Action positions remaining. Both
Existing role consumers reject the Neutral layout before decomposition. No
packet, Action, deadline, signature, native ledger row or SDK work is created.

After Root creates a registration, the result returns that owner even if the
later capacity reservation is refused. Native must stage it and retry with its
exact observer. Passing `None` requests a new registration; this factory alone
does not establish canonical native generation or recover a lost first reply.
Generation checks are nonzero and monotone cancellation-floor checks only.
Cancellation closes this admission registration without ending the call.

NativeActive remains an independent predicate: an old live bit cannot reopen
admission during the first phase-negative interval. Epoch/End closure invalidates
currency immediately but does not return still-owned capacity or pool backing.
All new prepayments have production inventory guards before allocation: pools
2 KiB and registration3 KiB. These are candidate allowances until compiled
measurements and focused tests pass; the formulas must not be reduced to fit.

The draft regressions cover actual Root preparation, full ordinary-pool refusal
and exact registration/RNG recovery, foreign observers/payments, cancellation,
phase-first closure, an actual MLS Add, neutral position accounting, both legacy
role refusals, partial packet reservation rollback, final body destruction and
concurrent final funding return. Formatting has been checked separately from
execution. Complete integration checks and adversarial actual-diff review remain
open.

Still required: the Actor/native admitted Slot, sticky local operation31,
canonical native generation ownership, Availability/Request/Grant/Commit,
immutable RoundGate and native parent-row consumption, admitted KDF and tags8/9,
Ready and RTC/media composition. Future role binding must pin the exact peer and
agreement before trimming pristine surplus positions. A retryable workflow-install
refusal must retain its same bound candidate; it must not reuse the older consuming
`into_chain` failure path. Future bound/workflow/media owners must carry the SAME
pool funding through their last real slot and allocation return.
