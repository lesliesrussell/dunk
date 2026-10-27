"""End-to-end PTY smoke test; never invokes real Homebrew. Run after cargo build."""
import os
import codecs
import re
import pathlib
import pty
import select
import subprocess
import tempfile
import time


class Screen:
    def __init__(self):
        self.cells = [[" "] * 120 for _ in range(30)]
        self.row = self.col = 0
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            if self.pending.startswith("\x1b["):
                match = re.match(r"\x1b\[([0-9;?]*)([A-Za-z])", self.pending)
                if not match:
                    return
                args, command = match.groups()
                if command in ("H", "f"):
                    values = [int(v or 1) for v in args.split(";")]
                    self.row = values[0] - 1
                    self.col = (values[1] if len(values) > 1 else 1) - 1
                elif command == "J" and args == "2":
                    self.cells = [[" "] * 120 for _ in range(30)]
                self.pending = self.pending[match.end():]
            else:
                c, self.pending = self.pending[0], self.pending[1:]
                if c == "\r":
                    self.col = 0
                elif c == "\n":
                    self.row += 1
                elif c >= " " and 0 <= self.row < 30 and 0 <= self.col < 120:
                    self.cells[self.row][self.col] = c
                    self.col += 1

    def text(self):
        return "\n".join("".join(row) for row in self.cells)


def wait_for(needle, timeout=10):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if select.select([master], [], [], 0.1)[0]:
            try:
                screen.feed(os.read(master, 65536))
            except OSError:
                break
        if needle.decode() in screen.text():
            return
    raise AssertionError(f"Missing {needle!r}: {screen.text()}")


def confirm_and_wait():
    # Require a fresh Ready render, not the previous operation's idle status.
    screen.cells[27] = [" "] * 120
    os.write(master, b"y")
    wait_for(b"Ready")


