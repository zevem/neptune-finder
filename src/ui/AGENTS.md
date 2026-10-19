# Native widgets

Apply [docs/design.md](../../docs/design.md) when changing visual hierarchy,
spacing, typography or controls. Keep files dominant and chrome quiet; use the
existing theme tokens, official outline icons and shared controls before adding
a new visual pattern. `../theme.rs`, `../icons.rs` and `../fonts.rs` supply the
same visual system as Neptune Terminal.

Widgets consume prepared view data and emit actions with stable targets.
Overlay/input ownership is explicit; retain dialog targets and preserve
cancellation and file-view focus restoration. Do not read filesystem metadata,
file contents or directory entries during rendering.

Use stable semantic egui IDs across hover/press/release and responsive hiding.
A neighboring control disappearing must not transfer keyboard focus or change
what Enter activates. Give icon-only controls matching tooltips and accessible
names. Narrow windows need contained, scrollable overlays with reachable
close/actions, not clipped forms.

Virtualize file rows and grid rows; create widgets only for the visible range.
A selection is keyed by path identity, not its current sorted row index. Sort,
filter and navigation must not silently act on a different file. Do not add
per-item texture decoding or path I/O to a visible-row callback.

Review fresh native captures after the final build, using
[scripts/AGENTS.md](../../scripts/AGENTS.md). Check the changed control's normal,
focused and open/closed states as applicable, relevant themes, single/split
panes and a narrow window. Inspect text/icon alignment, path truncation, focus
clarity and clipping. For motion/timing, observe the running transition.
Synthetic egui tests prove identity/logic; they do not approve native appearance.
