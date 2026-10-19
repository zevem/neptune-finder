#!/usr/bin/env python3
"""Lightweight Neptune Finder boundaries, adapted from Neptune Terminal's gate."""

from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys
import tempfile
import tomllib
import unittest

GUI_DEPENDENCIES = {
    "egui", "eframe", "epaint", "emath", "egui-wgpu", "egui-winit", "wgpu",
    "winit", "rfd", "arboard", "gtk", "gtk4", "tauri", "web-view", "wry",
}
CORE_CRATES = ("finder-core", "finder-operations")
# Rendering is a consumer of prepared snapshots; native I/O belongs to workers.
FILESYSTEM_CALLS = re.compile(
    r"\b(?:std\s*::\s*fs\b|fs\s*::\s*(?:read|read_dir|read_to_string|write|metadata|"
    r"symlink_metadata|canonicalize|rename|copy|remove_file|remove_dir|create_dir|"
    r"create_dir_all|File|OpenOptions)\b)"
)


def dependencies(document: dict) -> set[str]:
    names: set[str] = set()
    for section in ("dependencies", "build-dependencies", "dev-dependencies"):
        for alias, spec in document.get(section, {}).items():
            names.add(spec.get("package", alias) if isinstance(spec, dict) else alias)
    for target in document.get("target", {}).values():
        names.update(dependencies(target))
    return names


def check(root: Path) -> list[str]:
    errors: list[str] = []
    for name in CORE_CRATES:
        manifest = root / "crates" / name / "Cargo.toml"
        if not manifest.is_file():
            errors.append(f"Missing required core manifest: {manifest.relative_to(root)}")
            continue
        try:
            document = tomllib.loads(manifest.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            errors.append(f"{manifest.relative_to(root)}: {error}")
            continue
        forbidden = dependencies(document) & GUI_DEPENDENCIES
        if forbidden:
            errors.append(f"{name} must stay GUI-free; found: {', '.join(sorted(forbidden))}")
        # Crates may depend on each other, but never reach back into the desktop.
        if "neptune-finder" in dependencies(document):
            errors.append(f"{name} must not depend on the desktop package")
    for source in sorted((root / "src" / "ui").rglob("*.rs")):
        try:
            text = source.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            errors.append(f"{source.relative_to(root)}: {error}")
            continue
        # Strip ordinary comments to avoid documentation examples being code.
        text = re.sub(r"/\*.*?\*/|//[^\n]*", "", text, flags=re.DOTALL)
        if FILESYSTEM_CALLS.search(text):
            errors.append(f"{source.relative_to(root)}: filesystem I/O must run outside UI rendering")
    return errors


class BoundaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="neptune-boundaries-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for name in CORE_CRATES:
            folder = self.root / "crates" / name
            folder.mkdir(parents=True)
            (folder / "Cargo.toml").write_text('[package]\nname = "' + name + '"\n', encoding="utf-8")
        (self.root / "src/ui").mkdir(parents=True)

    def test_gui_free_crates_and_prepared_ui_are_allowed(self) -> None:
        (self.root / "src/ui/list.rs").write_text("// std::fs::read_dir belongs on a worker\nfn paint() {}\n")
        self.assertEqual(check(self.root), [])

    def test_renamed_platform_specific_gui_dependency_is_rejected(self) -> None:
        manifest = self.root / "crates/finder-core/Cargo.toml"
        with manifest.open("a", encoding="utf-8") as stream:
            stream.write('\n[target.\'cfg(windows)\'.dependencies]\nrenderer = { package = "egui", version = "0.36" }\n')
        self.assertTrue(any("egui" in error for error in check(self.root)))

    def test_ui_filesystem_access_is_rejected(self) -> None:
        (self.root / "src/ui/list.rs").write_text('fn paint() { std::fs::read_dir("."); }\n')
        self.assertTrue(any("outside UI" in error for error in check(self.root)))

    def test_missing_crate_is_a_failure(self) -> None:
        (self.root / "crates/finder-core/Cargo.toml").unlink()
        self.assertTrue(any("Missing required" in error for error in check(self.root)))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    if arguments.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(BoundaryTests)
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    errors = check(arguments.root.resolve())
    for error in errors:
        print(error, file=sys.stderr)
    if errors:
        return 1
    print("Architecture boundaries passed: GUI-free core/operations and I/O-free rendering.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
