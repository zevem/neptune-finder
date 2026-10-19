# Architecture and renderer decision

Neptune Finder is a native Rust desktop file explorer. It uses the same
`eframe`/`egui` 0.36.2 interface and `wgpu` renderer as Neptune Terminal, with
native windows and operating-system filesystem APIs. No browser or webview hosts
its interface. The Terminal repository's local eframe patch supports its
Wayland multi-window terminal transfers; Finder uses the same upstream framework
version without that terminal-specific patch.

Cross-platform dependencies and CI establish build coverage. Production support
requires [native acceptance](verification.md) on the actual operating systems.

## Component boundaries

| Component | Owns | Must not own |
| --- | --- | --- |
| `finder-core` | Directory snapshots, entry kinds, sorting/filtering and recursive search | GUI types, GPU resources, native dialogs or interactive frames |
| `finder-operations` | Copy, move, rename, creation, trash, collision checks and completion reporting | GUI widgets or implicit overwrite decisions |
| `neptune-finder` desktop | Worker orchestration, navigation/selection model, tabs/panes, bounded previews, native services and settings/session persistence | Blocking filesystem work while painting |
| `theme.rs`, `icons.rs`, `fonts.rs` | Neptune materials, geometry, typography, official icon fonts and shared controls | Filesystem or operation policy |

A directory response retains the originating target and request identity.
Navigation cancels superseded scans; a late response cannot replace a newer
folder or another pane. Recursive search and previews follow the same rule.
Operations capture source paths and destination before enqueueing, so a later
selection change cannot retarget an accepted copy, move or trash request.

The display path is:

```text
filesystem → bounded background work → owned directory/search/preview snapshot
           → visible list/grid rows → egui/wgpu → native window
```

The mutation path is:

```text
captured sources + destination → operation worker → completion/result
                              → refresh affected directory snapshots
```

Directory enumeration, metadata, canonicalization, recursive search, preview
reads, file operations and persistence belong outside interactive frames.
Rendering submits only visible list/grid rows. Idle windows sleep between input,
worker notifications and necessary animation deadlines. Repaint notifications
must be coalesced instead of being sent for every byte or every directory entry.

## Correctness under asynchronous work

Selection belongs to stable file identities rather than row indices. Sorting or
filtering must preserve the association between an action and its file. Each
pane owns its navigation history and current directory; its tab and request
identities survive asynchronous completions. Closed tabs reject late results. Only the visible tab
retains directory listings and scan workers: switching tabs releases hidden
listings/workers and re-reads them on activation. Replaced listing vectors are
retired on a bounded worker path so large snapshots are not freed in UI frames.

Never silently replace an existing destination. Recursive operations must handle
self-copy, destinations inside a source directory, symlinks, cancellation,
permission failures and partially completed work explicitly. Queued work is
bounded; dropping the service cancels queued requests while an accepted operation
finishes its transaction. There is no interactive mid-operation cancel or
application undo in the current implementation. System trash is preferred for
recoverable deletion. A failed trash request must remain a failure instead of
falling back to permanent deletion.

Search reads and desktop-owned previews (`src/preview.rs`) are bounded and
superseded by newer requests. Symbolic links must not
turn a recursive walk into a cycle, and a slow mount must not block a UI frame.
Diagnostics exclude paths, names and contents. Saved sessions store navigation
and settings, never operation queues or file contents. Saved path serialization
preserves native non-UTF-8 path identities rather than lossy display strings.

## Verification

`python3 scripts/check-architecture.py` guards GUI-free core/operation crates
and rendering files against direct filesystem access. It is a lightweight seam
check, not proof that every filesystem behavior is correct. Focused filesystem
tests use isolated temporary directories; native review uses a fresh data root.

Record OS, filesystem, GPU/backend, display scale, window dimensions, dependency
lockfile and build profile for performance or support claims. Directory scanning
throughput does not establish first useful frame, scroll latency or end-to-end
copy correctness. See [performance](performance.md),
[development](development.md) and [release gates](verification.md).
