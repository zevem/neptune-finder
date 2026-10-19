# Neptune Finder

<img src="assets/branding/neptune-finder-icon.png" alt="Neptune Finder logo" width="96" height="96">

A native Rust file explorer inspired by Ubuntu Files and macOS Finder, built for
the Neptune ecosystem. The interface uses Neptune Terminal's exact materials,
Geist typography, official Tabler icons and shared controls, rendered through
the same `egui`/`eframe` and `wgpu` stack.

Directory scans, recursive search, previews, filesystem operations and storage
run on workers. List and icon views render visible rows, and a settled window
sleeps between events. Performance claims require release measurements on the
actual host; see [performance](docs/performance.md).

## Features

- List and icon views, multi-selection, sorting and hidden-file controls.
- Tabs with independent navigation and an optional two-directory split.
- Sidebar places and pinned folders, a path bar per pane and an editable location.
- Folder filtering and cancellable recursive filename search.
- Bounded text and image Quick Look previews.
- Create, rename, copy, move and system trash, with existing destinations protected.
- Default-app opening, reveal in the native file manager and Copy Path.
- Graphite, Dusk and Light, nine accent choices, density and icon-size settings.
- Saved preferences and session directories with isolated verification storage.

File copy/cut/paste currently uses Finder's in-app clipboard. Copy Path uses the
system text clipboard. Operations do not provide overwrite/merge, application
undo or mid-operation cancellation. Copies do not yet preserve ACLs, extended
attributes or macOS resource forks. See the [operation contract](crates/finder-operations/README.md)
and [remaining release gates](docs/verification.md).

This is an initial implementation aimed at production quality. Linux is the
local validation platform. Native macOS/Windows runtime acceptance, full Files/
Finder feature parity, packaging/signing and measured performance acceptance
remain required before a production release. No production installers are
published by this repository yet.

## Build and run

Use the pinned Rust 1.97.1 toolchain through rustup and a graphical desktop.
On Debian/Ubuntu:

```sh
sudo apt-get install pkg-config libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
./scripts/run-local.sh
```

Open a directory or launch with isolated test settings:

```sh
./scripts/run-local.sh --path /some/folder
./scripts/run-local.sh --data-root /tmp/neptune-finder-review --no-restore
```

See [installation](docs/installation.md) for macOS/Windows prerequisites and
[development](docs/development.md) for build limits, scoped checks and CI.

## Documentation

| Guide | Contents |
| --- | --- |
| [User guide](docs/usage.md) | Navigation, selection, views, file operations and preferences |
| [Installation](docs/installation.md) | Source builds and CLI |
| [Architecture](docs/architecture.md) | Worker ownership, core boundaries and file safety |
| [Design](docs/design.md) · [Provenance](docs/provenance.md) | Exact ecosystem tokens/components/assets and necessary adaptations |
| [Development](docs/development.md) | Toolchain, scoped checks, native capture and CI |
| [Verification](docs/verification.md) · [Performance](docs/performance.md) · [Local results](docs/validation-2026-10-17.md) | Evidence scope, measured results and remaining production gates |
| [Releases](docs/releases.md) | Candidate policy and distribution requirements |

## License

Neptune Finder is [MIT licensed](LICENSE). Bundled fonts retain their SIL Open
Font License notices; Tabler icons and vendored skills retain their upstream
licenses and attribution. See [third-party notices](THIRD-PARTY-NOTICES.md).
