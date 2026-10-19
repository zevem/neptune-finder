# Verification and release gates

Neptune Finder is a native cross-platform implementation. This document separates
configured validation from observed native behavior. Production readiness must be
established by the evidence below, never inferred from the framework, copied CI
or another Neptune application's acceptance records.

## Development proof

Focused tests use temporary directories for listing, sorting/filtering,
navigation, search, bounded previews, collision handling and filesystem
operations. Rendering/worker seam changes also run the lightweight architecture
check. CI owns full workspace format, lint, tests and native builds; designated
validation is the only owner of explicitly requested local full checks.

## Native application verification

Build the required binary using the pinned toolchain and lockfile, then launch
with a new data root. The capture harness creates its own fixture folder and
never mutates the user's files:

```sh
cargo build -p neptune-finder --locked --bin neptune-finder
python3 scripts/native-harness.py --output artifacts/native-review
```

For native application interaction and a broader capture set, use the optional
inspection build and isolated flow:

```sh
cargo build -p neptune-finder --features inspection --locked --bin neptune-finder --bin neptune-finder-inspect
python3 scripts/inspect-regression.py --output artifacts/native-flow
```

This owns the listener/process/storage and tests file mutations only in its
fixture. The script cancels Trash and records every failed condition. Its egui
input verifies application routing; native OS integration still needs direct
acceptance. See [development](development.md#native-verification).

Review the resulting screenshot from the actual native app. Repeat relevant
states in Graphite, Dusk and Light and at narrow window widths. Native captures
verify framebuffer output; actual pointer/keyboard and OS-service behavior need
interactive native testing. Keep the exact owned process/window target and run
OS-input tests sequentially because focus is shared.

| Screen/state | Review |
| --- | --- |
| Directory list | Column alignment, long names, selection, sorting and visible-row rendering |
| Icon view | Grid wrapping, names, multi-selection and scrolling |
| Split view and tabs | Independent navigation/selection, clear focus, narrow layout |
| Search | Loading, empty/error results, cancellation and navigation away |
| Quick Look | Bounded text/image previews, unsupported/binary files, error and dismissal |
| File operation dialogs | Stable targets, collision/error feedback, progress, cancellation |
| Preferences and location | Theme changes, settings persistence, editable path focus, clipping |
| Restored session | Tabs/directories, invalid or unavailable paths, damaged/unsupported state |

## Remaining production release gates

| Area | Required evidence |
| --- | --- |
| Linux X11 and Wayland | Native release build, input, clipboard, drag/drop, dialogs, default-app opening, trash, mounts and DPI |
| macOS | Native Apple Silicon/Intel package acceptance; filesystem permissions, Trash, default-app opening, keyboard conventions, IME, accessibility and Retina |
| Windows | Native installed package; path/drive/UNC handling, long names, symlinks, sharing violations, Recycle Bin, clipboard, dialogs and DPI |
| File safety | Existing destinations, self-copy, recursive source/destination overlap, symlinks, readonly files, shutdown/queued cancellation, partial failures, cross-filesystem moves and unavailable mounts; implement/review interactive operation cancellation and progress before claiming them |
| Search and preview | Large/deep trees, Unicode and non-UTF-8 names where supported, symlink cycles, unreadable/binary/large files and rapid cancellation |
| Responsiveness | Release startup, first result, scroll frame percentiles, idle CPU/RSS, large folders, previews and operation load with recorded host/filesystem |
| Recovery | Atomic settings/session saves, malformed or unsupported saved state, crash during operation and clear partial-result reporting |
| Distribution | Platform packaging, signing/notarization, checksums, licenses, uninstall and authenticated update strategy |
| Visual/accessibility | Real native captures of every changed screen, narrow windows, theme contrast, keyboard focus, screen-reader roles and IME |

Do not advertise network browsing protocols, indexed content search, OS file tags,
archive editing, privileged operations or transactional undo unless the feature
is implemented and verified. Copies currently do not preserve ACLs, extended attributes, resource forks, sparse
allocation, hard-link relationships or directory timestamps. File copy/cut/paste
is currently application-local; cross-application file clipboard and native drag/drop
need implementation/acceptance before a full file-manager parity claim.

System trash offers recoverability through the OS;
it does not establish an application-wide undo stack. CI configuration is not a
completed CI run. Actual results belong in the delivery report with their build
and host scope.
