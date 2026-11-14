# Native fixture observations, 14 November

These are acceptance-fixture corrections, not new media or text protocol
implementation. Chat V2 remains disabled and incomplete.

## Closed SDP observation

Linux run43589791580 passed1041 native tests and failed24. Its first real
failure was native_v2_confirmation_framed_result_uses_actual_actor_body_and_original_phase:
the closed-original Release observation returned authenticated Capacity[1,3]
instead of Closing[5,2]. Later failures were fixture-isolation cascades. Windows
passed that run. This failure was not dismissed as flaky.

The fixture now briefly holds the real native transition index, calls the real
original Release, drops the guard and requires Capacity. It then retries the
same authenticated Release body/proof within the unchanged original deadline.
The final closed-history read uses that same bounded helper. Release Capacity
may occur before or after monotone closure, so this is original negative cleanup
and history recovery, not a claim that every refusal precedes side effects.
The index is never held across framed ingress, whose parent resolver takes it
synchronously. Exact closing/drained, proof, live-buffer and SDK assertions stay.
This direct barrier establishes a native refusal; it does not itself force an
authenticated Release refusal over the framed path.

## Fresh media Current observation

Windows run43591075301 passed1034 native tests and failed40. Its first real
failure was native_browser_local_discovery_refusal_selects_other_media_peer_and_preserves_indexed_sibling,
at the media-current-all harness step. Two void handoff completions were treated
as proof of a new authenticated Current. A void completion can join an existing
flight or observe a refusal, so that implication is false.

The final all-peer step now uses the same bounded original-peer observer as the
other six recovery sites: at most32 observations under its existing five-second
harness bound, without resetting driver/protocol deadlines or minting another
admission. Only an increased authenticated Current counter for each exact
original peer passes. The first old-flight join and peer-liveness checks remain.
Four executable regressions extract the actual helper and final switch case.

The three Current/refusal regressions previously supplied generic three-byte
Capacity instead of native media's four-byte[9,2,2,7]. That input failed the
header length check before AEAD. They now supply the actual encrypted shape and
require a browser-decrypt increment while preserving the same Session, keys,
bootstrap, SDK peer, no-new-Admit and final fee-retirement assertions.

The CI trace proved a four-byte media refusal but did not record its exact code.
It did not establish that the refusal was Capacity or prove a production media
lifecycle defect. Diagnostics now append only validated authenticated scalar
media-error category/code, never parent, proof, key, SDP or plaintext.

## Wrong cleanup receipt under contention

The corrected H run43599886163 passed all Windows native tests, but Linux
passed1060 and failed5. Its first failure was the maximum-SDP scenario's wrong
Release receipt: it expected Unauthorized[1,3,5] immediately and observed
Capacity[1,3,7]. Four subsequent scenarios failed on the poisoned shared fixture.
The actual ledger acquires its index and row before validating the receipt.

The wrong-proof check now first forces the real direct-native index refusal,
then drops that guard before framed parent resolution. It retries only the
exact authenticated Capacity shape for the same opcode10 body, with at most32
observations inside one fixed five-second watchdog, and still requires exact
Unauthorized. The original claim remains unreleased throughout; the following
correct receipt must still release and recover the original history. This
cleanup observer does not renew a Claim or require it to remain unexpired.
Other exchanges, including intentional guard-held Capacity checks, are unchanged.

## Read-only SDK preflight under server-map contention

At3950ce59 Linux full native passed. Windows run43608286310 passed1052 and
failed13; its first failure was sdk.rs's direct preflight unwrap returning
Protocol(Capacity), before any physical hold or SDK preparation/allocation.
The later twelve failures came from the shared fixture's poisoned isolation.

The three identical read-only SDK preflights now share one narrowly bounded
observer. It drives the same future with the real server map held, requires
exact Capacity, releases that map, then requires success. Session counters and
SDK release state stay unchanged. Only preflight Capacity can repeat, at most32
observations within one fixed five-second watchdog including pressure and Actor
IO. Allocation, attachment, Release and actual custody/deadline checks stay
outside the wrapper. The exact original failing TCP scenario passed using the
identical fixture copied into the authoring worktree (7.44 seconds, after the
native rebuild). The updated H-head full CI remains pending; this copied-source
focused pass is not an independent H full run.

## Evidence and limits

Read-only actual-diff reviews of the earlier SDP and Current corrections are clear. The actual native
text lifecycle cohort passed43 cases, the exact framed SDP scenario passed1,
and345 focused TypeScript cases passed with both Current/refusal corrections.
The exact original native media TCP scenario also passed (70.47 seconds). Those
focused native executions used the identical changed fixture files copied into
the later native-UI worktree, not an independent H build. Their full H Windows
run passed; the Linux failure and new wrong-receipt correction are described
above. Its copied-source exact maximum-SDP regression passed (6.43 seconds),
and the full3950ce59 Linux native run passed; Windows failed later at the
read-only SDK preflight. Only the newest SDK preflight correction's focused
execution and updated-head full CI remain pending. Earlier full Root runs remain
subject to the one explicitly deferred slow catch-up cost case. Root strict
Clippy still has unfinished application-pipeline errors; no passing sign-off
or suppression is claimed.
