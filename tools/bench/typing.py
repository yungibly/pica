#!/usr/bin/env python3
"""Keystroke latency in a real pseudo-terminal.

Each edit is timed from the write of its key bytes to the first output byte
and to the completed redraw: the end of the synchronized update that milli
wraps around every frame (CSI ? 2026 l). Typing, Enter, Backspace, and undo
are measured at the start, middle, and end of every file, on a scratch copy.

    cargo build --release
    python3 tools/bench/typing.py src/app.rs README.md \\
        docs/product-plan.md generated:1000 big.md

`generated:N` writes a deterministic N-line Markdown document. `--rss`
reports the resident set size after the edits. Standard library only;
Linux and macOS.
"""

import argparse
import os
import pty
import select
import shutil
import signal
import statistics
import struct
import sys
import tempfile
import time

try:
    import fcntl
    import termios
except ImportError:  # pragma: no cover
    sys.exit("This benchmark needs a POSIX pseudo-terminal")

FRAME_END = b"\x1b[?2026l"
KEYS = {
    "type": b"x",
    "backspace": b"\x7f",
    "enter": b"\r",
    "undo": b"\x1a",
}
CTRL_HOME = b"\x1b[1;5H"
CTRL_END = b"\x1b[1;5F"
ESC = b"\x1b"
# Go to line moved from Ctrl+G to Ctrl+L with the micro-aligned keymap.
GOTO_KEYS = [b"\x0c", b"\x07"]


def generated_markdown(lines):
    """Prose, lists, tasks, quotes, a table, and fenced code, repeating."""
    out = []
    section = 0
    while len(out) < lines:
        section += 1
        n = section
        out += [
            f"## Section {n}: notes on part {n}",
            "",
            f"Paragraph {n} has **bold**, *italic*, `inline code`, and a "
            f"[link](https://example.com/{n}) inside ordinary prose that is "
            "long enough to wrap at the default width of the live view.",
            "",
            f"- First item for {n} with a little text",
            f"- Second item with **emphasis** and `code`",
            f"  - Nested item {n}",
            f"- [ ] Open task {n}",
            f"- [x] Finished task {n}",
            "",
            f"> A quoted line {n} that also carries *emphasis* along.",
            "",
            "| Name | Value | Note |",
            "| --- | ---: | --- |",
            f"| alpha | {n} | first |",
            f"| beta | {n * 2} | second |",
            "",
            "```rust",
            f"fn section_{n}() -> usize {{",
            f"    let value = {n};",
            "    value * 2 // doubled",
            "}",
            "```",
            "",
        ]
    return "\n".join(out[:lines]) + "\n"


class Session:
    def __init__(self, binary, path, columns, rows, args):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
            os.environ["TERM"] = "xterm-256color"
            os.environ.pop("COLORTERM", None)
            os.execv(binary, [binary, "--no-state", *args, path])
        self.buffer = b""

    def read(self, timeout):
        ready, _, _ = select.select([self.fd], [], [], max(0.0, timeout))
        if not ready:
            return b""
        try:
            data = os.read(self.fd, 1 << 20)
        except OSError:
            return b""
        self.buffer += data
        return data

    def settle(self, quiet=0.03, limit=30.0):
        """Read until no output arrives for `quiet` seconds."""
        end = time.monotonic() + limit
        while time.monotonic() < end:
            if not self.read(quiet):
                return
        raise RuntimeError("output never settled")

    def wait_for(self, needle, limit=60.0):
        end = time.monotonic() + limit
        while needle not in self.buffer:
            if time.monotonic() > end:
                raise RuntimeError(f"timed out waiting for {needle!r}")
            self.read(end - time.monotonic())

    def press(self, data, limit=60.0):
        """Send one key; return (first byte, completed redraw) in seconds."""
        self.settle()
        self.buffer = b""
        start = time.perf_counter()
        os.write(self.fd, data)
        first = None
        end = time.monotonic() + limit
        while FRAME_END not in self.buffer:
            if time.monotonic() > end:
                raise RuntimeError(f"no redraw after {data!r}")
            if self.read(end - time.monotonic()) and first is None:
                first = time.perf_counter() - start
        done = time.perf_counter() - start
        return first, done

    def rss_mib(self):
        try:
            with open(f"/proc/{self.pid}/status") as status:
                for line in status:
                    if line.startswith("VmRSS"):
                        return int(line.split()[1]) / 1024
        except OSError:
            pass
        try:
            import subprocess

            out = subprocess.run(
                ["ps", "-o", "rss=", "-p", str(self.pid)],
                capture_output=True,
                text=True,
                check=True,
            )
            return int(out.stdout.strip()) / 1024
        except Exception:
            return None

    def close(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except OSError:
            pass
        os.close(self.fd)


def go_to_line(session, line):
    for key in GOTO_KEYS:
        session.settle()
        session.buffer = b""
        os.write(session.fd, key)
        session.settle(quiet=0.1)
        if b"Go to line" in session.buffer:
            session.settle()
            os.write(session.fd, str(line).encode())
            session.settle(quiet=0.05)
            os.write(session.fd, b"\r")
            session.settle(quiet=0.1)
            return True
        os.write(session.fd, ESC)
        session.settle(quiet=0.1)
    return False


def percentile(values, fraction):
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round(fraction * (len(ordered) - 1))))
    return ordered[index]


