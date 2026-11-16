"""Real-child controls for the extracted security runner; no Cargo required."""
import re
import runpy
from pathlib import Path
import sys
import tempfile
import unittest

GUARD = runpy.run_path(str(Path(__file__).with_name("security_test_guard.py")))


class GuardTests(unittest.TestCase):
    def test_exact_result_rejections(self):
        GUARD["self_test"]()

    def test_streamed_result_and_markers_are_bounded(self):
        with tempfile.TemporaryDirectory() as temp:
            code, summaries, expired, markers = GUARD["run_logged"](
                "bounded", [sys.executable, "-c", "print('mark 7\\n' * 10); print('test result: ok. 1 passed; 0 failed; 0 ignored;')"],
                Path(temp), limit=15, markers=(re.compile(r"mark (\d+)"),))
            self.assertFalse(expired)
            self.assertEqual(markers, [[("7",), ("7",)]])
            GUARD["validate"]("".join(summaries), code, 1)
            self.assertTrue((Path(temp) / "bounded.log").is_file())

    def test_failed_child_cannot_supply_success(self):
        with tempfile.TemporaryDirectory() as temp:
            code, summaries, expired, _ = GUARD["run_logged"](
                "failed", [sys.executable, "-c", "print('test result: ok. 1 passed; 0 failed; 0 ignored;'); raise SystemExit(9)"],
                Path(temp), limit=15)
            self.assertFalse(expired)
            with self.assertRaises(RuntimeError):
                GUARD["validate"]("".join(summaries), code, 1)

    def test_deadline_retires_owned_child(self):
        with tempfile.TemporaryDirectory() as temp:
            code, summaries, expired, _ = GUARD["run_logged"](
                "deadline", [sys.executable, "-c", "import time; time.sleep(60)"], Path(temp), limit=1)
            self.assertTrue(expired)
            self.assertNotEqual(code, 0)
            self.assertEqual(summaries, [])


if __name__ == "__main__":
    unittest.main()
