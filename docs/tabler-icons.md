# Tabler icons

Neptune Finder uses Neptune Terminal's official Tabler Icons 3.49.0 SVGs, glyph
mapping and native renderer, extended with fourteen file-explorer glyphs from
the same pinned release (home, download, file kinds, media, devices, list,
open folder, pin, scissors and info). The source assets are
pinned to upstream commit `a4ce1404bc6d24d3c365afe7b258d6bf6f48d62d` with their
MIT notice, hashes and provenance retained in
[assets/icons/tabler](../assets/icons/tabler/README.md).

The outline font contains 89 glyphs (23,464 bytes); the filled font contains
four (1,360 bytes). They are registered in dedicated families, preserving the
text families and their fallbacks. A 28-point icon button surrounds its
16-point glyph, with the same hover, focus, press and tooltip behavior as
Terminal. Accessible names remain plain labels even when tooltips show shortcut
hints.

`icons::paint` preserves the original 24-point design grid and optical offsets.
It centres that cell from the glyph's baseline, not the laid-out text row: the
fonts declare a typographic line gap that egui adds to the row, which would
otherwise lift every glyph (by over a point at 150 %).
`paint_rotated` rotates about the same design-cell center. Raster sizes are
rounded to whole physical pixels, and fractional animated bounds scale cached
meshes to avoid continuously growing the font atlas. Icon rendering requires no
per-frame filesystem access, image decode or SVG parsing.

Keep source SVG paths, glyph outlines and mappings unchanged. Add a genuinely
missing icon by extending the official pinned subset and regenerating it with
the bundled generator; do not substitute a Unicode character, emoji or
hand-drawn imitation. Preserve the upstream license and source/hash records.

The full mapping includes terminal and GitHub vocabulary for ecosystem
consistency. Finder does not display terminal-provider branding or depend on
the Terminal model's agent types. The only renderer adaptation is removal of
that separate provider-mask integration.
