"""Typing at the keyboard: stdin typed into a running program (a line at a time, Ctrl+D for end of
file) and the prompt's line editor (cursor keys, Backspace, Delete, Ctrl+A/E/K/U/W, history).

These race real key presses against the guest, and keys typed in the first moments after a
program starts can be lost (the virtio-input queue overflows while the guest is busy loading
it), so this group runs alone (`EXCLUSIVE`), waits `LAUNCH_SETTLE` before typing into a program,
and gets one retry (`RETRIES`) -- a retry is reported, so a flake is never silent."""

import time

from harness import (BACKSPACE, CTRL_A, CTRL_D, CTRL_E, CTRL_K, CTRL_U, CTRL_W, DELETE, DOWN, END,
                     ENTER, HOME, LEFT, RIGHT, UP)

EXCLUSIVE = True
RETRIES = 1
LAUNCH_SETTLE = 1.0


def run(ctx):
    s, check = ctx.s, ctx.check

    def into_program(command, *typed, eof_after=1):
        """Starts `command`, types each of `typed` (text, or a list of keys) into it, presses
        Ctrl+D `eof_after` times, and returns the transcript up to the next prompt."""
        s.start(command)
        time.sleep(LAUNCH_SETTLE)
        for item in typed:
            if isinstance(item, list):
                s.keys(item)
            else:
                s.type(item)
            time.sleep(0.2)
        for _ in range(eof_after):
            s.keys([CTRL_D])
            time.sleep(0.2)
        return s.finish()

    # ---------------------------------------------------------------- stdin of a program
    check("cat echoes typed lines (the typed line, then cat's output)",
          into_program("cat", "hello\n", "world\n"), "cat\nhello\nhello\nworld\nworld\n")
    check("sort reads typed lines until Ctrl+D",
          into_program("sort", "pear\n", "apple\n"), "sort\npear\napple\napple\npear\n")
    check("wc counts typed input", into_program("wc", "one two\n", "three\n"), "wc\none two\nthree\n2 3 14\n")
    check("Ctrl+D on a non-empty line delivers it unterminated; a second ends the input",
          into_program("cat", "ab", eof_after=2), "cat\nabab")
    check("Backspace edits the line a program will read",
          into_program("cat", "abx", [BACKSPACE], "c\n"), "cat\nabc\nabc\n")
    check("the shell is intact afterwards", s.run("echo alive"), "alive\n")

    # ---------------------------------------------------------------- the prompt's line editor
    def edit(*steps):
        """Performs `steps` (text to type, or a list of keys to press), presses Enter, and returns
        what the resulting command printed."""
        for step in steps:
            if isinstance(step, list):
                s.keys(step)
            else:
                s.type(step)
        s.keys([ENTER])
        return s.wait_prompt().partition("\n")[2]

    check("Backspace erases before the cursor", edit("echo abx", [BACKSPACE], "c"), "abc\n")
    check("typing inserts at the cursor", edit("echo ac", [LEFT], "b"), "abc\n")
    check("Left and Right move", edit("echo abc", [LEFT, LEFT, RIGHT], "X"), "abXc\n")
    check("Home goes to the start", edit("cho x", [HOME], "e"), "x\n")
    check("End goes to the end", edit("echo a", [HOME, END], "b"), "ab\n")
    check("Ctrl+A and Ctrl+E are Home and End", edit("cho z", [CTRL_A], "e", [CTRL_E], "!"), "z!\n")
    check("Delete removes under the cursor", edit("echo abc", [LEFT, LEFT, DELETE]), "ac\n")
    check("Ctrl+K kills to the end", edit("echo abXXX", [LEFT, LEFT, LEFT, CTRL_K]), "ab\n")
    check("Ctrl+U kills to the start", edit("garbage", [CTRL_U], "echo ok"), "ok\n")
    check("Ctrl+W kills a word", edit("echo one two", [CTRL_W], "three"), "one three\n")
    check("Backspace at the start does nothing", edit([BACKSPACE, BACKSPACE], "echo fine"), "fine\n")
    check("Ctrl+D at the prompt does nothing", edit([CTRL_D], "echo still"), "still\n")

    # ---------------------------------------------------------------- history
    s.run("echo first")
    s.run("echo second")
    check("Up recalls the last command", edit([UP]), "second\n")
    check("Up twice recalls the one before (a repeat of the last was not recorded again)", edit([UP, UP]), "first\n")
    check("Down walks forward again (UP, UP, DOWN lands on the newest)", edit([UP, UP, DOWN]), "first\n")
    check("Down past the newest restores the line being typed", edit("echo draft", [UP, DOWN]), "draft\n")
    s.run("")
    s.run("echo after-empty")
    check("an empty line is not recorded in the history", edit([UP]), "after-empty\n")
