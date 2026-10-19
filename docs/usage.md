# User guide

Neptune Finder opens your home directory by default. A directory passed on the
command line opens directly. Preferences and session directories restore on the
next launch unless restoration is disabled.

## Navigation and views

Use the sidebar's familiar places or pinned folders, open a folder in the file
view, or click an enclosing folder in the toolbar's path bar or the path bar at
the bottom of a pane. When a deep path does not fit, the toolbar's … button lists
the folders it hides. To type a location, click the toolbar's path bar beside
its folders, press Go to Folder's shortcut or choose Go to Folder (in the
toolbar's More menu); Enter opens it and Escape keeps the current folder. A
window too narrow for the path bar asks in a dialog instead. The location
accepts absolute paths, paths relative to the active pane, `~` and `~/…`. Back
and Forward use that pane's own history; Up opens its parent. A mouse's side
Back and Forward buttons travel the pane under the pointer, or the active pane
elsewhere in the window. Middle-click a folder in the file view, sidebar or
either path bar to open it in a new tab.

List view shows sortable file details; drag the dividers between column
headers to resize them, or double-click one to restore the default widths.
Icon view uses the same entries and selection with adjustable icon size, and
shows thumbnails where it can: PNG, JPEG, GIF, WebP and BMP images are reduced
on background workers, and on Linux and other freedesktop.org desktops a
current thumbnail that another application (such as GNOME Files) already saved
is reused, which covers videos, PDFs and other formats. Other files keep their
kind glyph, and thumbnails appear as they finish.
Folders appear first; hidden files are configurable. Each pane's path bar
shows where it is and how many items it holds or has selected. Toggle the
sidebar from the toolbar or with Command+Shift+B (⌘B on macOS), and drag its
trailing edge to resize it. In narrow windows it yields its space to files;
the same button or shortcut then opens it over the files, and it closes again
once you choose a place, click outside it or press Escape. A loading state
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
double-click; files request their operating system's default application,
except ZIP archives, which extract (see [Archives](#archives)). A middle click
opens a file the same way without changing the selection.

Drag from empty space to draw a selection rectangle: between or below icons,
below the last row, or anywhere in a list row other than its icon and name. The
rectangle selects what it crosses and scrolls the pane while dragged past its
top or bottom. Hold Shift to add to the current selection, or Ctrl (Command on
macOS) to toggle the items it crosses. Dragging an item by its icon or name
still drags the selection to another folder. Clicking empty space clears the
selection.

The context menu offers actions for its target, including creation, rename,
copy/cut, paste, trash, Quick Look, Copy Path, Properties and native reveal.
Copy and Cut capture the selected paths. Navigate to the destination
pane/folder and Paste. The current file clipboard is local to Neptune Finder;
Copy Path writes text to the system clipboard. Cross-application native file
clipboard integration is a remaining release requirement.

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

## Archives

Open a ZIP archive (`.zip`) with Enter, a double-click, a middle click or the
context menu's Extract to extract it beside the archive. An archive holding one
top-level item produces that item; several items are gathered in a folder named
after the archive, as Ubuntu Files and macOS Finder do. Extract to… extracts
into a typed folder, accepting `~` and paths relative to the archive's folder.
An existing item is never replaced or merged: the result takes the next free
name, such as “Photos 2”. Open in default app hands the archive to the operating
system instead. Other formats, such as `.tar.gz` and `.7z`, still open in their
default application.

Compress… creates a ZIP archive of the selected items in the current folder,
suggesting the item's name or “Archive.zip” for several. An existing archive
name is reported rather than replaced. Archives keep file contents, symbolic
links, Unix permissions and modification times (to ZIP's two-second precision).
Password-protected archives, entries that would land outside the extracted
folder and archives built to expand without limit are refused, and a failed
extraction leaves nothing behind. See
[the operation contract](../crates/finder-operations/README.md) for the limits.

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

## Properties

Choose Properties from an item's context menu, or select one item and press
Alt+Enter or Command+I, to see its kind, parent folder, dates, permissions and
the free space on its volume. With nothing selected, the shortcut (or
Properties in the context menu of the folder's empty area) describes the folder
in view. Permissions say what you, the owner, the group and everyone else may
do, and give the mode as `ls` writes it; on Windows they show the read-only
attribute.

A folder's item count and total size update while a worker counts everything
beneath it, and closing the sheet stops the count. Counting never follows
links or enters other volumes mounted inside the folder, and stops after
1,000,000 items or 64 folder levels; the sheet says when unreadable items or
these limits leave the total incomplete. The size is the length of the
regular files counted, so a hard-linked file counts each time it appears.
For an item outside the pane's folder, such as a recursive search result,
Open Parent Folder shows that folder with the item selected.

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
| Properties | Alt+Enter or Command+I |
| Open selected item; extract a ZIP archive | Enter |
| Preferences | Command+, |
| Zoom in / out / actual size | Command+= or Command++ / Command+- / Command+0 |
| Clear selection | Escape |

The interface implements the core workflow above. Full Ubuntu Files/macOS
Finder parity, browsing network services or the trash, indexed content search,
file tags, archive editing, application undo and signed installers are remaining
product/release work; see [verification](verification.md).
