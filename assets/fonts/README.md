# Bundled fonts

These are unmodified static TrueType fonts from their official repositories.
Their full SIL Open Font License 1.1 notices are kept beside the font files.

| File | Source | SHA-256 |
| --- | --- | --- |
| `Geist-Regular.ttf` | [Geist](https://github.com/vercel/geist-font/blob/main/fonts/Geist/ttf/Geist-Regular.ttf) | `85a1c6b18a6b0a06dfe9fd4f6d6a5d4979f74ec861eaef4bc7868b5492b8a117` |
| `Geist-Medium.ttf` | [Geist Medium](https://github.com/vercel/geist-font/blob/main/fonts/Geist/ttf/Geist-Medium.ttf) | `3a3b36f0d0b981f4857f7f00eeef4a5ee123605575d362ab31ee7c19e3d11f2f` |
| `Geist-SemiBold.ttf` | [Geist SemiBold](https://github.com/vercel/geist-font/blob/main/fonts/Geist/ttf/Geist-SemiBold.ttf) | `b9f52322d6d20a7ff1e7fe1310ff12ec851aebb83d24f8b4f1d4149298f489b2` |
| `JetBrainsMono-Regular.ttf` | [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono/blob/master/fonts/ttf/JetBrainsMono-Regular.ttf) | `e6fd0d7e91550b3ed2b735d4312474362c4716edc4fc0577a0f61ed782d5aed1` |
| `JetBrainsMono-Bold.ttf` | [JetBrains Mono Bold](https://github.com/JetBrains/JetBrainsMono/blob/master/fonts/ttf/JetBrainsMono-Bold.ttf) | `d22c4f3821d725eb01210d278d95dfcfcaadc34699a06658d47c8a5cc5830ada` |

Geist is used for interface text; JetBrains Mono is used for code and monospaced
preview text. Geist ships in regular, medium and semibold weights for interface
hierarchy; JetBrains Mono includes regular and bold. Shipping these files gives
consistent text metrics across supported operating systems without depending on
installed fonts.

The bundled egui fallback families remain available after these primary fonts.
Finder installs only bundled fonts at startup; it performs no system font scan
or font-file I/O in an interactive frame.
