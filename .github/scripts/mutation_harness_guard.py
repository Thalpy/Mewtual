"""Shared source-preservation guard for mutation-test harnesses.

Mutation harnesses deliberately replace production source for one test run.  Their preflight
snapshots can become stale while an earlier mutant compiles, especially in a shared worktree.  A
later entry must therefore compare the live bytes immediately before installing its mutant; using
the old snapshot without this check could erase another developer's edit and then "restore" the
already stale content.
"""

from pathlib import Path


def write_if_unchanged(source: Path, expected: bytes, replacement: bytes) -> None:
    """Write ``replacement`` only while ``source`` still matches ``expected`` byte-for-byte."""

    if source.read_bytes() != expected:
        raise AssertionError(f"source changed concurrently: {source}")
    source.write_bytes(replacement)