def describe(values):
    return f"{percentile(values, 0.5) * 1000:6.2f} {percentile(values, 0.95) * 1000:6.2f}"


def measure(binary, path, label, args, ops, positions, samples, warmup, size, rss):
    columns, rows = size
    with open(path, "rb") as source:
        lines = source.read().count(b"\n") + 1
    session = Session(binary, path, columns, rows, args)
    results = []
    try:
        session.wait_for(FRAME_END, limit=120)
        session.settle(quiet=0.2)
        for position in positions:
            if position == "start":
                session.press(CTRL_HOME)
            elif position == "end":
                session.press(CTRL_END)
            else:
                if not go_to_line(session, max(1, lines // 2)):
                    print(f"{label}: no go-to-line key; skipped middle", file=sys.stderr)
                    continue
            timings = {op: ([], []) for op in ops}
            for iteration in range(warmup + samples):
                # Each round leaves the text as it found it.
                for op in ops:
                    first, done = session.press(KEYS[op])
                    if iteration >= warmup:
                        timings[op][0].append(first)
                        timings[op][1].append(done)
            for op in ops:
                results.append((label, position, op, *timings[op]))
        memory = session.rss_mib() if rss else None
    finally:
        session.close()
    return results, memory


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--binary", help="default: target/release/pica, else target/release/milli"
    )
    parser.add_argument("--size", default="120x40", help="COLUMNSxROWS")
    parser.add_argument("--samples", type=int, default=15, help="rounds per position")
    parser.add_argument("--warmup", type=int, default=2)
    parser.add_argument("--positions", default="start,middle,end")
    parser.add_argument("--ops", default="type,backspace,enter,undo")
    parser.add_argument("--source", action="store_true", help="open with --source")
    parser.add_argument("--rss", action="store_true", help="report resident memory")
    parser.add_argument("--detail", action="store_true", help="one row per operation")
    parser.add_argument("files", nargs="+")
    options = parser.parse_args()
    columns, rows = (int(part) for part in options.size.lower().split("x"))
    binary = options.binary or next(
        (
            path
            for path in ("target/release/pica", "target/release/milli")
            if os.path.exists(path)
        ),
        "target/release/milli",
    )
    binary = os.path.abspath(binary)
    ops = options.ops.split(",")
    positions = options.positions.split(",")
    scratch = tempfile.mkdtemp(prefix="milli-bench-")
    print(f"binary {binary} · {columns}x{rows} · {options.samples} rounds · ms p50 p95")
    print(f"{'file':<22}{'where':<8}{'operation':<11}{'first byte':>14}{'redraw':>14}")
    try:
        for name in options.files:
            if name.startswith("generated:"):
                count = int(name.split(":", 1)[1])
                label = f"generated-{count}.md"
                path = os.path.join(scratch, label)
                with open(path, "w") as out:
                    out.write(generated_markdown(count))
            else:
                label = os.path.basename(name)
                path = os.path.join(scratch, label)
                shutil.copyfile(name, path)
            args = ["--source"] if options.source else []
            results, memory = measure(
                binary,
                path,
                label,
                args,
                ops,
                positions,
                options.samples,
                options.warmup,
                (columns, rows),
                options.rss,
            )
            every_first, every_done = [], []
            for label_, position, op, first, done in results:
                every_first += first
                every_done += done
                if options.detail:
                    print(f"{label_:<22}{position:<8}{op:<11}{describe(first):>14}{describe(done):>14}")
            if not options.detail:
                for position in positions:
                    first = [t for r in results if r[1] == position for t in r[3]]
                    done = [t for r in results if r[1] == position for t in r[4]]
                    if first:
                        print(f"{label:<22}{position:<8}{'all':<11}{describe(first):>14}{describe(done):>14}")
            if every_first:
                print(f"{label:<22}{'all':<8}{'all':<11}{describe(every_first):>14}{describe(every_done):>14}")
            if memory is not None:
                print(f"{label:<22}rss {memory:.1f} MiB")
            sys.stdout.flush()
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    main()
