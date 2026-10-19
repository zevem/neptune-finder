#!/usr/bin/env python3
"""Encode native Finder icons from user-supplied artwork without editing sources.

Requires Pillow. The regular Finder icon supplies Linux PNGs and Windows ICO;
the macOS-specific artwork supplies ICNS. Original assets/branding PNG bytes
are never written. Only Finder-owned manifest hashes are updated.
"""

import hashlib
import json
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
BRANDING = ROOT / "assets/branding"
ICONS = ROOT / "assets/icons"


def digest(path):
    return {"path": path.relative_to(ROOT).as_posix(), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def main():
    sources = [BRANDING / name for name in (
        "neptune-finder-icon.png", "neptune-finder-logo.png", "neptune-finder-macos-logo.png",
    )]
    before = {path: hashlib.sha256(path.read_bytes()).hexdigest() for path in sources}
    generated = []
    with Image.open(sources[0]) as source:
        if source.width != source.height:
            raise ValueError("Finder icon must be square; source artwork has not been changed")
        regular = source.convert("RGBA")
        for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
            target = ICONS / f"neptune-finder-{size}.png"
            regular.resize((size, size), Image.Resampling.LANCZOS).save(target, format="PNG")
            generated.append(target)
        target = ICONS / "neptune-finder.ico"
        regular.save(target, format="ICO", sizes=[(size, size) for size in (16, 24, 32, 48, 64, 128, 256)])
        generated.append(target)
    with Image.open(sources[2]) as source:
        if source.width != source.height:
            raise ValueError("macOS Finder icon must be square; source artwork has not been changed")
        target = ICONS / "neptune-finder.icns"
        source.convert("RGBA").save(target, format="ICNS")
        generated.append(target)
    for path, sha256 in before.items():
        if hashlib.sha256(path.read_bytes()).hexdigest() != sha256:
            raise RuntimeError(f"Original artwork changed unexpectedly: {path}")
    manifest_path = ROOT / "assets/neptune-source.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["material"] = "Exact Terminal-shared fonts, Tabler SVGs, license and generator; Finder's extended Tabler subset and user branding are recorded separately"
    manifest["files"] = [entry for entry in manifest["files"] if entry["path"].startswith(("assets/fonts/", "assets/icons/tabler/"))]
    manifest["finder_branding"] = {
        "policy": "Finder-owned user artwork; intentionally distinct from Neptune Terminal. Sources preserved byte-for-byte; derived files only resize/encode those pixels.",
        "sources": [digest(path) for path in sources],
        "derived": [digest(path) for path in generated],
        "regular_source": sources[0].relative_to(ROOT).as_posix(),
        "macos_source": sources[2].relative_to(ROOT).as_posix(),
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"Encoded {len(generated)} Finder native icon files; all {len(sources)} user source PNGs unchanged.")


if __name__ == "__main__":
    main()
