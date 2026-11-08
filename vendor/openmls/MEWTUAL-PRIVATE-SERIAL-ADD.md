# Private serial Add computation

The separately named native ordinary-profile method
`CommitBuilder<LoadedPsks>::build_private_text_add_with_time` shares the existing
commit algorithm and full validation, while selecting serial execution at all
three native TreeKEM parallel iterator sites. Existing methods still use their
original wrappers and parallel execution. There is no global or thread-local
mode, new runtime pool, key extraction or Source grant.

Before entering the shared algorithm, the method requires exactly one Add,
forced path update, consuming policy with an actually empty proposal queue,
no PSKs/external commit or leaf-parameter changes, suite3, an operational group,
at most64 actual leaf slots with room/reuse, and a nonoverflowing next epoch.
Unmerged targets must be strictly ordered, nonblank and inside their parent's
subtree. This avoids both a blank-target resolution assertion and amplification
through unrelated subtrees. Unsupported preflight returns the new unit error
`CreateCommitError::UnsupportedPrivateTextAdd`, without diagnostic allocation.

The method is absent on wasm and unsupported structural draft feature sets.
With SDK backtrace, crypto-debug or content-debug enabled it refuses before
algorithm entry: those diagnostics are outside this paid/private profile. Legacy behavior
and errors are unchanged. The method cannot retroactively bound `load_psks` or
input construction; its actual caller must separately check and pay those cuts.

Seven real SDK regressions pass in0.31s after8.25s compilation of the final test
addition (the initial six compiled in14.65s). They exercise
stage/merge/Commit/Welcome and joiner channel-key agreement, subsequent ordinary
multi-Add building, six shape refusals preserving exact original rows, a genuine
full64-member tree, reuse after real removal, a signed queued proposal, and
an actual stored and loaded PSK with its SDK-signed proposal, and restored
malformed public-tree fixtures with blank/out-of-subtree unmerged
targets. The malformed inputs return the exact unit refusal before resolution.
The tests do not supply a production closed-graph, Source or fee admission.

The separate diagnostic-profile fixture has now passed actual builds for each
of backtrace, crypto-debug and content-debug (0.01s each after25.33s/16.31s/14.61s
compilation). It requires the immutable support predicate to be false, then checks
the otherwise valid one-Add shape returns the unit refusal with no pending state
or stored-row change. It is ignored in the ordinary cohort because that profile
must succeed; it is explicitly run with `--ignored` in the three diagnostic builds.
An isolated manifest points at the actual MLS sources and uses a separate test
lockfile for optional diagnostic dependencies, preserving production resolution.
This is no proof of early Source/payer admission; its consumer still needs tests.
The owning private candidate, full allocation/work invoice,
durable governance request/result, Source replacement and native activation are
separate incomplete consumers. Numeric execution layout is not a payment or the
complete SDK allocation bound. `docs/HANDOVER.md` records integration checks.
