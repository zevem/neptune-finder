# Performance strategy and measurements

The initial Linux measurements and their filesystem/build scope are recorded in
[the 2026-10-17 validation report](validation-2026-10-17.md).

Responsiveness is a product requirement. Neptune Finder keeps filesystem work
outside UI frames, cancels obsolete directory/search work, bounds preview reads,
virtualizes list/grid rows and repaints in response to input or changed worker
results. Hidden tabs release their directory listings and scan workers; only
the visible one or two panes retain active snapshots. Large replaced snapshots
are retired through a bounded worker path, keeping bulk destruction off frames. These choices create a measurable architecture; they do not establish
that Finder is faster than another explorer.

GPU swapchain acquisition runs synchronously on the UI thread, so frame CPU time
alone does not establish responsiveness. Linux presents with wgpu's
`AutoNoVsync`, matching Neptune Terminal's fix for an NVIDIA FIFO acquire that
blocked the whole window for seconds after it sat idle behind other
applications. Immediate presentation is selected where the surface offers it and
can permit tearing on native X11; other operating systems keep `AutoVsync`.
The stall was reproduced in Terminal under XWayland, not yet in Finder.

## What to measure

| Measurement | Purpose |
| --- | --- |
| First useful frame and first directory result | Startup and navigation responsiveness |
| Directory scan time and entries per second | Read-only listing cost on a known filesystem |
| Frame p50/p95/p99 while scrolling/filtering | Interactive smoothness with large directories |
| Search cancellation and first result latency | Responsiveness while typing or switching folders |
| Idle process CPU/RSS | Whether a settled window sleeps and retains bounded state |
| Copy/move throughput and cancellation latency | End-to-end operation behavior |
| RSS with large folders and image previews | Retained state, textures and snapshot bounds |

Use optimized release builds. Record commit, lockfile, toolchain, OS, CPU, RAM,
GPU/backend, display scale, dimensions, filesystem/mount, dataset size and whether
caches are warm. Compare identical workloads and keep unsuccessful runs as
diagnostic evidence. Scan throughput is distinct from GPU rendering and
input-to-display latency.

## Reproducible listing probe

```sh
cargo build -p neptune-finder --release --locked --bin neptune-finder
./target/release/neptune-finder --benchmark /path/to/representative/directory
```

Use both a typical folder and a generated large directory in task-owned storage.
Do not benchmark by mutating a user's real folder. Report the actual count and
elapsed time. The command measures the listing path; it does not prove scroll
performance, recursive search speed or Explorer/Finder parity.

The native inspection flow also records a Linux-only development-build idle
observation from its owned process's `/proc` CPU ticks and RSS. It waits one
second without inspection requests, then samples another quiet second. This
excludes startup and records the graphics environment, but is not optimized
release acceptance or a native hardware-GPU performance claim.

## Native measurements

Launch the same release executable in isolated storage. Sample only its owned
PID, and state whether startup/GPU initialization is included. Use the same
window dimensions, sort and view mode for comparisons. A quiet window should
have no continuously requested frames; verify process CPU at rest and after
search/preview completion. Profile before replacing the renderer or worker
architecture.

Large-folder review includes loading, scrolling, sorting, hidden-file toggles,
search cancellation, navigation away during scanning and closing the tab. Slow
network/removable mounts need separate measurements. File operations include
permission failures, existing destinations, cancellation and cleanup of partial
results; throughput alone is insufficient.

No fixed throughput, latency or platform superiority claim is made without an
attached host-specific result. See [verification](verification.md) for release
gates and the current evidence scope.
