import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import capture_hc_bootstrap


@unittest.skipUnless(
    sys.platform == "linux" and os.getuid() != 0,
    "requires a non-root Linux test user",
)
class InitializationCleanup(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="hubcap-unit-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.scratch = self.root / "scratch"
        self.scratch.mkdir()
        self.hc = self.root / "fake-hc"
        self.hc.write_text("#!/bin/sh\nexit 23\n", encoding="utf-8")
        self.hc.chmod(0o700)

    def run_capture(self):
        with patch.object(sys, "argv", ["capture", str(self.hc)]):
            with patch.object(
                capture_hc_bootstrap.tempfile, "gettempdir",
                return_value=str(self.scratch),
            ):
                capture_hc_bootstrap.main()

    def test_failed_version_command_removes_scratch_without_stopping_a_core(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_capture()
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_connector_write_failure_removes_scratch(self):
        with patch.object(Path, "write_text", side_effect=OSError("injected write failure")):
            with self.assertRaises(OSError):
                self.run_capture()
        self.assertEqual(list(self.scratch.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