with tempfile.TemporaryDirectory() as tmp:
    mock = pathlib.Path(tmp) / "brew"
    record = pathlib.Path(tmp) / "commands"
    mock.write_text('''#!/bin/sh
printf '%s\\n' "$*" >> "$DUNK_RECORD"
case "$1" in
formulae) printf 'alpha\\nbeta\\n' ;;
casks) printf 'gamma\\nomega\\n' ;;
info)
 if [ "$2" = '--json=v2' ]; then
  formula=''
  cask=''
  if [ -f "$DUNK_RECORD.tap" ]; then
   formula=',{"name":"widget","full_name":"acme/tools/widget","outdated":false}'
  fi
  if [ -f "$DUNK_RECORD.cask" ]; then
   cask=',{"token":"viewer","full_token":"acme/tools/viewer","outdated":false}'
  fi
  printf '{"formulae":[{"name":"beta","full_name":"beta","outdated":true,"desc":"Beta package"}%s],"casks":[{"token":"omega","outdated":true,"desc":"Omega cask"}%s]}\\n' "$formula" "$cask"
 else
  printf 'Package information and caveats\\n'
 fi ;;
install)
 case "$*" in
  'install acme/tools/widget') : > "$DUNK_RECORD.tap" ;;
  'install --cask acme/tools/viewer') : > "$DUNK_RECORD.cask" ;;
  'install acme/tools/fail') printf 'Package not found\\n' >&2; exit 1 ;;
 esac
 printf 'Operation completed\\n' ;;
upgrade|uninstall) printf 'Operation completed\\n' ;;
*) exit 1 ;;
esac
''')
    mock.chmod(0o755)
    env = dict(os.environ, DUNK_BREW=str(mock), DUNK_RECORD=str(record), TERM="xterm-256color")
    master, slave = pty.openpty()
    import fcntl
    import struct
    import termios
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
    proc = subprocess.Popen(["target/debug/dunk"], stdin=slave, stdout=slave, stderr=slave, env=env)
    os.close(slave)
    screen = Screen()
    try:
        wait_for(b"Ready")
        for label in [b"1: All", b"2: Installed", b"3: Outdated"]:
            wait_for(label)
        os.write(master, b"i")
        wait_for(b"Confirm:")
        os.write(master, b"n")
        time.sleep(0.15)
        assert "install --formula alpha" not in record.read_text()
        os.write(master, b"i")
        wait_for(b"Confirm:")
        confirm_and_wait()
        os.write(master, b"2u")
        wait_for(b"Confirm:")
        confirm_and_wait()
        # Mark both an installed formula and cask, then hide them with search.
        os.write(master, b" j /no-match\ru")
        wait_for(b"Upgrade 2 selected package(s)")
        wait_for(b"brew upgrade --formula beta")
        wait_for(b"brew upgrade --cask omega")
        # Nothing runs before confirmation, and cancellation leaves marks intact.
        before = record.read_text().splitlines()
        os.write(master, b"n")
        time.sleep(0.15)
        assert record.read_text().splitlines() == before
        os.write(master, b"u")
        wait_for(b"Confirm:")
        confirm_and_wait()
        commands = record.read_text().splitlines()
        assert commands.count("upgrade --formula beta") == 2, commands
        assert commands.count("upgrade --cask omega") == 1, commands
        # Upgrade all is global, even with no visible search matches.
        os.write(master, b"U")
        wait_for(b"Upgrade ALL installed")
        wait_for(b"Includes packages hidden by")
        os.write(master, b"n")
        time.sleep(0.15)
        assert "upgrade" not in record.read_text().splitlines()
        os.write(master, b"U")
        wait_for(b"Upgrade ALL installed")
        confirm_and_wait()
        assert record.read_text().splitlines().count("upgrade") == 1
        # Clear search and focus beta for the existing removal check.
        os.write(master, b"/\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f\r")
        os.write(master, b"X")
        wait_for(b"Confirm:")
        confirm_and_wait()
        os.write(master, b"1/gamma\ri")
        wait_for(b"Confirm:")
        confirm_and_wait()
        os.write(master, b"\r")
        wait_for(b"Package information and caveats")
        # Clear the earlier persistent marks before checking focused tap upgrades.
        os.write(master, b"2/\x7f\x7f\x7f\x7f\x7f\r j ")
        wait_for(b"0 marked")
        # Tap input is modal, validates input, and supports cancellation.
        before = record.read_text().splitlines()
        os.write(master, b"t\r")
        wait_for(b"Use owner/tap/package")
        os.write(master, b"garbage\x15acme/tools/widgetx\x7f\r")
        wait_for(b"Confirm: brew install acme/tools/widget")
        assert record.read_text().splitlines() == before
        os.write(master, b"n")
        time.sleep(0.15)
        assert record.read_text().splitlines() == before
        os.write(master, b"tacme/tools/widget\r")
        wait_for(b"Confirm: brew install acme/tools/widget")
        confirm_and_wait()
        assert "install acme/tools/widget" in record.read_text().splitlines()
        wait_for(b"Packages (1)")
        wait_for(b"acme/tools/widget")
        # The automatically focused installed package retains its tap identity.
        os.write(master, b"u")
        wait_for(b"brew upgrade --formula acme/tools/widget")
        os.write(master, b"n")
        time.sleep(0.15)
        # Explicit cask mode and full_token survive refresh and later upgrades.
        os.write(master, b"tacme/tools/viewer\t\t\r")
        wait_for(b"brew install --cask acme/tools/viewer")
        confirm_and_wait()
        wait_for(b"Packages (1)")
        os.write(master, b"u")
        wait_for(b"brew upgrade --cask acme/tools/viewer")
        os.write(master, b"n")
        time.sleep(0.15)
        # A failed Brew install is visible and releases the busy lock.
        os.write(master, b"tacme/tools/fail\r")
        wait_for(b"Confirm: brew install acme/tools/fail")
        confirm_and_wait()
        wait_for(b"Package not found")
        wait_for(b"acme/tools/viewer")
        os.write(master, b"t")
        wait_for(b"owner/tap/package, e.g.")
        os.write(master, b"\x1b")
        time.sleep(0.15)
        os.write(master, b"q")
        assert proc.wait(timeout=5) == 0
        commands = record.read_text()
        for command in ["install --formula alpha", "upgrade --formula beta", "uninstall --formula beta", "install --cask gamma", "info --cask gamma"]:
            assert command in commands, commands
        print("PTY smoke passed: cancel, install, focused/marked/all upgrades, uninstall, cask, details, tap installs/cancellation/errors, quit")
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        os.close(master)
