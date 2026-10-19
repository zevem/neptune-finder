# Neptune ecosystem provenance

The source of Finder's visual system and engineering workflow is the sibling
Neptune Terminal repository at `../neptune`, as requested for consistency across
the ecosystem. The application uses the same Rust toolchain and native
`eframe`/`egui`/`wgpu` stack. A shared UI crate can replace these copies later.

| Material | Treatment |
| --- | --- |
| `.agents/skills/`, `.claude/skills/`, `skills-lock.json`, skill licenses | Copied verbatim, preserving relative symlinks and upstream attribution |
| `rust-toolchain.toml`, `.cargo/config.toml`, `LICENSE` | Copied verbatim |
| Geist and JetBrains Mono TrueType files and license notices | Copied verbatim |
| Tabler SVGs, TrueType subsets, mappings, generator and MIT notice | Copied verbatim |
| Finder branding artwork | User-provided Finder-specific PNGs preserved byte-for-byte; platform icon formats are encoded from these images |
| `src/theme.rs` | Exact original Graphite/Dusk/Light palettes, nine accents, geometry and egui defaults; terminal imported-theme configuration removed |
| `src/icons.rs` | Exact original 79-variant glyph vocabulary, registration, renderer and icon buttons; terminal-provider integration removed |
| `src/controls.rs` | Exact original production buttons, toggles, segmented controls, fields, grouped settings, menus and sheets |
| `src/fonts.rs` | Same bundled fonts, weight family names and fallback order; terminal font discovery omitted |
| `AGENTS.md`, local guides, docs | Engineering constraints retained; terminal/PTY/model concepts replaced by filesystem/operation/worker contracts |
| `.github/workflows/ci.yml` | Pinned actions, compiler, profiles and validation policy retained; Finder targets replace Terminal/website/helper targets |
| Native capture and architecture scripts | Equivalent isolation and ownership rules, adapted to Finder's CLI and crate boundaries |

Terminal's unrelated browser, subscriptions, agent orchestration, payment,
website and release identity/secret configuration do not belong in Finder.
Their runtime docs and release workflows are therefore not copied. Terminal's
native acceptance records do not establish Finder correctness. Finder's own
[release gates](verification.md) retain the same requirement for honest native,
platform and performance evidence.

Terminal-shared font/Tabler hashes and separate Finder-owned source/derived
branding hashes are recorded in
[assets/neptune-source.json](../assets/neptune-source.json). Finder artwork is
intentionally distinct from Terminal. The original font and Tabler manifests
retain official upstream source URLs and licenses.

`python3 scripts/export-icons.py` requires Pillow and produces only platform
icon encodings/sizes; it never writes the user-provided PNGs. macOS uses
`neptune-finder-macos-logo.png`, while Linux/Windows use `neptune-finder-icon.png`.
