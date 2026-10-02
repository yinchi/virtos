#!/usr/bin/env python3
"""Boots virtos and runs the given command lines, printing each one's output -- for finding out
what the shell really prints before writing a check for it.

Usage: try.py [--home | --no-home] COMMAND...   (paths as for `just test-qemu`: run it from the
repository root after `just build system`; set MKIMAGE if it is not at its default path)
"""

import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import harness  # noqa: E402

args = sys.argv[1:]
with_home = "--no-home" not in args
args = [a for a in args if a not in ("--home", "--no-home")]
mkimage = os.environ.get("MKIMAGE", "tools/mkimage/target/aarch64-unknown-linux-gnu/release/mkimage")
work = tempfile.mkdtemp(prefix="virtos-try-")
system = os.path.join(work, "system.img")
subprocess.run(["cp", "--sparse=always", "system.img", system], check=True)
harness.put_file(system, "etc/environment", harness.DEFAULT_ENVIRONMENT)
home = None
if with_home:
    home = os.path.join(work, "home.img")
    harness.make_home_image(mkimage, home, work, {})
s = harness.Session("virtos.elf", system, home, work)
try:
    for command in args:
        print(f"$ {command}")
        print(s.run(command, timeout=harness.SLOW), end="")
finally:
    s.close()
