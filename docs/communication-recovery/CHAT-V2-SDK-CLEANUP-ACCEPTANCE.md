# Native SDK cleanup fixture

The Windows native job at runtime checkpoint `d4496bdd` failed its original SDK
Release assertion (expected `[6,3]`, observed protocol-refusal prefix `[1,3]`),
followed by 23 fixture-poison failures. The log omitted the third refusal byte;
it does not establish which protocol error occurred.

Static tracing found that fail-fast ledger contention can return Capacity before
any close/release effect. The fixture now deterministically holds the actual
parent row mutex, calls the exact release and requires Capacity with the same
original obligation still unreleased. It then observes only exact authenticated
Capacity responses again under one original runtime Clock watchdog (5 seconds,
at most 32 observations), preserving the same Session, body, generation and proof.
Other responses and outer errors remain terminal. Wrong-proof Unauthorized and
the complete terminal reply size/state/proof remain strict. Fresh SDK Admit,
allocation, attach and production behavior are unchanged.

The previously failing exact native case passed locally (1 case, 6.01 seconds)
on the successor worktree, including the deterministic contention check. Complete
native CI is still required. This is no physical media or browser audio evidence.
