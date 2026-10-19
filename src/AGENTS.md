# Desktop composition

`neptune-finder` is the native desktop package; `src/main.rs` launches Neptune
Finder. Do not move desktop services into the filesystem crates or expose GUI
backend internals through the core API.

## Find the owner

| Change | Entry points / local contract |
| --- | --- |
| Frames, tabs, panes, navigation and asynchronous completion routing | `app.rs` |
| Widgets, overlays and presentation data | [ui/AGENTS.md](ui/AGENTS.md) |
| Listing, filtering and recursive search | `../crates/finder-core/` |
| Navigation, stable selection and saved session DTOs | `model.rs` |
| Bounded text/image Quick Look workers | `preview.rs` |
| Filesystem mutation, cancellation and collision handling | `../crates/finder-operations/` |
| Settings, paths and saved-state protection | `config.rs`, `persistence.rs` |
| Shared palette, fonts and icons | `theme.rs`, `fonts.rs`, `icons.rs` |

Keep directory handles, generation cancellation, preferences, search and overlays
application-owned. UI rendering reads prepared state and never enumerates a
directory, reads a file, starts a native dialog or waits for an operation.
Background work reports its original pane/tab and request generation; focus
changes must not retarget or duplicate it. Close/navigation cancels obsolete
work, and stale results are discarded.

Input routing gives overlays, editable fields, search and the file view explicit
ownership. Preserve host shortcuts, IME/preedit, and text/key ownership without
duplicate delivery. Confirmation retains its original target even if focus or
selection changes before confirmation. A key opening a dialog must not also
confirm it.

Search changes cancel obsolete requests. Preview reads, recursive walks and
operation traversal have explicit limits. A slow or unavailable mount must not
hold the frame thread. Diagnostics never contain names, paths or file contents.

## Focused proof

Choose the relevant filter/target; these are alternatives, not a checklist:

```sh
cargo test -p neptune-finder --locked config::tests
cargo test -p finder-core --locked <affected_test>
cargo test -p finder-operations --locked <affected_test>
cargo check -p neptune-finder --bin neptune-finder --locked
```

Format changed Rust files with the pinned rustfmt. Dependency/seam changes can
use `python3 scripts/check-architecture.py`. Coordinate compilation with the
validation owner, use the configured job limit and avoid simultaneous GUI builds.
For user-visible changes, use [the native workflow](../scripts/AGENTS.md), then
inspect fresh captures of affected states. Headless success does not establish
native visual acceptance.
