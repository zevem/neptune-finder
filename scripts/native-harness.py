#!/usr/bin/env python3
"""Capture Neptune Finder's real native framebuffer using isolated test storage."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import time

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def png_dimensions(path: Path) -> tuple[int, int]:
    with path.open("rb") as stream:
        header = stream.read(24)
    if len(header) != 24 or header[:8] != PNG_SIGNATURE or header[12:16] != b"IHDR":
        raise ValueError("Native screenshot is not a PNG with an IHDR header")
    width, height = struct.unpack(">II", header[16:24])
    if width < 100 or height < 100 or path.stat().st_size < 100:
        raise ValueError("Native screenshot is empty or unexpectedly small")
    return width, height


def make_fixture(root: Path) -> Path:
    folder = root / "Review folder"
    folder.mkdir()
    for name in ("Design", "Documents", "Images", "Projects", "Downloads"):
        (folder / name).mkdir()
    (folder / "Welcome.md").write_text(
        "# Neptune Finder\n\nA native file explorer in the Neptune ecosystem.\n",
        encoding="utf-8",
    )
    (folder / "Project notes.txt").write_text("Review list, grid, navigation and Quick Look.\n", encoding="utf-8")
    (folder / ".hidden-example").write_text("Hidden-file review fixture.\n", encoding="utf-8")
    (folder / "Cargo.toml").write_text('[package]\nname = "review-fixture"\nversion = "0.1.0"\n', encoding="utf-8")
    return folder


def capture(arguments: argparse.Namespace) -> dict:
    app = arguments.app.expanduser().resolve()
    if not app.is_file():
        raise FileNotFoundError(f"Build the native executable first: {app}")
    output = arguments.output.expanduser().resolve()
    output.mkdir(parents=True, exist_ok=True)
    # Each invocation owns its storage and process. No user state is reused.
    with tempfile.TemporaryDirectory(prefix="neptune-finder-native-") as temporary:
        storage = Path(temporary)
        folder = arguments.path.expanduser().resolve() if arguments.path else make_fixture(storage)
        screenshot = output / "native.png"
        if screenshot.exists():
            screenshot.unlink()
        command = [
            str(app), "--data-root", str(storage / "data"), "--no-restore",
            "--path", str(folder), "--theme", arguments.theme,
            "--size", arguments.size, "--screenshot", str(screenshot),
        ]
        started = time.monotonic()
        with (output / "app.log").open("w", encoding="utf-8") as log:
            process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
            try:
                returncode = process.wait(timeout=arguments.timeout)
            except subprocess.TimeoutExpired as error:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
                raise RuntimeError(f"Owned native process timed out after {arguments.timeout}s; see {output / 'app.log'}") from error
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
        if returncode != 0:
            raise RuntimeError(f"Native app exited with {returncode}; see {output / 'app.log'}")
        if not screenshot.is_file():
            raise RuntimeError(f"Native app exited without a screenshot; see {output / 'app.log'}")
        width, height = png_dimensions(screenshot)
        report = {
            "app": str(app), "platform": sys.platform,
            "theme": arguments.theme, "requested_size": arguments.size,
            "screenshot": str(screenshot), "pixels": [width, height],
            "sha256": hashlib.sha256(screenshot.read_bytes()).hexdigest(),
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "exit_code": returncode, "isolated_storage": True,
            "display_protocol": "wayland" if os.environ.get("WAYLAND_DISPLAY") else "x11" if os.environ.get("DISPLAY") else "native",
            "scope": "Native framebuffer capture; inspect visually. OS input/services require separate native acceptance.",
        }
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, default=Path("target/debug/neptune-finder.exe" if os.name == "nt" else "target/debug/neptune-finder"))
    parser.add_argument("--output", type=Path, default=Path("artifacts/native"))
    parser.add_argument("--path", type=Path, help="Read-only capture of this directory instead of an isolated fixture")
    parser.add_argument("--theme", choices=("graphite", "dusk", "light"), default="graphite")
    parser.add_argument("--size", default="1180x780")
    parser.add_argument("--timeout", type=float, default=45.0)
    arguments = parser.parse_args()
    if arguments.timeout <= 0:
        parser.error("--timeout must be positive")
    try:
        report = capture(arguments)
    except (OSError, ValueError, RuntimeError) as error:
        print(error, file=sys.stderr)
        return 1
    print(f"Native capture: {report['screenshot']} ({report['pixels'][0]}×{report['pixels'][1]})")
    print(f"Report: {arguments.output.resolve() / 'report.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
