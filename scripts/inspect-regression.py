#!/usr/bin/env python3
"""Exercise and capture a run-owned Neptune Finder through native egui inspection.

All file operations target a temporary fixture. Trash is opened and cancelled,
never accepted. The client injects egui events; actual OS keyboard, clipboard,
IME and native drag/drop need separate platform acceptance.
"""

from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import tomllib
import zipfile
import zlib

REPO_ROOT = Path(__file__).resolve().parents[1]
SUFFIX = ".exe" if os.name == "nt" else ""


def fixture(root: Path) -> Path:
    directory = root / "Neptune review"
    directory.mkdir()
    for name in ("Documents", "Pictures", "Projects", "Destination", "Empty folder"):
        (directory / name).mkdir()
    (directory / "Welcome.md").write_text(
        "# Welcome to Neptune Finder\n\nNative files. A little space to explore.\n",
        encoding="utf-8",
    )
    (directory / "Notes.txt").write_text("A native inspection fixture.\n", encoding="utf-8")
    (directory / "Documents/Research.txt").write_text("Recursive search result.\n", encoding="utf-8")
    (directory / ".hidden-note").write_text("Hidden file fixture.\n", encoding="utf-8")
    width, height = 240, 140
    pixels = b"".join(
        b"\x00" + bytes(channel for x in range(width) for channel in (40 + x // 2, 75 + y, 180))
        for y in range(height)
    )

    def chunk(kind, payload):
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))

    (directory / "Sample.png").write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(pixels))
        + chunk(b"IEND", b"")
    )
    return directory


def free_endpoint() -> str:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as endpoint:
        endpoint.bind(("127.0.0.1", 0))
        return f"127.0.0.1:{endpoint.getsockname()[1]}"


