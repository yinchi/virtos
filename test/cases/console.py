"""The console `ioctl`s (`TIOCGWINSZ`, `CONSOLE_READ_KEY`, `CONSOLE_DRAW`, `CONSOLE_CLEAR`) and the
console's alternate screen. `test/progs`' `console` makes each call and prints its result; frames
are inspected on the display with `Session.screendump`."""

from harness import Screen

TEST_PROGS = ["console"]

ENOENT, EBADF, EFAULT, EINVAL, ENOTTY = -2, -9, -14, -22, -25

# Addresses a program may not hand to the kernel (see `syscalls.py`).
LOW, KERNEL = "0x1000", "0x40000000"
USER_BASE = 0x4400_0000


def nonblank_rows(screen):
    return sum(1 for r in range(screen.rows) if not screen.blank_row(r))


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x /tests/console")
    c = "/tests/console"

    # ---------------------------------------------------------------- TIOCGWINSZ
    for fd in (0, 1, 2):
        check(f"fd {fd} reports 30 rows x 80 columns, no pixel size", s.run(f"{c} winsize {fd}"), "30 80 0 0\n")
    check("a closed fd is EBADF", s.run(f"{c} winsize 9"), f"{EBADF}\n")
    check("redirected stdin is not a terminal", s.run(f"{c} winsize 0 < /etc/environment"), f"{ENOTTY}\n")
    s.run(f"{c} winsize 1 > /tmp/ws")
    check("redirected stdout is not a terminal", s.run("cat /tmp/ws"), f"{ENOTTY}\n")
    check("stderr still answers when only stdout is redirected",
          s.run(f"{c} winsize 2 > /tmp/ws2"), "")  # prints nothing: stdout went to the file
    check("...and what went to the file was the size", s.run("cat /tmp/ws2"), "30 80 0 0\n")
    for name, addr in (("null", "0"), ("below the window", LOW), ("kernel memory", KERNEL),
                       ("the window's top", hex(USER_BASE + 0x200_0000)),
                       ("a wrapping address", hex(2**64 - 2))):
        check(f"a bad output pointer ({name}) is EFAULT", s.run(f"{c} winsize-ptr 1 {addr}"), f"{EFAULT}\n")

    # ---------------------------------------------------------------- requests and fds
    check("an unknown request is ENOTTY", s.run(f"{c} ioctl 1 4242"), f"{ENOTTY}\n")
    check("READ_KEY is not answered on stdout", s.run(f"{c} read-key 1"), f"{ENOTTY}\n")
    check("DRAW is not answered on stdin", s.run(f"{c} draw-stdin"), f"{ENOTTY}\n")
    check("a bad READ_KEY pointer is refused at once, not after a keypress",
          s.run(f"{c} read-key-ptr 0 {LOW}"), f"{EFAULT}\n")

    # ---------------------------------------------------------------- CONSOLE_READ_KEY
    def key(names, want, what):
        s.start(f"{c} read-key 0")
        s.keys(names)
        check(what, s.finish().split("\n", 1)[1], want)

    # evdev: A = 30, Left = 105; MOD_NUM (16) is set because NumLock starts on.
    key(["a"], "30 16 97\n", "a plain key: code, NumLock bit, character")
    key(["shift-a"], "30 17 65\n", "Shift sets its bit and capitalizes the character")
    key(["ctrl-left"], "105 18 0\n", "Ctrl+Left: the raw code, the Ctrl bit, no character")
    key(["ret"], "28 16 0\n", "Enter has no character")
    check("read(0) still works as a line afterwards", s.run("echo ok"), "ok\n")

    # ---------------------------------------------------------------- CONSOLE_DRAW
    check("a frame with the wrong size is EINVAL", s.run(f"{c} draw-bad-size"), f"{EINVAL}\n")
    check("a surrogate and a control character are each EINVAL", s.run(f"{c} draw-bad-char"), f"{EINVAL} {EINVAL}\n")
    check("a wide glyph in the last column is EINVAL", s.run(f"{c} draw-wide-edge"), f"{EINVAL}\n")
    check("a cursor past the console is clamped, not refused", s.run(f"{c} draw-cursor-oob"), "0\n")
    for name, addr in (("below the window", LOW), ("kernel memory", KERNEL), ("null", "0")):
        check(f"a bad frame pointer ({name}) is EFAULT", s.run(f"{c} draw-ptr {addr}"), f"{EFAULT}\n")

    # Recorded, not asserted: the cost of a full-frame redraw per keystroke (the design `edit`
    # relies on). Printed so a regression shows up in the transcript.
    print("   timing:", s.run(f"{c} draw-timing 1000").strip())

    # A frame is on screen while the program holds; attributes change what is drawn.
    s.run("clear")
    before = nonblank_rows(Screen(*s.screendump()))
    s.start(f"{c} draw-hold")
    # (the program blocks on a key now; give its frame time to appear)
    screen = Screen(*s.screendump())
    plain, inverse, dim = screen.cell(0, 0), screen.cell(1, 0), screen.cell(2, 0)
    check("the frame is drawn: all thirty rows hold something", nonblank_rows(screen), 30)
    check("an inverse cell is not the plain one", inverse != plain, True)
    check("a dim cell is not the plain one", dim != plain, True)
    check("an inverse cell is not a dim one", inverse != dim, True)
    s.keys(["a"])
    check("the call returned 0", s.finish().split("\n", 1)[1], "0\n")

    # ---------------------------------------------------------------- the alternate screen
    after = nonblank_rows(Screen(*s.screendump()))
    check("the screen from before the program is back: its own echo and the new prompt, no frame",
          after, before + 1)
    # --------------------------------------------------------------- CONSOLE_CLEAR
    s.run("echo something")
    s.run("clear")
    check("clear still empties the screen", nonblank_rows(Screen(*s.screendump())), 1)
    check("ioctl(1, CLEAR) works only on the console", s.run(f"{c} ioctl 0 1"), f"{ENOTTY}\n")
