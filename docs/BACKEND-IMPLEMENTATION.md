# Creative Suite backend implementation checklist

This is an acceptance checklist, not a count of source files. UI layout, components and the
canonical Flipnote mockups remain user-owned. No item is complete merely because its core
helper exists. Scope is the Creative Suite backend, including P1, unless the user narrows it.

## Current evidence

P1 is approximately **65% complete, with a ten-percentage-point uncertainty**. This estimate
does not describe the whole Creative Suite. Initial baseline: `cce0528` on `Create-suite-2`;
HANDOVER records subsequent verified slices and their test/review evidence.
The protocol/store foundations have substantially more coverage than the runtime integration.

## P1 acceptance milestones

- [x] Signed domain operations, epoch gates, close candidates, owner receipts and fault model.
- [x] Deterministic registry checkpoint materialization and projection preflight.
- [x] Vault-sealed registry, intent, receipt and recovery records, bounded inventory and reserves.
- [x] Recovery-first durable settlement and restart paths; included-only intent retirement.
- [x] Cooperative durable-intent replay and driver-acknowledged one-shot publication.
- [x] Opt-in authenticated registry gossip with durable receive admission.
- [x] Bounded durable registry operation pages with provider-local authenticated cursors.
- [x] Authenticated registry page request/response with bounded cooperative source serving.
- [x] Bounded receiver continuation that saves complete pages before advancing; joined-member
      divergence, duplicate, cancellation, rotation and uncertain-storage regressions.
- [ ] Keyed receipt-head and expected-seed discovery, including a newcomer after rotation.
      Keyed authenticated registry head queries and checked durable owner-selection proofs are
      implemented cooperatively; expected-seed fetch/recovery-first newcomer install remain.
      The installer must capture/recheck runtime, MLS and discovery authority; raw mutable
      `ReceiptHeadAnswer.proof: Some` is not an admission permit or proof the seed is available.
      Independently observed owner-tenure evidence is saved with MLS; unknown tenure must
      not authorize a fresh head proof by copying a restored receipt's claimed tenure. The
      proof publisher uses explicit local snapshot preparation and source/journal barriers. Legacy or
      newly joined owners may stay Unknown; do not substitute the current MLS epoch.
- [ ] Runtime ownership, scheduling, cancellation, vault lifecycle and complete storage accounting.
      Before automatic scheduling, measure maximum-epoch page-source rebuild time and set the
      cooperative work budget from that evidence; fixed memory/rate caps alone do not prove latency.
- [ ] Owner receipt issuance, succession, fault/repair and settlement driven end to end.
- [ ] Recovery listing, Restore/Copy/Export actions and settlement events over the actor/bridge.
- [ ] Multi-peer, restart, partition, capacity and owner-offline acceptance scenarios through the
      production adapters rather than direct calls to protocol helpers.

## Creative backend acceptance milestones

- [ ] C0 foundations: finish publication/retention and authoritative local avatar state;
      full-identity signalling and shared data-channel admission. Existing bounded PIX
      publication/fetch commands are implemented; the whole foundation is not yet complete.
- [ ] StudioIndex/StudioObject domain validators, projections, caps and P1 adapters.
- [ ] Studio actor/native commands, registry discovery and typed update/settlement events.
- [ ] Score and flipnote export, patch-union validation and referenced-blob enumeration.
- [ ] Announcement replies and chat doodle persistence/attachments.
- [ ] Non-visual draw/claim/replay, emoji-sound, ring and play protocol/state-machine contracts.
- [ ] Remaining backend/media support explicitly required by the creative design, with UI-only
      work separated from protocol/codec/export work when each slice is audited.
- [ ] Backend acceptance tests for the frontend's ten dependencies and documented honest limits.

## Completion and delivery rule

Each code slice gets focused regressions, a read-only adversarial review of the actual diff,
resolution of blocker/high findings, and all checks required by AGENTS.md. Verified slices are
committed and periodically pushed to the existing feature branch without force-pushing.
Unrelated work is preserved. This checklist and HANDOVER record actual progress and gaps.

The final UI implementation guide will list real commands, schemas, events, lifecycle/recovery
rules, limits and examples. It will distinguish production-ready backend paths from UI work
still to be built. Remaining material product/security choices require the user's direction;
they are not filled in by claiming a narrower definition of “100%”.
