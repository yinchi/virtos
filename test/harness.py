"""The QEMU test harness: boots the kernel headless and drives it the way a user would -- typing
on the virtio keyboard through the QEMU monitor's `sendkey` -- so a test case can check what each
command printed (the serial log mirrors the console: finished lines, program output and, after
each command, the prompt) and, once QEMU has exited, what ended up on the disks.

Test cases live in `cases/`; `run_tests.py` runs them. Any kernel panic on the serial log fails a
session at once, wherever it shows up (`Session.wait_until`).
"""

import os
import re
import socket
import subprocess
import time

# Seconds between keys. The guest drains its keyboard queue at roughly 30 ms a key; a 16-slot
# queue overflows if keys arrive much faster. Commands are typed one at a time and the next one
# waits for the prompt, so nothing is ever typed ahead of the shell.
KEY_DELAY = 0.03
TIMEOUT = 40  # seconds to wait for a prompt or text before giving up
# Programs that embed the time-zone database are 1.3 MB and take a few seconds to load; give a
# command that runs one this long.
SLOW = 90

# QEMU's key names for the characters that are not their own name (a US keyboard).
KEY_NAMES = {
    " ": "spc", "\n": "ret", "\t": "tab",
    "!": "shift-1", '"': "shift-apostrophe", "#": "shift-3", "$": "shift-4", "%": "shift-5",
    "&": "shift-7", "'": "apostrophe", "(": "shift-9", ")": "shift-0", "*": "shift-8",
    "+": "shift-equal", ",": "comma", "-": "minus", ".": "dot", "/": "slash",
    ":": "shift-semicolon", ";": "semicolon", "<": "shift-comma", "=": "equal",
    ">": "shift-dot", "?": "shift-slash", "@": "shift-2", "[": "bracket_left",
    "\\": "backslash", "]": "bracket_right", "^": "shift-6", "_": "shift-minus",
    "`": "grave_accent", "{": "shift-bracket_left", "|": "shift-backslash",
    "}": "shift-bracket_right", "~": "shift-grave_accent",
}

# Non-text keys for `Session.keys`.
UP, DOWN, LEFT, RIGHT = "up", "down", "left", "right"
HOME, END, DELETE, BACKSPACE, ENTER = "home", "end", "delete", "backspace", "ret"
CTRL_A, CTRL_D, CTRL_E, CTRL_H, CTRL_K, CTRL_U, CTRL_W = (
    "ctrl-a", "ctrl-d", "ctrl-e", "ctrl-h", "ctrl-k", "ctrl-u", "ctrl-w")

# What ends the serial log when the kernel has died -- never legitimate output of any test.
FATAL_MARKERS = ("Panic occurred!", "Unexpected exception", "Kernel Panic")

# The shell prompt: a working directory (an absolute path), then `> ` (the tests' own `PS1`, see
# `DEFAULT_ENVIRONMENT`) or `# ` (the shell's default, `\w\$`: there is only root), at the very
# end of the log.
PROMPT = re.compile(r"(/[^\s>#$]*)[>#$] $")

DEFAULT_ENVIRONMENT = "HOME=/root\nPS1='\\w> '\n"


