# Owner-return scheduling fixture isolation

The PR29 Windows CI log for job `108646903914` reports all three owner-return
variants failing the unchanged 160-pass / 40,000-ms installation assertion. The
Linux review also reports a third preview never becoming ready. These failures
do not justify weakening the authoritative Studio/Registry installation checks.

The scheduling fixture, preview parser and Registry retention code are unchanged
between `48f9069` and the reviewed `d8f72ce`. Historical failures at the same
boundary are recorded in `GATE4-AGENT-1-STATUS.md`; historical isolated passes
are not evidence that current parallel CI passes.

The fixture had one independent `ManualClock` per test but shared production's
four-slot preparation pool and three-slot preview parser pool with every other
test in the executable. A retained Registry source releases its permit against
its own clock; a cancelled parser keeps its permit until its actual worker exits.
Advancing another fixture's clock cannot release either resource. Moreover,
`wait_studio_preparation` only waits for a detached local preparation: a refused
reservation leaves that flag false. The test could consume its entire simulated
deadline while unrelated test work retained the real permits.

Each owner-return scenario now configures one test resource domain before the
first Studio custody visit. Both spawned actors share exactly four general and
three preview parser permits. Source capture, Registry inventory/service,
overlay work and preview parsing all use that shared domain. Production still
uses the original process-wide pools; configuration commands exist only under
`cfg(test)`. No deadline, retry, retention, worker cancellation or authority rule
changes. The raw hint provider performs no local source preparation.

The existing three cases retain all seed custodians, keep zero preview capacity,
restore only owner transport reachability and demand both exact durable classes
within the original budget. The cancelled-parser case additionally checks that
its real blocked worker keeps both kinds of preparation permit after cancellation.
A shared-capacity regression checks cross-actor refusal and release through the
existing reservation/custody implementation.

The opt-in saturation regression holds all production preparation permits while
running the actual owner-return scenario against its simulated-process domain.
It must run alone: draining globals in the normal parallel suite would itself
interfere with unrelated tests. To establish the counterfactual, temporarily
replace only the test actor configuration dispatch with `drop((shared, preview))`.
The saturation regression must then fail at `preview 0 never became ready`, not
at fixture setup, compilation or a timeout. Restore the source byte-for-byte and
run it again. This proves the cross-fixture mechanism, rather than attributing
every historical scheduling failure to it without evidence.

Required checks, owned by the root agent; not executed in this implementation lane:

```text
cargo test -p catcoms-app --lib actor_preparation_classes_share_capacity_and_cancelled_custody
cargo test -p catcoms-app --lib studio_actor_owner_return_installs_both_classes -- --nocapture --test-threads=3
cargo test -p catcoms-app --lib studio_actor_owner_return_survives_unrelated_process_pool_contention -- --ignored --nocapture --test-threads=1
cargo test -p catcoms-app --lib studio_exchange::tests:: -- --test-threads=4
```

There remains a wall-clock handoff in the fixture: after waiting for known local
CPU work, it gives detached transport completions five milliseconds before
advancing simulated time. Pool isolation does not prove that handoff sufficient
under arbitrary CPU starvation. The parallel actor/suite runs must establish
whether a separate same-scenario scheduling failure remains; this patch makes
no broader Gate 4 acceptance claim.