class Review:
    def __init__(self, arguments, data_root: Path, directory: Path):
        self.args = arguments
        self.output = arguments.output.resolve()
        self.address = free_endpoint()
        self.data_root = data_root
        self.directory = directory
        self.process = None
        self.command_modifier = "cmd" if sys.platform == "darwin" else "ctrl"
        self.report = {
            "status": "running", "address": self.address,
            "input": "native egui inspection protocol, semantic accessibility bounds",
            "scope": "Actual native framebuffer and application routing; OS input/services remain separate acceptance.",
            "isolated_storage": True, "fixture_only_operations": True,
            "platform": sys.platform, "checks": [], "screenshots": [], "steps": [],
            "errors": [], "app": str(arguments.app.resolve()),
            "client": str(arguments.client.resolve()),
        }
        self.output.mkdir(parents=True, exist_ok=True)
        self.save()

    def save(self):
        (self.output / "report.json").write_text(json.dumps(self.report, indent=2) + "\n", encoding="utf-8")

    def call(self, *arguments):
        self.report["steps"].append(list(map(str, arguments)))
        result = subprocess.run(
            [str(self.args.client.resolve()), "--addr", self.address, *map(str, arguments)],
            capture_output=True, text=True, timeout=25,
        )
        if result.returncode:
            raise RuntimeError(result.stderr.strip() or result.stdout.strip() or f"Inspection exited {result.returncode}")
        return json.loads(result.stdout)

    def wait(self, description, predicate, timeout=None):
        deadline = time.monotonic() + (timeout or self.args.timeout)
        last_error = None
        while time.monotonic() < deadline:
            if self.process is not None and self.process.poll() is not None:
                raise RuntimeError(f"Owned native process exited with {self.process.returncode}; see app.log")
            try:
                value = predicate()
                if value:
                    return value
            except (OSError, ValueError, RuntimeError) as error:
                last_error = str(error)
            time.sleep(0.05)
        raise RuntimeError(f"Timed out waiting for {description}; last error: {last_error}")

    def endpoint_open(self):
        host, port = self.address.rsplit(":", 1)
        try:
            with socket.create_connection((host, int(port)), timeout=0.15):
                return True
        except OSError:
            return False

    def tree(self, name=None):
        result = self.call("tree")
        if name:
            (self.output / f"{name}.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        accesskit = result["Tree"]["accesskit"]
        return [dict(node, inspection_id=ident, inspection_focused=ident == accesskit["focus"]) for ident, node in accesskit["nodes"]]

    @staticmethod
    def label(node):
        return node.get("properties", {}).get("label", "")

    def has_text(self, text):
        # Text-only egui labels expose AccessKit `value`, while interactive
        # widgets expose `label`. Never use values to locate clickable nodes:
        # label/textRun pairs can contain the same text at different bounds.
        return any(
            text in value
            for node in self.tree()
            for value in (node.get("properties", {}).get("label"), node.get("properties", {}).get("value"))
            if isinstance(value, str)
        )

    def wait_text(self, text):
        return self.wait(f"visible text {text!r}", lambda: self.has_text(text))

    def nodes(self, label, role=None, prefix=False):
        return [node for node in self.tree() if (self.label(node).startswith(label) if prefix else self.label(node) == label)
                and (role is None or node.get("role") == role)
                and "bounds" in node.get("properties", {})]

    def unique(self, label, role=None, prefix=False):
        found = self.nodes(label, role, prefix)
        if len(found) != 1:
            raise RuntimeError(f"Expected unique widget {label!r}; found {len(found)}")
        return found[0]

    def widget(self, label, role=None, prefix=False):
        return self.wait(f"widget {label!r}", lambda: self.unique(label, role, prefix))

    @staticmethod
    def center(node):
        bounds = node["properties"]["bounds"]
        point = ((bounds["x0"] + bounds["x1"]) / 2, (bounds["y0"] + bounds["y1"]) / 2)
        if not all(math.isfinite(value) for value in point) or bounds["x1"] <= bounds["x0"] or bounds["y1"] <= bounds["y0"]:
            raise RuntimeError("Widget has no finite clickable bounds")
        return point

    def click(self, label, role=None, prefix=False, secondary=False):
        self.widget(label, role, prefix)
        self.call("settle", 4)
        target = self.unique(label, role, prefix)
        self.call("context" if secondary else "click", *self.center(target))

    def key(self, chord):
        self.call("key", chord)

    def command(self, key):
        self.key(self.command_modifier + "+" + key)

    def text(self, text):
        self.call("text", text)

    def set_field(self, label, text):
        self.click(label, "textInput")
        self.command("a")
        self.text(str(text))

    def check(self, name, passed, detail):
        self.report["checks"].append({"name": name, "passed": bool(passed), "detail": detail})
        self.save()
        if not passed:
            raise RuntimeError(f"{name}: {detail}")
        print(f"PASS {name}", flush=True)

    def shot(self, name):
        self.call("settle", 8)
        path = self.output / f"{name}.png"
        result = self.call("screenshot", path)
        if not path.is_file() or path.stat().st_size < 100:
            raise RuntimeError(f"Missing native screenshot: {path}")
        self.report["screenshots"].append({"name": name, "path": str(path), "result": result})
        self.tree(name + "-tree")
        self.save()
        print(f"Captured {name}", flush=True)

    def observe_idle(self):
        if sys.platform != "linux" or self.process is None:
            self.report["idle_observation"] = {"available": False, "reason": "Linux /proc observation only"}
            self.save()
            return
        process_root = Path("/proc") / str(self.process.pid)

        def ticks():
            value = (process_root / "stat").read_text(encoding="utf-8")
            fields = value[value.rfind(")") + 2:].split()
            return int(fields[11]) + int(fields[12])

        try:
            # No inspection requests or screenshots during this quiet period/sample.
            time.sleep(1.0)
            frequency = os.sysconf("SC_CLK_TCK")
            before = ticks()
            started = time.monotonic()
            time.sleep(1.0)
            after = ticks()
            duration = time.monotonic() - started
            values = {}
            for line in (process_root / "status").read_text(encoding="utf-8").splitlines():
                if line.startswith(("VmRSS:", "VmHWM:")):
                    key, value = line.split(":", 1)
                    values[key] = int(value.strip().split()[0])
            self.report["idle_observation"] = {
                "available": True, "owned_pid": self.process.pid,
                "quiet_before_seconds": 1.0, "sample_seconds": round(duration, 3),
                "clock_ticks_per_second": frequency, "cpu_ticks": after - before,
                "cpu_percent_one_logical_core": round((after - before) * 100 / frequency / duration, 3),
                "rss_kib": values.get("VmRSS"), "peak_rss_kib": values.get("VmHWM"),
                "build_scope": "Development build observation; excludes startup, not release-performance acceptance",
                "requested_backend": os.environ.get("WGPU_BACKEND"),
                "software_graphics_requested": os.environ.get("LIBGL_ALWAYS_SOFTWARE") == "1",
            }
        except (OSError, ValueError, IndexError) as error:
            self.report["idle_observation"] = {"available": False, "reason": str(error)}
        self.save()

    def config(self):
        return tomllib.loads((self.data_root / "config.toml").read_text(encoding="utf-8"))

    def session(self):
        return json.loads((self.data_root / "session.json").read_text(encoding="utf-8"))

    def row(self, name):
        return self.widget(name + ", ", prefix=True)

    def select(self, name):
        self.click(name + ", ", prefix=True)

    def dismiss_notice(self):
        if self.nodes("Dismiss", "button"):
            self.click("Dismiss", "button")

    def close_sheet(self):
        self.key("Escape")
        self.call("settle", 4)

    def go(self, path, capture=None):
        self.command("l")
        self.widget("Location", "textInput")
        self.set_field("Location", path)
        if capture:
            self.shot(capture)
        self.key("Enter")
        self.wait("location editor to close", lambda: not self.nodes("Location", "textInput"))
        self.wait("saved active directory", lambda: Path(self.session()["tabs"][self.session()["active"]]["paths"][0]) == path)

    def scenario(self, name, callback):
        try:
            callback()
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, KeyError) as error:
            message = f"{name}: {error}"
            self.report["errors"].append(message)
            self.save()
            print(message, file=sys.stderr, flush=True)
            # Retain diagnostic evidence; never hide the original failed step.
            try:
                self.shot("failure-" + name)
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, KeyError):
                pass
            return False
        return True

    def run(self):
        app = self.args.app.resolve()
        client = self.args.client.resolve()
        if not app.is_file() or not client.is_file():
            raise FileNotFoundError("Build the inspection-enabled app and neptune-finder-inspect first")
        env = os.environ.copy()
        env["EGUI_INSPECTION"] = self.address
        with (self.output / "app.log").open("w", encoding="utf-8") as log:
            self.process = subprocess.Popen(
                [str(app), "--data-root", str(self.data_root), "--path", str(self.directory), "--no-restore", "--size", "1180x780"],
                env=env, stdout=log, stderr=subprocess.STDOUT,
            )
            self.report["owned_pid"] = self.process.pid
            self.save()
            try:
                self.wait("owned inspection listener", self.endpoint_open, timeout=30)
                self.report["info"] = self.call("info")
                self.row("Welcome.md")
                self.tree("00-initial-widget-tree")
                self.shot("00-list")
                self.observe_idle()
                if not self.scenario("views-and-search", self.views_and_search):
                    return
                if not self.scenario("previews", self.previews):
                    self.close_sheet()
                    self.go(self.directory)
                if not self.scenario("properties", self.properties):
                    self.close_sheet()
                    self.call("resize", 1180, 780)
                    self.go(self.directory)
                if not self.scenario("file-operations", self.file_operations):
                    self.close_sheet()
                    self.go(self.directory)
                elif not self.scenario("undo-and-redo", self.undo_and_redo):
                    self.close_sheet()
                    self.call("resize", 1180, 780)
                    self.go(self.directory)
                if not self.scenario("archives", self.archives):
                    self.close_sheet()
                    self.go(self.directory)
                self.scenario("preferences-and-themes", self.preferences)
                self.close_sheet()
                self.call("resize", 1180, 780)
                self.scenario("empty-and-error", self.empty_and_error)
            finally:
                self.stop()

    def views_and_search(self):
        labels = {self.label(node) for node in self.tree() if node.get("role") == "button"}
        required = {"Toggle sidebar", "Back", "Forward", "More actions", "New tab", "Close window", "Minimize", "Maximize"}
        self.check("named native chrome", required <= labels, f"Required controls missing: {sorted(required-labels)}")
        self.command("1")
        self.wait("icon view preference", lambda: self.config().get("view") == "grid")
        self.shot("01-icon-view")
        self.command("2")
        self.wait("list view preference", lambda: self.config().get("view") == "list")
        self.command("Backslash")
        self.wait("two independent panes", lambda: len(self.session()["tabs"][0]["paths"]) == 2)
        self.shot("02-split-view")
        self.command("Backslash")
        self.wait("single pane", lambda: len(self.session()["tabs"][0]["paths"]) == 1)
        self.command("t")
        self.wait("second tab", lambda: len(self.session()["tabs"]) == 2)
        self.shot("03-tabs")
        self.command("w")
        self.wait("first tab retained", lambda: len(self.session()["tabs"]) == 1)
        self.command("f")
        self.text("Welcome")
        self.wait("filtered filename", lambda: bool(self.nodes("Welcome.md, ", prefix=True)) and not self.nodes("Notes.txt, ", prefix=True))
        self.shot("04-folder-search")
        self.set_field("Search files", "Research")
        self.click("All subfolders")
        self.row("Research.txt")
        self.shot("05-recursive-search")
        self.check("recursive filename search", bool(self.nodes("Research.txt, ", prefix=True)), "Descendant fixture appears through the native search UI")
        self.go(self.directory)
        self.wait("ordinary directory after recursive search", lambda: bool(self.nodes("Notes.txt, ", prefix=True)) and not self.nodes("Research.txt, ", prefix=True))
        self.check("navigation leaves recursive results", not self.nodes("Research.txt, ", prefix=True), "Returning to a normal directory shows immediate children only")
        self.row("Welcome.md")
        self.select("Welcome.md")
        self.click("Welcome.md, ", prefix=True, secondary=True)
        self.widget("Move to Trash…")
        self.shot("06-context-menu")
        self.key("Escape")

    def previews(self):
        self.go(self.directory)
        self.select("Welcome.md")
        self.key("Space")
        self.widget("Close Quick Look", "button")
        self.wait_text("# Welcome to Neptune Finder")
        self.shot("07-text-quick-look")
        self.close_sheet()
        self.select("Sample.png")
        self.key("Space")
        self.widget("Close Quick Look", "button")
        self.wait("image preview to finish", lambda: not self.has_text("Preparing preview…"))
        self.shot("08-image-quick-look")
        self.close_sheet()

    def properties(self):
        self.go(self.directory)
        self.select("Welcome.md")
        self.key("alt+Enter")
        self.widget("Close properties", "button")
        self.wait_text("Kind: Markdown document")
        self.shot("08a-file-properties")
        self.close_sheet()
        self.click("Documents, ", prefix=True, secondary=True)
        self.click("Properties")
        self.widget("Close properties", "button")
        self.wait("finished folder count", lambda: self.has_text("1 item, totalling 25 B") and not self.has_text("still counting"))
        self.check("folder properties count nested items", self.has_text("1 item, totalling 25 B"), "Documents holds one 25-byte file in the fixture")
        self.shot("08b-folder-properties")
        self.close_sheet()
        self.key("Escape")
        self.command("i")
        self.widget("Close properties", "button")
        self.wait("finished current folder count", lambda: self.has_text("items, totalling") and not self.has_text("still counting"))
        self.shot("08c-current-folder-properties")
        self.call("resize", 640, 400)
        self.widget("Close properties", "button")
        self.shot("08d-narrow-properties")
        self.close_sheet()
        self.call("resize", 1180, 780)

    def file_operations(self):
        self.go(self.directory)
        self.command("shift+n")
        self.set_field("File or folder name", "Review created")
        self.shot("09-new-folder-dialog")
        self.click("Create", "button")
        created = self.directory / "Review created"
        self.wait("created fixture folder", created.is_dir)
        self.row(created.name)
        self.check("create folder", created.is_dir(), "Created only inside the run-owned fixture")
        self.dismiss_notice()
        self.shot("10-created-folder")
        self.select(created.name)
        self.key("F2")
        self.set_field("File or folder name", "Review renamed")
        self.shot("11-rename-dialog")
        self.click("Rename", "button")
        renamed = self.directory / "Review renamed"
        self.wait("renamed fixture folder", lambda: renamed.is_dir() and not created.exists())
        self.row(renamed.name)
        self.check("rename folder", renamed.is_dir() and not created.exists(), "Native action renamed the captured source in the fixture")
        self.dismiss_notice()
        self.shot("12-renamed-folder")
        self.select("Welcome.md")
        self.command("c")
        self.go(self.directory / "Destination")
        self.command("v")
        copied = self.directory / "Destination/Welcome.md"
        self.wait("copied fixture file", lambda: copied.is_file() and copied.read_bytes() == (self.directory / "Welcome.md").read_bytes())
        self.row("Welcome.md")
        self.check("copy via native file actions", copied.read_bytes() == (self.directory / "Welcome.md").read_bytes(), "Source remains present and destination bytes match")
        self.dismiss_notice()
        self.shot("13-copy-destination")
        self.go(self.directory)
        notes = self.directory / "Notes.txt"
        self.select(notes.name)
        self.command("x")
        self.shot("13a-cut-item")
        self.go(self.directory / "Projects")
        self.command("v")
        pasted = self.directory / "Projects" / notes.name
        self.wait("cut fixture file moved", lambda: pasted.is_file() and not notes.exists())
        self.row(notes.name)
        self.check("cut and paste moves", pasted.is_file() and not notes.exists(), "The pasted item left its original folder")
        self.dismiss_notice()
        self.go(self.directory)
        welcome = self.directory / "Welcome.md"
        source = self.row(welcome.name)["properties"]["bounds"]
        target = self.row("Pictures")["properties"]["bounds"]
        # Files are picked up by their icon or name, at the row's leading edge.
        grab = (source["x0"] + 30, (source["y0"] + source["y1"]) / 2)
        over = (target["x0"] + 30, (target["y0"] + target["y1"]) / 2)
        self.call("press", *grab)
        self.call("move", *over)
        self.shot("13b-dragging-onto-folder")
        self.call("release", *over)
        dropped = self.directory / "Pictures" / welcome.name
        self.wait("dragged fixture file moved", lambda: dropped.is_file() and not welcome.exists())
        self.check("drag onto a folder moves", dropped.is_file() and not welcome.exists(), "The dropped item left its original folder")
        self.dismiss_notice()
        self.select(renamed.name)
        self.key("Delete")
        self.widget("Move to Trash", "button")
        self.shot("14-trash-confirmation-cancelled")
        self.click("Cancel", "button")
        self.check("trash cancellation preserves fixture", renamed.is_dir(), "Confirmation was cancelled; no item was sent to the OS Trash")
        self.go(self.directory, capture="15-location-editor")

    def toggle_more_menu(self, shown):
        self.click("More actions", "button")
        self.wait("More menu " + ("open" if shown else "closed"), lambda: bool(self.nodes("About Neptune Finder")) == shown)

    def undo_and_redo(self):
        # The newest changes are the drag and the cut and paste. Undoing the
        # copy before them would send its copy to the OS Trash, so this stops
        # short of it.
        welcome, dropped = self.directory / "Welcome.md", self.directory / "Pictures/Welcome.md"
        notes, pasted = self.directory / "Notes.txt", self.directory / "Projects/Notes.txt"
        self.go(self.directory)
        self.toggle_more_menu(True)
        self.widget("Undo move", "button")
        self.check("More menu names the next Undo", not self.nodes("Redo move", "button"), "Only Undo is offered before anything is undone")
        self.shot("15g-more-menu-undo")
        self.toggle_more_menu(False)
        self.command("z")
        self.wait("dragged item put back", lambda: welcome.is_file() and not dropped.exists())
        self.wait_text("Undo Move")
        self.shot("15h-undo-chip")
        self.check("undo returns a dragged item", welcome.is_file() and not dropped.exists(), "Command+Z moved the dropped file back to its folder")
        self.command("z")
        self.wait("pasted item put back", lambda: notes.is_file() and not pasted.exists())
        self.check("undo returns a cut and pasted item", notes.is_file() and not pasted.exists(), "A second Command+Z reversed the cut and paste")
        rows = [self.row(name)["properties"]["bounds"] for name in ("Welcome.md", "Notes.txt", "Pictures")]
        below = max(bounds["y1"] for bounds in rows) + 80
        left = min(bounds["x0"] for bounds in rows)
        self.call("context", left + 260, below)
        self.widget("Undo copy", "button")
        self.widget("Redo move", "button")
        self.shot("15i-folder-menu-undo-redo")
        # A click outside closes the menu; the file list consumes Escape.
        self.call("click", left + 40, below)
        self.wait("folder menu closed", lambda: not self.nodes("Undo copy", "button"))
        self.call("resize", 640, 400)
        self.toggle_more_menu(True)
        self.widget("Redo move", "button")
        self.shot("15j-narrow-more-menu-undo-redo")
        self.toggle_more_menu(False)
        self.call("resize", 1180, 780)
        self.command("shift+z")
        self.wait("cut and paste redone", lambda: pasted.is_file() and not notes.exists())
        # Linux and Windows also redo with Ctrl+Y.
        self.command("shift+z" if sys.platform == "darwin" else "y")
        self.wait("drag redone", lambda: dropped.is_file() and not welcome.exists())
        self.check("redo moves both items again", pasted.is_file() and dropped.is_file(), "Redo repeated the cut and paste and the drag")
        self.command("shift+z")
        self.wait_text("Nothing to redo.")
        self.shot("15k-nothing-to-redo")
        self.dismiss_notice()

    def archives(self):
        self.go(self.directory)
        source = self.directory / "Documents"
        self.row(source.name)
        self.click(source.name + ", ", prefix=True, secondary=True)
        self.click("Compress…", "button")
        self.widget("Close compress dialog", "button")
        self.shot("15a-compress-dialog")
        self.call("resize", 480, 420)
        self.widget("Close compress dialog", "button")
        self.shot("15b-narrow-compress-dialog")
        self.call("resize", 1180, 780)
        self.click("Compress", "button")
        archive = self.directory / "Documents.zip"
        self.wait("compressed fixture archive", lambda: archive.is_file() and zipfile.is_zipfile(archive))
        with zipfile.ZipFile(archive) as contents:
            names = sorted(contents.namelist())
        self.check("compress via the context menu", names == ["Documents/", "Documents/Research.txt"], f"Archive entries: {names}")
        self.dismiss_notice()
        self.row(archive.name)
        self.click(archive.name + ", ", prefix=True, secondary=True)
        self.widget("Extract to…", "button")
        self.shot("15c-archive-context-menu")
        # A click outside closes the menu; the file list consumes Escape.
        self.select(archive.name)
        self.wait("archive context menu closed", lambda: not self.nodes("Extract to…", "button"))
        # egui counts clicks within 0.6 s of that one into a triple click.
        time.sleep(0.7)
        self.call("double-click", *self.center(self.row(archive.name)))
        extracted = self.directory / "Documents 2"
        self.wait("archive extracted beside itself", lambda: (extracted / "Research.txt").is_file())
        self.row(extracted.name)
        self.check(
            "double-click extracts without replacing",
            (extracted / "Research.txt").read_bytes() == (source / "Research.txt").read_bytes() and archive.is_file(),
            "The existing folder kept its name; the archive's copy took the next free name",
        )
        self.shot("15d-extracted-archive")
        self.dismiss_notice()
        self.click(archive.name + ", ", prefix=True, secondary=True)
        self.click("Extract to…", "button")
        self.set_field("Destination folder", self.directory / "Destination")
        self.shot("15e-extract-to-dialog")
        self.call("resize", 480, 420)
        self.widget("Close extract dialog", "button")
        self.shot("15f-narrow-extract-to-dialog")
        self.call("resize", 1180, 780)
        self.click("Extract", "button")
        moved = self.directory / "Destination/Documents/Research.txt"
        self.wait("archive extracted into the typed folder", moved.is_file)
        self.check("extract to a typed folder", moved.read_bytes() == (source / "Research.txt").read_bytes(), "Extract to… used the typed destination")
        self.dismiss_notice()

    def preferences(self):
        self.go(self.directory)
        self.command("comma")
        self.widget("Close preferences", "button")
        self.shot("16-preferences-graphite")
        self.click("Appearance")
        self.widget("Blue accent")
        self.shot("16-preferences-appearance")
        self.click("Files")
        self.widget("Show hidden files")
        self.shot("16-preferences-files")
        self.click("Appearance")
        self.call("resize", 640, 400)
        self.widget("Close preferences", "button")
        self.shot("17-narrow-preferences")
        self.close_sheet()
        self.shot("18-narrow-main")
        self.call("resize", 1180, 780)
        for theme in ("Light", "Dusk"):
            self.command("comma")
            self.click(theme)
            self.wait(f"{theme} theme preference", lambda: self.config().get("theme") == theme.lower())
            self.shot(f"19-{theme.lower()}-preferences")
            self.click("Close preferences", "button")
            self.shot(f"20-{theme.lower()}-main")
        self.check("theme controls update saved preferences", self.config().get("theme") == "dusk", "Theme changes used the native theme choices")
        self.click("More actions", "button")
        self.click("About Neptune Finder")
        self.widget("Close About Neptune Finder", "button")
        self.shot("21-about")
        self.close_sheet()

    def empty_and_error(self):
        self.go(self.directory / "Empty folder")
        self.wait_text("This folder is empty")
        self.shot("22-empty-directory")
        self.go(self.directory / "Unavailable location")
        self.widget("Try again", "button")
        self.shot("23-unavailable-directory")
        self.check("unavailable folder remains interactive", bool(self.nodes("Try again", "button")), "The owned missing directory shows a retry action")

    def stop(self):
        if self.process is None or self.process.poll() is not None:
            return
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--app", type=Path, default=REPO_ROOT / "target/debug" / ("neptune-finder" + SUFFIX))
    parser.add_argument("--client", type=Path, default=REPO_ROOT / "target/debug" / ("neptune-finder-inspect" + SUFFIX))
    parser.add_argument("--output", type=Path, default=REPO_ROOT / "artifacts/native-flow")
    parser.add_argument("--timeout", type=float, default=12.0, help="Per-condition wait budget in seconds")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    review = None
    with tempfile.TemporaryDirectory(prefix="neptune-finder-inspect-") as temporary:
        root = Path(temporary)
        review = Review(args, root / "data", fixture(root))
        if hasattr(signal, "SIGTERM"):
            signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt("Native review interrupted")))
        try:
            review.run()
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, KeyError, KeyboardInterrupt) as error:
            review.report["errors"].append(str(error))
            print(error, file=sys.stderr)
        finally:
            review.stop()
            review.report["status"] = "failed" if review.report["errors"] else "passed"
            review.save()
    print(f"Native flow report: {args.output / 'report.json'}", flush=True)
    return 0 if review.report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
