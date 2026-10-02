# End-to-end tests

`just test` runs the host unit tests (`just test-host`: pure logic, plain `cargo test`) and then the
QEMU tests (`just test-qemu`: the real kernel booted headless in QEMU and driven by typing on its
virtio keyboard, the way a user would). This directory is the QEMU side. `just test-qemu shell launch`
runs only the named modules; `just test-smoke` is a few seconds' worth of boot, pipe and redirect.

## How it works

- **The serial log is the transcript.** Every finished line, every program's output and, after each
  command, the shell's prompt are mirrored to the UART, which QEMU writes to a file. A command is done
  when the log ends with the prompt, so nothing waits on a timer. `Session.run(cmd)` types a command and
  returns what it printed (without the echoed line and the prompt); `Session.status(cmd)` adds its exit
  status; `start`/`finish` are for programs that read the keyboard.
- **One QEMU per module.** Each module in `cases/` is a *group*: its own QEMU, its own private copy of
  `system.img`, its own fresh HOME disk. Groups run in a thread pool at `cpus - 1` at a time
  (`QEMU_TEST_WORKERS=N` overrides), and results print afterwards in a fixed order. After QEMU has quit
  each group's `verify_disk` runs, and `fsck.fat -n` must call every image clean.
- **Fixtures** are set on the group's images with mtools before boot, through module attributes:
  `SYSTEM_FILES` and `HOME_FILES` (`{path: content}`; `None` removes a system file), `HOME = False` for no
  HOME disk. By default `/etc/environment` is `HOME=/root` and `PS1='\w> '` and the HOME disk is empty, so
  a test never depends on the image's own start-up files by accident (`startup.py` and friends are the
  exceptions that test those mechanisms).
- **Test-only programs** (`test/progs`, built by `just build-test-progs`): EL0 programs that make the
  system calls no shell command can make badly (an unknown number, a pointer outside the program, an
  fd that is not open) and that fault on purpose. A module lists the ones it wants in `TEST_PROGS`;
  they are put on its private image as `/tests/<name>` (not executable: the module `chmod +x`es them).
  They use `userlib` only for `_start` and exit and issue their own raw `svc`, so a wrapper that
  checked its arguments could not hide a kernel that did not. Never in `user/`, never on the shipped
  image. `syscalls.py` holds the table of what each bad call must return.
- **The boot transcript** (start-up notes, what `.profile` printed) is `Session.boot_log`.

## Writing a test

A module has `run(ctx)`; `ctx.s` is the session and `ctx.check(name, got, want)` records a result
(nothing aborts at the first failure). Check one behavior per line and word the name as the claim
(`"rm -f ignores a missing file"`). Take the expected output from what the program *should* print --
GNU's, or the project's documented rule in `plans/PROGS.md` -- not from whatever it printed the first
time. `test/try.py "cmd" ...` boots the kernel and shows what commands really print.

## Flakes

Keys typed in the first moments after a program starts can be lost (the virtio-input queue fills while
the guest is busy). Everything types one command at a time and waits for the prompt, so only code that
types *into a running program* is exposed: `interactive.py` waits `LAUNCH_SETTLE` before typing, runs
alone after the parallel groups (`EXCLUSIVE = True`) and gets one retry (`RETRIES = 1`). A retry that
helps is printed as a `NOTE`, so a flake is never silent. Untagged modules run once, at full speed. A
failed run can leave QEMU processes behind that slow the next one; the harness kills its own on every
exit path.
