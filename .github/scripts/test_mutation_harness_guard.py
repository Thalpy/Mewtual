"""Focused regressions for mutation harness source preservation."""

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from mutation_harness_guard import write_if_unchanged


class MutationHarnessGuardTests(unittest.TestCase):
    def test_writes_mutant_when_preflight_snapshot_is_current(self) -> None:
        with TemporaryDirectory() as directory:
            source = Path(directory) / "source.rs"
            source.write_bytes(b"original")

            write_if_unchanged(source, b"original", b"mutated")

            self.assertEqual(source.read_bytes(), b"mutated")

    def test_preserves_edit_made_after_preflight(self) -> None:
        with TemporaryDirectory() as directory:
            source = Path(directory) / "source.rs"
            source.write_bytes(b"original")
            original = source.read_bytes()
            source.write_bytes(b"another agent's edit")

            with self.assertRaisesRegex(AssertionError, "source changed concurrently"):
                write_if_unchanged(source, original, b"mutated")

            self.assertEqual(source.read_bytes(), b"another agent's edit")


if __name__ == "__main__":
    unittest.main()
