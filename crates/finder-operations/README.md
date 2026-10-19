# Finder operations

`Operations` accepts at most 32 queued requests and retains at most 32 unread
completions. Its single worker performs filesystem and desktop-launch work.
`submit` and `poll` do not perform filesystem I/O or wait for queue capacity.
Dropping the service cancels queued work without waiting on disk I/O. An
in-progress operation can finish while the process remains alive. The
application should await its completions before allowing process exit; a
forced exit may leave a private staging directory, with the original retained.

Creating, copying, moving, and renaming never replace an existing destination.
Copies use a private temporary directory on the destination filesystem, then
publish the completed payload with an exclusive native rename. A failed copy
does not expose an incomplete destination or modify the source. A batch is
preflighted before work begins; a later failure reports how many items already
completed. Completed batch items are retained rather than destructively rolled
back.

Cross-filesystem moves publish a full copy before moving the original into the
OS Trash. The original's identity, type, size, and modification time are checked
again before recycling. A changed original or recycling failure leaves the
original and completed copy in place and reports both locations. Ordinary
same-filesystem moves use the native exclusive rename directly.

Copies retain regular file contents, file permissions and timestamps, and
symbolic links, including dangling links. They stream directory traversal and
file I/O with a 256-level nesting limit. Directory symlink/junction ancestors
are rejected; symbolic links inside copied trees are preserved without
traversing their targets. Devices, sockets, and pipes are rejected. Windows
symbolic-link creation requires the OS privilege or Developer Mode.

Current limits: extended attributes, ACLs, resource forks, sparse allocation,
hard-link relationships, and directory timestamps are not preserved. Copying
actively written trees does not provide a filesystem snapshot. Directory
ancestry validation and OS Trash handoffs remain path based and are not a
security boundary against a malicious process concurrently replacing ancestor
directories. Open writers are not locked; cross-filesystem originals remain
recoverable in Trash. The service does not provide undo, permanent deletion,
automatic overwrite, or directory merging.

Native Open and Reveal pass exact native paths as process arguments, without a
shell. One shared sleeping reaper bounds live launcher processes and prevents
zombies. On Linux Reveal opens the containing folder; macOS and Windows select
the item. An accepted launch reports a request; subsequent native file
association dialogs and errors belong to the desktop.

Focused verification, coordinated with the repository's compilation owner:

```sh
cargo test -p finder-operations --locked
cargo clippy -p finder-operations --all-targets --locked -- -D warnings
```

Tests use unique temporary fixtures. Cross-device and Trash failure tests inject
a fixture-only recycler and never modify the user's OS Trash or files.
