# Native packaging preparation

These files adapt Neptune Terminal's native desktop integration for Finder's
binary and application identity. They are development staging instructions;
there are no published Finder installers, provisioned signing identities or
release automation here. See [release requirements](../docs/releases.md).

## Linux

Build the optimized native executable with the pinned compiler and lockfile:

```sh
cargo build -p neptune-finder --release --locked --bin neptune-finder
```

`neptune-finder.desktop` uses application ID `rs.neptune.finder`, the same ID as
the native window. It accepts a directory through the positional CLI path. It
advertises `inode/directory` support without changing the system's default file
manager.

To stage a per-user install manually, after reviewing the executable:

```sh
install -Dm755 target/release/neptune-finder "$HOME/.local/bin/neptune-finder"
install -Dm644 packaging/neptune-finder.desktop "$HOME/.local/share/applications/rs.neptune.finder.desktop"
install -Dm644 assets/icons/neptune-finder-256.png "$HOME/.local/share/icons/hicolor/256x256/apps/rs.neptune.finder.png"
```

The desktop environment must include `~/.local/bin` on its application PATH.
Validate the desktop entry with `desktop-file-validate` where available. These
commands install only the executable, launcher and icon; a distributable package
must also ship LICENSE, THIRD-PARTY-NOTICES, Tabler's MIT notice, the bundled font
licenses and dependency license notices. Test both X11 and Wayland on the actual
supported distribution before publishing a package. Building an AppImage or DEB
and supporting updates remain release work.

## Windows

Use a Windows host with Rust via rustup and the Visual Studio C++ desktop build
tools/MSVC SDK. From the repository root:

```powershell
cargo build -p neptune-finder --release --locked --bin neptune-finder
.\target\release\neptune-finder.exe --data-root "$env:TEMP\neptune-finder-review" --no-restore
```

For an explicit x64 target, install `x86_64-pc-windows-msvc` through rustup and
pass `--target x86_64-pc-windows-msvc`; the executable then lives under that
target's `release` directory. Native CI provides build/test coverage, while
installed-runtime acceptance must be performed on Windows.

The build script embeds the Finder-specific ICO and product identity in the Windows executable, matching Neptune Terminal's resource pipeline. GUI launches use the Windows subsystem.

A future per-user installer needs its own Finder product GUID, install directory,
shortcuts, uninstall entry and signing configuration. Do not copy Terminal's
product GUID or browser payload into Finder. The Finder-specific ICO is
available at `assets/icons/neptune-finder.ico`. Do not register Finder as the system's
default file manager without an explicit user choice.

Acceptance includes native open/reveal, Recycle Bin, directory/symlink handling,
sharing violations, Unicode/long/UNC paths, high-DPI, IME and keyboard/clipboard
behavior. Produce checksums and ship all applicable license notices. No signed
installer or verified Windows runtime is claimed by these instructions.

## macOS

Use a macOS host with Xcode Command Line Tools and Rust via rustup:

```sh
cargo build -p neptune-finder --release --locked --bin neptune-finder
./target/release/neptune-finder --data-root /tmp/neptune-finder-review --no-restore
```

The release executable is native to the host target. Apple Silicon and Intel
need their own native validation; a universal application must contain both
validated architectures. The Finder-specific ICNS is available at
`assets/icons/neptune-finder.icns`.

A distributable `.app` needs `Contents/MacOS/neptune-finder`, its icon under
`Contents/Resources`, and an Info.plist with Finder's own name, executable and
bundle identifier `rs.neptune.finder`. Version/build values must match the actual
candidate. Finder does not need Terminal's browser helper or browser entitlements.
Signing, notarization and DMG packaging need Finder-owned credentials and native
acceptance before distribution.

Review Finder/Trash integration, default-app opening, filesystem permissions,
resource-fork/extended-attribute copy limitations, Retina scaling, accessibility,
IME, keyboard conventions and installed launch behavior. Neither a Linux build
nor the CI configuration establishes these results.

## Artwork provenance

The user-provided Finder PNGs in `assets/branding/` are preserved byte-for-byte.
Linux/Windows icon derivatives use `neptune-finder-icon.png`; the ICNS uses
`neptune-finder-macos-logo.png`. Regenerate native encodings with
`python3 scripts/export-icons.py` (Pillow required). This never writes source
PNGs. `scripts/verify-design-assets.py` verifies Finder-owned artwork separately
from the exact shared Terminal fonts and Tabler assets.
