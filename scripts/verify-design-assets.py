#!/usr/bin/env python3
"""Verify exact Terminal fonts/Tabler, Finder's extended Tabler subset and Finder-owned artwork."""

import hashlib
import json
from pathlib import Path
import sys


def main():
    root = Path(__file__).resolve().parent.parent
    manifest = json.loads((root / "assets/neptune-source.json").read_text(encoding="utf-8"))
    failures = []
    for entry in manifest["files"]:
        path = root / entry["path"]
        if not path.is_file():
            failures.append(f"Missing design asset: {entry['path']}")
        elif hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
            failures.append(f"Design asset differs from Neptune Terminal: {entry['path']}")
    icons = manifest.get("finder_icons", {}).get("files", [])
    for entry in icons:
        path = root / entry["path"]
        if not path.is_file():
            failures.append(f"Missing Finder icon asset: {entry['path']}")
        elif hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
            failures.append(f"Finder icon asset changed; regenerate with generate.py and refresh the manifest: {entry['path']}")
    branding = manifest.get("finder_branding", {})
    for entry in branding.get("sources", []) + branding.get("derived", []):
        path = root / entry["path"]
        if not path.is_file():
            failures.append(f"Missing Finder artwork: {entry['path']}")
        elif hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
            failures.append(f"Finder artwork changed; preserve user artwork and refresh derived icons/manifest: {entry['path']}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"Verified {len(manifest['files'])} exact Terminal-shared font/Tabler assets, {len(icons)} Finder icon assets and {len(branding.get('sources', []))} Finder-owned source images with {len(branding.get('derived', []))} derived icon files.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
