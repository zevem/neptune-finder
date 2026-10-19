# Native implementation validation — 2026-10-17

This is development evidence, not a production release acceptance record.
The application was built and exercised on Ubuntu 26.04.1, Linux 7.0.0-38-generic,
x86_64, Intel Core i9-13900HX (32 logical CPUs), with pinned Rust 1.97.1.

## Correctness and native UI

- 17 filesystem-core tests, 17 file-operation tests, 25 desktop tests and
  2 inspection-client tests passed across focused runs (61 unique tests).
- The desktop library, binaries and tests passed Clippy with warnings denied.
- Formatting, GUI/filesystem dependency boundaries, four boundary self-tests,
  four capture-harness tests and bundled-asset verification passed.
- The shared asset check verifies 93 exact Neptune Terminal font/Tabler files.
  The three user-supplied Finder branding PNGs are preserved; eleven platform
  icon assets derive from them.
- Native inspection exercised nine workflow assertions and captured 26 actual
  application states, using a unique loopback endpoint, temporary fixtures and
  isolated settings. It created, renamed and copied fixture files, verified
  recursive search/navigation and saved theme changes, cancelled Trash, and
  checked unavailable-folder recovery. Screens include list, icons, tabs, split,
  searches, context menu, text/image Quick Look, file dialogs, location, three
  themes, narrow windows, branded About, empty and error states.

Native UI evidence uses X11/Xvfb and software Vulkan. Inspection injects toolkit
events into a real native application; it does not verify actual OS key routing,
clipboard interoperability, drag/drop, IME, hardware GPU performance, default-app
opening or real Trash integration. No user files were modified by the harness.
The source checkout is initially uncommitted; CI is configured but has not run
on a remote host.

The default-feature optimized Linux executable also built without warnings in
3 minutes 13 seconds. `target/release/neptune-finder` is 24,900,648 bytes
(23.747 MiB), SHA-256
`62f54136241f7ed20645b414531b0212bda005abf0170cc96a23fc328163ad30`.
Its version/help commands and read-only 10,000-entry ext4 fixture probe passed;
that individual CLI scan reported 17.923 ms. A separate release framebuffer
capture passed and was visually reviewed at 1180×780 with isolated storage.
The capture's 3.475-second process duration includes an intentional screenshot
delay and is not a startup-latency measurement. Release evidence is under
`artifacts/native-release/`.

Generated screenshots, accessibility trees, logs and the machine-readable report
live under `artifacts/native-final/`. Earlier unsuccessful captures are retained
separately. A text-only accessibility selector initially checked a label where
egui reports text as a value; the selector was corrected and the flow passed.

## Optimized directory scan measurements

The `finder-core` release benchmark generated 10,000 files, then performed 11
scans. Fixture creation is excluded; the first scan is reported separately and
warm percentiles use the remaining ten scans. OS caches were not reset. Each
scan includes enumeration, metadata and natural filename sorting.

| Fixture filesystem | First timed scan | Warm p50 | Warm p95 |
| --- | ---: | ---: | ---: |
| Project filesystem, ext4 | 12.434 ms | 10.349 ms | 11.622 ms |
| `/tmp`, tmpfs | 10.260 ms | 8.863 ms | 10.935 ms |

These numbers measure the listing engine on this host. They do not establish
cold-disk performance, first-paint/input latency, scrolling percentiles, copy
throughput, performance on network/removable volumes or superiority over another
file manager. A 100,000-entry UI test independently verifies that list and grid
create only visible file widgets and a bounded number of paint shapes.

To reproduce the ext4 probe, choose an existing task-owned directory on the
filesystem being measured:

```sh
TMPDIR="$PWD/artifacts/bench-data" cargo run --release --locked -p finder-core --example scan_benchmark -- 10000 11
```

The benchmark creates and removes only its unique fixture below that directory.

## Production acceptance still required

The [release gates](verification.md) remain authoritative. Native Windows/macOS
runtime acceptance, real Wayland and OS-service testing, cross-application file
clipboard, operation progress/cancellation, extended metadata preservation,
crash recovery, installed packaging/signing and broader release performance
measurements remain outstanding. Files/Finder feature parity and a production
release are not claimed by this development evidence.
