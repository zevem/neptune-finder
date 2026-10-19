# Native verification and probes

Prefer `native-harness.py` for isolated native capture: it owns a fresh data
root, fixture directory, subprocess handle, logs, screenshot and cleanup. Build
only the required desktop target and coordinate one compilation owner:

```sh
cargo build -p neptune-finder --locked --bin neptune-finder
python3 scripts/native-harness.py --output artifacts/native
```

For final release captures or performance probes, build with `--release` and
pass `--app target/release/neptune-finder`. Load [verification](../docs/verification.md)
for native acceptance or [performance](../docs/performance.md) for measurements.
Use each script's `--help` for current arguments.

The optional inspection workflow drives the actual native interface through
accessible labels and bounds, with a task-owned listener/process/data root:

```sh
cargo build -p neptune-finder --features inspection --locked --bin neptune-finder --bin neptune-finder-inspect
python3 scripts/inspect-regression.py --output artifacts/native-flow
```

It asserts creation, rename, copy, moves, their Undo and Redo, compression and
extraction only in temporary fixtures. The Trash confirmation is cancelled, never
accepted, and Undo stops before reversals that would use the OS Trash. Captures, native
trees, logs and a truthful report remain under the selected output directory.
Client events verify application routing; they do not substitute for native OS
keyboard integration.

## Isolation hazards

- Every independent launch needs a fresh `--data-root PATH`. `--config` and
  `--no-restore` do not isolate future session/settings writes.
- Reap only captured subprocess handles/PIDs owned by this run. Never kill by
  app name, worktree path or pattern, and never drive the user's working window.
- OS keyboard/pointer workflows run sequentially because desktop focus is global.
  Target the exact owned native window, using semantic accessible labels where
  available and bounded polling rather than arbitrary delays.
- Use temporary fixture directories for all filesystem mutations. Never use a
  real personal folder to verify overwrite, trash, cancellation or recovery.

## Evidence and focused checks

`--screenshot` captures the actual egui/wgpu native framebuffer and exits. It
needs a working display; on headless Linux CI use `xvfb-run -a`. Captures do not
establish real OS input, clipboard, drag/drop, default-app or system-trash
integration. Inspect fresh images of affected states after the final build.

The architecture checker is a lightweight boundary proof. Its self-tests are
needed when changing the checker, not after every Rust change:

```sh
python3 scripts/check-architecture.py --self-test
python3 -m unittest discover -s scripts -p test_native_harness.py
```

Performance requires optimized release builds. Distinguish directory scan,
frame CPU, process CPU/RSS, operation throughput and input-to-display latency.
Record actual host, filesystem and build metadata; keep failed runs as evidence.
Do not launch native/resource scenarios for instruction-only edits.

Release packaging follows [the release guide](../docs/releases.md). Never create
a tag/release just to test a future pipeline without explicit release authority.
