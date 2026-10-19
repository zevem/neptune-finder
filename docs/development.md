# Development guide

Neptune Finder uses the same native Rust `egui`/`eframe` 0.36.2 and `wgpu` stack
as Neptune Terminal. Its visual tokens, fonts, Tabler icon assets, engineering
guidance and repository skills are ported from that repository. Filesystem core
and operations replace the terminal engine and PTY crates. No browser/webview
hosts the interface.

[Project overview](../README.md) · [Installation](installation.md)

## Development builds

Use the pinned Rust 1.97.1 toolchain through rustup and the checked-in lockfile.
The repository defaults to two Cargo build jobs per invocation so agents working
in parallel leave capacity for the desktop. Coordinate one compilation owner;
do not increase job counts without an explicit request. Commands run from the
repository root.

```sh
./scripts/run-local.sh --path "$HOME"
```

The launcher uses `cargo run --release --locked --bin neptune-finder` with the
pinned toolchain and forwards all arguments to Finder. Cargo rebuilds only when
needed. You can invoke the script by its absolute path from another directory.

`--data-root PATH` isolates preferences and session state. `--no-restore` starts
without restored tabs but does not isolate future writes. Use a task-owned data
root for every test launch, including relaunches.

## Focused verification

Local development uses the narrowest meaningful affected crate/target test;
CI owns workspace-wide formatting, lint, tests and builds. Run repository-wide
checks locally only when explicitly requested. The [agent guides](../AGENTS.md)
provide scoped commands. Typical alternatives are:

```sh
cargo test -p finder-core --locked <affected_test>
cargo test -p finder-operations --locked <affected_test>
cargo check -p neptune-finder --bin neptune-finder --locked
python3 scripts/check-architecture.py
```

Do not compile concurrently with another agent. Format changed Rust files using
the pinned rustfmt. Documentation-only changes need reference/consistency review,
not Rust builds.

## Native verification

A task-owned native run creates its own process, fixture directory, data root,
logs and screenshot. The harness never attaches to the user's file manager.

```sh
cargo build -p neptune-finder --locked --bin neptune-finder
python3 scripts/native-harness.py --output artifacts/native-review
```

Pass `--app target/release/neptune-finder` for release captures, `--theme light`
or `--theme dusk` for alternate materials, and `--size 720x540` for narrow-window
review. A graphical desktop is required. Headless Linux CI uses `xvfb-run -a`
with the same executable, GPU framework and screenshot event as the native app.
A framebuffer capture establishes appearance; actual pointer/keyboard, clipboard,
IME, drag/drop, native dialogs and platform trash still need native interaction.

The optional native inspection feature supports semantic, repeatable application
interaction. The ordinary application starts no inspection listener. Build the
app and developer client, then run the owned flow:

```sh
cargo build -p neptune-finder --features inspection --locked --bin neptune-finder --bin neptune-finder-inspect
python3 scripts/inspect-regression.py --output artifacts/native-flow
```

The flow owns a unique loopback endpoint, process and temporary data/fixture
folders. It drives the real app through accessible control bounds and egui input,
asserts fixture create/rename/copy behavior, cancels the Trash confirmation, and
captures list/grid/split/tabs, search, previews, dialogs, narrow layouts and all
three themes. `report.json`, per-screen accessibility trees, screenshots and
`app.log` are retained; failures are reported without claiming unobserved passes.
No personal files or existing application state are used. These events establish
application input routing, not native OS keyboard/clipboard/drag/drop behavior.

For headless Linux verification:

```sh
xvfb-run -a env -u WAYLAND_DISPLAY WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1 python3 scripts/inspect-regression.py --output artifacts/native-flow
```

Use only owned process handles for cleanup. Desktop input tests run sequentially
because keyboard/pointer focus is global. Inspect fresh captures after the final
build; success of a harness alone does not establish visual acceptance.

## Continuous integration

The CI workflow is adapted from Neptune Terminal, retaining pinned actions,
read-only permissions, timeouts, Rust 1.97.1, the low-overhead `ci` profile,
change classification, dependency caching and the final `Check` gate. It runs
formatting/architecture checks, a Linux/macOS/Windows build-and-test matrix,
Linux Clippy, exact bundled asset hashes, latest-stable compatibility and isolated Linux native capture.
Documentation-only changes skip compilation while keeping jobs successful.

PRs test GitHub's merge commit. A push to `main` tests the resulting commit and
classifies the whole pushed range. New commits replace older runs for the same
PR; separate pushes to `main` remain independent. The final gate rejects failed,
cancelled or skipped dependency jobs. This repository does not assume that
Terminal's branch rules, signing secrets, website or release infrastructure have
already been provisioned here.

## Artifacts and releases

`artifacts/` is ignored local output for screenshots, logs, measurements and
isolated test data. Generated evidence is not committed. Review artifacts before
uploading: file manager screenshots may show private names and paths. Preserve
build/host metadata needed to assess claims.

The [release guide](releases.md) defines the remaining packaging and native
acceptance requirements. No signing, updater or download infrastructure is
claimed merely because Terminal has it. AI agents must not create/push release
tags or publish releases unless explicitly asked to release that version.
