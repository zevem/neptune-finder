"""Regression coverage for capture isolation, process failures and bounded cleanup."""

import argparse
import importlib.util
from pathlib import Path
import os
import struct
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("native_harness", Path(__file__).with_name("native-harness.py"))
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)


@unittest.skipUnless(os.name == "posix", "Fake executable uses a POSIX shebang; native CI capture runs on Linux")
class NativeHarnessTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="finder-harness-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def arguments(self, body, timeout=3):
        app = self.root / "fake-app"
        app.write_text("#!/usr/bin/env python3\n" + body, encoding="utf-8")
        app.chmod(0o700)
        return argparse.Namespace(app=app, output=self.root / "artifacts", path=None, theme="graphite", size="1000x700", timeout=timeout)

    def test_success_uses_isolated_storage_and_records_native_dimensions(self):
        arguments = self.arguments('''import pathlib, struct, sys
args = sys.argv
storage = pathlib.Path(args[args.index("--data-root") + 1])
assert storage.name == "data"
assert "--no-restore" in args
folder = pathlib.Path(args[args.index("--path") + 1])
assert (folder / "Welcome.md").is_file()
png = pathlib.Path(args[args.index("--screenshot") + 1])
png.write_bytes(b"\\x89PNG\\r\\n\\x1a\\n" + struct.pack(">I", 13) + b"IHDR" + struct.pack(">II", 1000, 700) + bytes(100))
''')
        report = HARNESS.capture(arguments)
        self.assertEqual(report["pixels"], [1000, 700])
        self.assertTrue(report["isolated_storage"])
        self.assertTrue((arguments.output / "report.json").is_file())
        self.assertFalse((self.root / "data").exists())

    def test_failed_native_process_is_not_reported_as_a_capture(self):
        arguments = self.arguments("raise SystemExit(7)\n")
        with self.assertRaisesRegex(RuntimeError, "exited with 7"):
            HARNESS.capture(arguments)
        self.assertFalse((arguments.output / "report.json").exists())

    def test_timeout_terminates_owned_process(self):
        arguments = self.arguments("import time\ntime.sleep(60)\n", timeout=0.05)
        with self.assertRaisesRegex(RuntimeError, "timed out"):
            HARNESS.capture(arguments)

    def test_non_png_output_is_rejected(self):
        arguments = self.arguments('''import pathlib, sys
args = sys.argv
pathlib.Path(args[args.index("--screenshot") + 1]).write_bytes(bytes(200))
''')
        with self.assertRaisesRegex(ValueError, "not a PNG"):
            HARNESS.capture(arguments)


if __name__ == "__main__":
    unittest.main()
