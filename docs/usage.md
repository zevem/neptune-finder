# User guide

Neptune Finder opens your home directory by default. A directory passed on the
command line opens directly. Preferences and session directories restore on the
next launch unless restoration is disabled.

## Navigation and views

Use the sidebar's familiar places or pinned folders, open a folder in the file
view, click a folder in the path bar at the bottom of a pane, or choose Go to
Folder (in the toolbar's More menu) to enter a location. The
location accepts absolute paths, paths relative to the active pane, `~` and
`~/…`. Back and Forward use that pane's own history; Up opens its parent.

List view shows sortable file details; drag the dividers between column
headers to resize them, or double-click one to restore the default widths.
Icon view uses the same entries and selection with adjustable icon size.
Folders appear first; hidden files are configurable. Each pane's path bar
shows where it is and how many items it holds or has selected. The sidebar
yields space in narrow windows; toggle it from the toolbar or with
Command+Shift+B (⌘B on macOS), and drag its trailing
edge to resize it. A loading state
appears while a worker reads a directory, and unavailable locations show errors
without freezing navigation.

Open another tab to keep a directory available; a tab strip appears under the
toolbar once there is more than one. Split view places two independent
directories side by side; click a pane to make it active before using keyboard
navigation or paste. Up to 24 tabs are retained, with at most two panes per tab.
Session restoration remembers directories and organization; it does not resume
file operations.

## Selection and file actions

Click selects an item. Ctrl-click (Command-click on macOS) toggles items;
Shift-click selects a range. Arrow keys move selection, Shift extends it, and
Select All selects the current result set. Open a selected folder with Enter or
double-click; files request their operating system's default application.

The context menu offers actions for its target, including creation, rename,
copy/cut, paste, trash, Quick Look, Copy Path and native reveal. Copy and Cut
capture the selected paths. Navigate to the destination pane/folder and Paste.
The current file clipboard is local to Neptune Finder; Copy Path writes text to
the system clipboard. Cross-application native file clipboard integration is a
remaining release requirement.

Creating, copying, moving and renaming protect existing destinations. A collision
is reported; automatic overwrite and directory merging are not offered. Copies
are prepared in private destination-side temporary storage and published only
when complete. Batch failures report already completed items rather than
removing them. Cross-filesystem moves finish the destination copy before moving
the source into system trash.

Move to Trash uses the operating system's recoverable trash and requires
confirmation. A trash failure is reported and never falls back to permanent
deletion. Finder has no application-wide undo, permanent delete, merge/overwrite
or mid-operation cancellation yet. Restore trashed items through the operating
system. Native Reveal opens the containing folder on Linux and selects the item
on macOS/Windows; native runtime acceptance is still required on those systems.

Regular file contents, file permissions/timestamps and symbolic links are
preserved by copy. ACLs, extended attributes, resource forks, sparse allocation,
hard-link relationships and directory timestamps are not yet preserved. See
[the operation contract](../crates/finder-operations/README.md) for safety behavior
and current limits before relying on a copy of specialized filesystem data.

## Search and Quick Look

The search field filters names in the current directory. Recursive search also
walks descendants; it searches filenames rather than file contents. Rapid query
changes supersede earlier requests. Hidden-file settings apply to search too.
Walks do not follow directory symlinks, and bounds prevent unlimited traversal:
100,000 retained results, 250,000 visited entries and 64 directory levels. A
truncated result is identified in the UI.

Select one item and press Space for Quick Look. Text previews read at most
128 KiB; supported images are decoded on a worker with byte/pixel limits and
resized to a bounded preview. Unsupported, binary or oversized files show an
explanation. Quick Look does not execute files. Close the preview to return to
the file view.

## Preferences and saved state

Preferences is laid out like Neptune Terminal's: General, Appearance and Files
panes in a source list. Appearance offers Graphite, Dusk and Light using the
exact Neptune Terminal materials, plus nine accent choices. View mode, hidden
files, sidebar visibility/width, list column widths, icon size, row density,
session restoration
and favorites are saved in Finder's own configuration/session storage.

For an isolated launch, pass `--data-root PATH`. `--config PATH` selects an
explicit TOML preferences file but does not isolate session storage.
`--no-restore` ignores restored tabs for that launch while allowing future saves.
If settings/session data is unreadable, malformed or from a future schema,
Finder preserves it and disables saving for that launch, with a visible warning.

Example configuration values match the fields in `src/config.rs`:

```toml
theme = "graphite"
accent = "blue"
view = "list"
show_hidden = false
sidebar = true
sidebar_width = 216.0
icon_size = 56.0
row_height = 28.0
window_zoom = 1.0
restore_session = true
favorites = []

[column_widths]
date = 168.0
size = 78.0
kind = 150.0
```

## Keyboard shortcuts

Command below means Ctrl on Linux/Windows and Command on macOS. Editing fields
and modal dialogs retain their own text/selection shortcuts.

| Action | Shortcut |
| --- | --- |
| Go to Folder | Command+L or Command+Shift+G |
| Search | Command+F |
| New tab / close tab | Command+T / Command+W |
| Toggle split view | Command+\\ |
| Toggle sidebar | Command+Shift+B; ⌘B on macOS |
| Back / Forward / Up | Alt+Left / Alt+Right / Alt+Up |
| Refresh | F5 or Command+R |
| Icon view / list view | Command+1 / Command+2 |
| Show hidden files | Ctrl+H on Linux/Windows; ⌘⇧. on macOS |
| New folder | Command+Shift+N |
| Rename | F2 |
| Copy / Cut / Paste | Command+C / Command+X / Command+V |
| Select all | Command+A |
| Move selected items to Trash | Delete; ⌘Backspace on macOS |
| Quick Look | Space |
| Open selected item | Enter |
| Preferences | Command+, |
| Zoom in / out / actual size | Command+= or Command++ / Command+- / Command+0 |
| Clear selection | Escape |

The interface implements the core workflow above. Full Ubuntu Files/macOS
Finder parity, browsing network services or the trash, indexed content search,
file tags, archive editing, application undo and signed installers are remaining
product/release work; see [verification](verification.md).
