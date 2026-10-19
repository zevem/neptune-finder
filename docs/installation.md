# Installation

Neptune Finder currently builds from source. Published installers and signed
updates are release gates, not available downloads. A native graphics stack and
a graphical desktop are required.

## Build and run

Install Rust through rustup; `rust-toolchain.toml` selects Rust 1.97.1 with
rustfmt and Clippy. Keep `Cargo.lock` and use `--locked`.

On Debian/Ubuntu, install the native window dependencies:

```sh
sudo apt-get install pkg-config libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
cargo run --release --locked --bin neptune-finder
```

On macOS, install Xcode Command Line Tools and rustup, then use the same Cargo
command. On Windows, install rustup with the MSVC toolchain and Visual Studio
Build Tools with the C++ desktop workload. Build with:

```sh
cargo build --release --locked --bin neptune-finder
```

The executable is `target/release/neptune-finder` on Linux/macOS or
`target/release/neptune-finder.exe` on Windows. Native runtime verification on
all supported platforms remains required before distribution; a successful
cross-platform build does not establish it.

## Isolated launch and command line

```sh
neptune-finder --path /some/folder
neptune-finder --data-root /tmp/neptune-finder-review --no-restore
neptune-finder --theme light --size 1100x740
neptune-finder --help
```

`--config PATH` chooses an explicit preferences file. It does not replace
`--data-root` for isolation. `--screenshot PATH` is a native verification path;
see [development](development.md). `--benchmark DIR` measures directory listing
without creating a GUI; see [performance](performance.md).

Native launcher staging and platform packaging preparation are documented in
[packaging/README.md](../packaging/README.md). No default file-manager
association is changed automatically.
