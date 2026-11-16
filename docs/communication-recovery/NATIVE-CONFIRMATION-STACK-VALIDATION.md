# Native confirmation fixture stack correction

Linux CI at `6f8c7c6f` aborts in
`native_v2_abandoned_custody_needs_exact_physical_collection_dead_lease_and_root_drain`.
The actual TCP confirmation wrapper's retained future measures93,936 bytes locally before
the fix. Raising the thread stack would hide the nested composition problem.

Three synchronous constructors now allocate their complete original async bodies on the
heap before enclosing futures embed them: the two-endpoint custody cycle, the mode-specific
confirmation round and the six-client setup. The real endpoint/parent/barrier owners, every
cleanup branch, original polling, timeouts and final teardown/assertions remain unchanged.
The two actual custody children are also measured before their original `join!`.

Focused validation on the default Windows stack passes all three tests in21.97s:

- the actual caller-future size regression, which failed before the correction;
- the exact abandoned-custody scenario reported by Linux CI;
- the existing closing-fence/storage-copy/SDK-debt scenario.

Strict native all-target/all-feature Clippy passes in2m41s after building the unchanged
frontend bundle in the integration worktree. Root/native formatting and whitespace pass.
Actual-diff adversarial review finds no change to production behavior, ownership or test
assertions. These checks were run in the current Chat V2 integration worktree, alongside
the separately reviewed Core/Sync work. This is a fixture-only correction, not voice V2
completion, fresh Linux execution or a green full native suite. Full validation remains open.