class Session:
    def __init__(self, elf, system_img, home_img, workdir, expect_boot=True, extra_imgs=()):
        self.serial_path = os.path.join(workdir, "serial.log")
        self.sock_path = os.path.join(workdir, "mon.sock")
        open(self.serial_path, "w").close()
        drives = []
        # The home disk first, then the system disk, as `just run` does (the kernel names a
        # volume by its label, so the order does not matter).
        for n, path in enumerate([p for p in (home_img, *extra_imgs, system_img) if p]):
            drives += ["-drive", f"file={path},if=none,format=raw,id=d{n}",
                       "-device", f"virtio-blk-device,drive=d{n}"]
        self.qemu = subprocess.Popen(
            ["qemu-system-aarch64", "-machine", "virt", "-cpu", "max", "-nodefaults",
             "-display", "none",
             "-serial", f"file:{self.serial_path}",
             "-monitor", f"unix:{self.sock_path},server,nowait",
             "-device", "virtio-gpu-device", "-device", "virtio-keyboard-device",
             *drives, "-kernel", elf],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        self.sock = socket.socket(socket.AF_UNIX)
        deadline = time.time() + TIMEOUT
        while True:
            try:
                self.sock.connect(self.sock_path)
                break
            except (FileNotFoundError, ConnectionRefusedError):
                if time.time() > deadline or self.qemu.poll() is not None:
                    raise RuntimeError("QEMU did not start: " + self._stderr())
                time.sleep(0.1)
        time.sleep(0.3)
        self.sock.recv(4096)  # the monitor's banner
        self.pos = 0
        self.boot_log = ""
        if expect_boot:
            self.wait_prompt()  # boot finished: the first prompt is on the serial log
            self.boot_log = self.log()

    def _stderr(self):
        if self.qemu.poll() is None:
            return ""
        return self.qemu.stderr.read().decode(errors="replace")

    def log(self):
        """The serial log so far, `\\r\\n` as `\\n`."""
        with open(self.serial_path, "rb") as f:
            return f.read().decode(errors="replace").replace("\r\n", "\n")

    def check_alive(self):
        """Raises if the kernel has panicked or QEMU has exited."""
        log = self.log()
        for marker in FATAL_MARKERS:
            if marker in log:
                raise RuntimeError(f"kernel died ({marker!r}):\n{log[log.index(marker):]}")
        if self.qemu.poll() is not None:
            raise RuntimeError("QEMU exited: " + self._stderr())

    # -------------------------------------------------------------------- input

    def keys(self, names):
        for name in names:
            self.sock.sendall(f"sendkey {name} 20\n".encode())
            time.sleep(KEY_DELAY)

    def type(self, text):
        for c in text:
            if c.isupper():
                self.keys(["shift-" + c.lower()])
            elif c.isalnum():
                self.keys([c])
            else:
                self.keys([KEY_NAMES[c]])

    # ------------------------------------------------------------------- waiting

    def wait_until(self, pred, what, timeout=TIMEOUT):
        """Waits until `pred(text since the checkpoint)` holds; returns that text."""
        deadline = time.time() + timeout
        while True:
            self.check_alive()
            text = self.log()[self.pos:]
            if pred(text):
                return text
            if time.time() > deadline:
                raise TimeoutError(f"timed out waiting for {what}; log since the checkpoint:\n{text!r}")
            time.sleep(0.05)

    def wait_for_log(self, text, timeout=TIMEOUT):
        """Waits for `text` to appear anywhere in the serial log, panics included -- for a test of
        a boot that is meant to fail. Returns the whole log; raises `TimeoutError` if it never does."""
        deadline = time.time() + timeout
        while text not in self.log():
            if time.time() > deadline:
                raise TimeoutError(f"{text!r} never appeared; log:\n{self.log()!r}")
            time.sleep(0.05)
        return self.log()

    def wait_prompt(self, timeout=TIMEOUT):
        """Waits for the shell's prompt at the end of the log; returns the text since the
        checkpoint without it, and moves the checkpoint past it."""
        text = self.wait_until(lambda t: PROMPT.search(t) is not None, "the prompt", timeout)
        self.pos += len(text)
        return text[:PROMPT.search(text).start()]

    # ------------------------------------------------------------------- the display

    def screendump(self, settle=0.3, timeout=10):
        """The display as it is now: `(width, height, rgb)` where `rgb` is the P6 PPM's pixel
        bytes. The serial log is written as the guest draws, but the display is flushed a moment
        after; so the dump is taken again until two in a row agree (`settle` seconds apart)."""
        path = self.sock_path + ".ppm"
        deadline = time.time() + timeout
        previous = None
        while True:
            if os.path.exists(path):
                os.remove(path)
            self.sock.sendall(f"screendump {path}\n".encode())
            while not os.path.exists(path):
                if time.time() > deadline:
                    raise TimeoutError("the screendump never appeared")
                time.sleep(0.02)
            time.sleep(0.05)  # let QEMU finish writing it
            with open(path, "rb") as f:
                data = f.read()
            if data == previous:
                break
            if time.time() > deadline:
                raise TimeoutError("the display never settled")
            previous = data
            time.sleep(settle)
        header, rest = data.split(b"\n", 1)
        assert header == b"P6", header
        dims, rest = rest.split(b"\n", 1)
        width, height = (int(n) for n in dims.split())
        _maxval, pixels = rest.split(b"\n", 1)
        return width, height, pixels

    # ------------------------------------------------------------------- running

    def run(self, command, timeout=TIMEOUT):
        """Types `command`, waits for the next prompt; returns what the command printed (the
        echoed command line and the prompt are not part of it)."""
        self.type(command + "\n")
        text = self.wait_prompt(timeout)
        _, _, output = text.partition("\n")  # the first line is the typed command, echoed
        return output

    def start(self, command):
        """Types `command` and returns at once, for a program that reads the keyboard: `type` its
        input, then `finish()`."""
        self.type(command + "\n")

    def finish(self, timeout=TIMEOUT):
        """The rest of what `start` began: the transcript up to the next prompt (the echoed
        command line, then everything typed and printed since)."""
        return self.wait_prompt(timeout)

    def status(self, command, timeout=TIMEOUT):
        """Runs `command`, then `echo $?`: returns `(output, status)`."""
        output = self.run(command, timeout)
        return output, int(self.run("echo $?").strip())

    def close(self):
        try:
            self.sock.sendall(b"quit\n")
        except OSError:
            pass
        try:
            self.qemu.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.qemu.kill()
            self.qemu.wait()


# ------------------------------------------------------------------------ disk fixtures
# Edits to a group's own private copy of an image, made with mtools before QEMU boots (or after
# it has quit): never to an image QEMU has open. Every call has no stdin and a session of its own
# (no controlling terminal): mtools asks on /dev/tty, not stdin, what to do about a name that
# exists already (`mmd` of an existing directory), and a suite run from a terminal then waits
# forever for an answer.

def _mtools(*args):
    subprocess.run(args, check=True, capture_output=True, stdin=subprocess.DEVNULL, start_new_session=True)


def put_file(img, path, content):
    """Puts `content` (str or bytes) at `path` on `img`, creating parent directories; `None`
    removes the file."""
    if content is None:
        subprocess.run(["mdel", "-i", img, f"::{path}"], capture_output=True, stdin=subprocess.DEVNULL, start_new_session=True)  # absent is fine
        return
    parts = path.strip("/").split("/")
    for i in range(1, len(parts)):
        subprocess.run(["mmd", "-i", img, "::" + "/".join(parts[:i])], capture_output=True, stdin=subprocess.DEVNULL, start_new_session=True)  # exists is fine
    local = img + ".stage"
    with open(local, "wb") as f:
        f.write(content.encode() if isinstance(content, str) else content)
    _mtools("mcopy", "-o", "-i", img, local, f"::{path}")
    os.remove(local)


def get_file(img, path):
    """The bytes of `path` on `img`, or None if it is not there."""
    r = subprocess.run(["mcopy", "-i", img, f"::{path}", "-"], capture_output=True, stdin=subprocess.DEVNULL, start_new_session=True)
    return r.stdout if r.returncode == 0 else None


def relabel(img, label):
    """Gives `img`'s FAT volume a new label (in the boot sector, where the kernel reads it)."""
    _mtools("mlabel", "-i", img, f"::{label}")


def make_home_image(mkimage, path, workdir, files, label="HOME", volume_id="5e6f7a8b"):
    """A fresh volume (label `label`, ID `volume_id`: 8 hex digits, 5E6F-7A8B by default) holding
    `files` ({path: content})."""
    seed = os.path.join(workdir, "home-seed")
    os.makedirs(seed, exist_ok=True)
    for name, content in files.items():
        target = os.path.join(seed, name)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target, "wb") as f:
            f.write(content.encode() if isinstance(content, str) else content)
    subprocess.run([mkimage, path, "64M", label, volume_id, seed], check=True, capture_output=True, stdin=subprocess.DEVNULL, start_new_session=True)


class Screen:
    """A display captured by `Session.screendump`, in 8x16-pixel character cells."""

    CELL_W, CELL_H = 8, 16

    def __init__(self, width, height, pixels):
        self.width, self.height, self.pixels = width, height, pixels
        self.cols, self.rows = width // self.CELL_W, height // self.CELL_H

    def cell(self, row, col):
        """The pixels of one cell, as bytes (row by row)."""
        out = bytearray()
        for y in range(row * self.CELL_H, (row + 1) * self.CELL_H):
            start = (y * self.width + col * self.CELL_W) * 3
            out += self.pixels[start:start + self.CELL_W * 3]
        return bytes(out)

    def drawn(self, row, col):
        """Whether anything but background is in the cell."""
        return any(self.cell(row, col))

    def blank_row(self, row):
        return not any(self.drawn(row, c) for c in range(self.cols))


class Context:
    """What a case module's `run` and `verify_disk` get."""

    def __init__(self, session, check, workdir, system_img, home_img, extra_imgs=()):
        self.s = session
        self.check = check
        self.workdir = workdir
        self.system_img = system_img
        self.home_img = home_img
        self.extra_imgs = list(extra_imgs)
