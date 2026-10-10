"""`edit FILE`, the full-screen editor: typing and deleting, `^S` save and `^O` save-as, `^X` exit
(asking first if the buffer is modified), the help screen, cut/copy/paste of lines and of a region,
search and go to line, the mark, the line-number gutter, the auto-indent toggle, the keypad, a file
bigger than the heap's first chunk, and that leaving restores the screen it took over.

`edit` owns the console while it runs, so a case starts it (`s.start`), sends keys with `s.keys`,
and ends it with `^X`, after which `s.finish()` returns at the shell's prompt. What ends up on disk
is checked by reading the file back with `cat`/`head`/`tail` -- exact and simple. Screen-only state
(the status bar, the help screen, the gutter, the cursor) is checked structurally from a display
dump: a cell is "inverse" when most of its pixels are the foreground colour, so the check does not
depend on which glyph is under it.
"""

from harness import Screen

ROWS, COLS = 30, 80
STATUS_ROW = 0  # the inverse-video header bar
TEXT_ROWS = range(1, ROWS - 3)  # the text area
MESSAGE_ROW = ROWS - 3  # a message or a prompt, blank otherwise
MENU_ROWS = (ROWS - 2, ROWS - 1)  # the key hints
FG = bytes((0x77, 0xFF, 0x77))
DIM_FG = bytes((0x3B, 0x7F, 0x3B))

SYSTEM_FILES = {
    "data/three.txt": "one\ntwo\nthree\n",
}


def pixels_of(screen, row, col):
    cell = screen.cell(row, col)
    return [cell[i:i + 3] for i in range(0, len(cell), 3)]


def is_inverse(screen, row, col):
    """Most of the cell's pixels are foreground (the cursor, the status bar)."""
    px = pixels_of(screen, row, col)
    return sum(p == FG or p == DIM_FG for p in px) > len(px) // 2


def status_bar_showing(screen):
    return all(is_inverse(screen, STATUS_ROW, c) for c in range(COLS))


def find_cursor(screen):
    """(row, col) of the one inverse cell in the text area or the message row (the header and the
    key hints have inverse cells of their own), or None."""
    for row in (*TEXT_ROWS, MESSAGE_ROW):
        for col in range(COLS):
            if is_inverse(screen, row, col):
                return (row, col)
    return None


def nonblank_rows(screen):
    return sum(1 for r in range(screen.rows) if not screen.blank_row(r))


def look(s):
    return Screen(*s.screendump())


def wait_editor(s, timeout=20):
    """Waits for the editor's first frame: there is no serial-side signal for it, so the display is
    the only evidence. Also leaves the guest time to get past the launch window where keys can be
    lost (`test/README.md`'s "Flakes")."""
    import time
    deadline = time.time() + timeout
    while time.time() < deadline:
        if status_bar_showing(look(s)):
            return
    raise TimeoutError("the editor never drew its status bar; log:\n" + s.log()[-600:])


def edit(s, path):
    s.start(f"edit {path}")
    wait_editor(s)


def quit_editor(s):
    s.keys(["ctrl-x"])
    return s.finish()


def save_and_quit(s):
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    return s.finish()


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("cd /tmp")

    # ------------------------------------------------------------------ typing and saving
    edit(s, "e1")
    s.type("hi\nthere")
    save_and_quit(s)
    check("typed text and Enter are saved", s.run("cat e1"), "hi\nthere\n")

    edit(s, "e2")
    s.type("abcd")
    s.keys(["left", "left", "ret"])
    s.type("X")
    s.keys(["backspace", "backspace"])  # the X, then the line break: the two halves rejoin
    s.type("e")
    save_and_quit(s)
    check("Backspace deletes and joins lines", s.run("cat e2"), "abecd\n")

    edit(s, "e3")
    s.type("a")
    s.keys(["tab"])
    s.type("b")
    s.keys(["home", "delete"])
    save_and_quit(s)
    check("Tab inserts a tab and Delete removes the character under the cursor", s.run("cat -T e3"), "^Ib\n")

    # ------------------------------------------------------------------ save as, exit prompts
    s.run("echo old > old")
    edit(s, "old")
    s.type("x")
    s.keys(["ctrl-o"])
    s.keys(["backspace"] * 3)
    s.type("new")
    s.keys(["ret"])
    quit_editor(s)
    check("^O saves under the name typed", s.run("cat new"), "xold\n")
    check("...and the original name is untouched", s.run("cat old"), "old\n")

    edit(s, "e5")
    s.type("z")
    s.keys(["ctrl-x"])
    check("^X on a modified buffer asks first (the cursor moves to the question)",
          find_cursor(look(s)) is not None, True)
    s.keys(["esc"])
    check("Esc cancels the question and editing goes on", status_bar_showing(look(s)), True)
    s.keys(["ctrl-x"])
    s.keys(["n"])
    s.finish()
    check("N discards the buffer", s.run("cat e5").endswith("No such file or directory\n"), True)
    edit(s, "e5")
    s.type("q")
    s.keys(["ctrl-x"])
    s.keys(["y"])
    s.finish()
    check("Y saves and exits", s.run("cat e5"), "q\n")

    edit(s, "none")
    quit_editor(s)
    check("quitting an untouched new buffer exits 0", s.run("echo $?"), "0\n")
    check("...and creates nothing", s.run("cat none").endswith("No such file or directory\n"), True)
    s.start("edit")
    check("no operand is an error", s.finish().endswith("missing operand\nTry 'edit --help' for more information.\n"), True)

    # ------------------------------------------------------------------ help and status
    edit(s, "e6")
    plain = look(s)
    check("the header bar is drawn across the whole top row", status_bar_showing(plain), True)
    check("the message row is blank until there is something to say", plain.blank_row(MESSAGE_ROW), True)
    check("both key-hint rows are drawn", [not plain.blank_row(r) for r in MENU_ROWS], [True, True])
    check("the hints start with an inverse key", is_inverse(plain, MENU_ROWS[0], 0), True)
    check("...followed by its plain label", is_inverse(plain, MENU_ROWS[0], 4), False)
    check("the cursor starts at the top left of the text area", find_cursor(plain), (1, 0))
    s.type("x")
    typed = look(s)
    check("typing changes the status bar ([Modified], the column)",
          typed.cell(STATUS_ROW, 1) != plain.cell(STATUS_ROW, 1)
          or any(typed.cell(STATUS_ROW, c) != plain.cell(STATUS_ROW, c) for c in range(COLS)), True)
    s.keys(["ctrl-g"])
    help_screen = look(s)
    check("^G replaces the screen with the help text (no header bar)", status_bar_showing(help_screen), False)
    check("...which has text on it", nonblank_rows(help_screen) > 10, True)
    s.keys(["a"])
    check("any key returns to the editor, and is not inserted", status_bar_showing(look(s)), True)
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.finish()
    check("the key that closed the help was not typed", s.run("cat e6"), "x\n")

    # ------------------------------------------------------------------ cut, copy, paste
    s.run("cp /data/three.txt c1")
    edit(s, "c1")
    s.keys(["ctrl-k", "down", "ctrl-u"])  # cut "one"; paste it above "three"
    save_and_quit(s)
    check("^K cuts a line and ^U pastes it above the cursor", s.run("cat c1"), "two\none\nthree\n")

    s.run("cp /data/three.txt c2")
    edit(s, "c2")
    s.keys(["alt-6", "down", "down", "ctrl-u"])  # copy "one"; paste above "three"
    save_and_quit(s)
    check("Alt+6 copies without removing", s.run("cat c2"), "one\ntwo\none\nthree\n")

    s.run("cp /data/three.txt c3")
    edit(s, "c3")
    s.keys(["ctrl-k", "ctrl-k", "ctrl-k"])  # three cuts in a row become one block
    s.type("z")
    s.keys(["ctrl-u"])
    save_and_quit(s)
    check("consecutive ^K cuts accumulate into one paste", s.run("cat c3"), "one\ntwo\nthree\nz\n")

    # ------------------------------------------------------------------ search, go to line
    s.run("seq 1 50 > n1")
    edit(s, "n1")
    s.keys(["alt-g"])
    s.type("30\n")
    s.type("X")
    save_and_quit(s)
    check("Alt+G jumps to a line", s.run("head -n 31 n1 | tail -n 2"), "X30\n31\n")

    s.run("seq 1 50 > n2")
    edit(s, "n2")
    s.keys(["ctrl-w"])
    s.type("20\n")
    s.type("Y")
    save_and_quit(s)
    check("^W finds the term and leaves the cursor after it", s.run("head -n 20 n2 | tail -n 1"), "20Y\n")

    s.run("seq 1 50 > n3")
    edit(s, "n3")
    s.keys(["alt-slash"])
    s.type("Z")
    s.keys(["alt-backslash"])
    s.type("A")
    save_and_quit(s)
    check("Alt+/ goes to the last line", s.run("tail -n 1 n3"), "50Z\n")
    check("Alt+\\ goes back to the first", s.run("head -n 1 n3"), "A1\n")

    # ------------------------------------------------------------------ the mark, the toggles
    s.run("echo hello world > m1")
    edit(s, "m1")
    s.keys(["alt-a"] + ["right"] * 5)
    marked = look(s)
    check("with a mark set the region is drawn inverse", is_inverse(marked, 1, 0) and is_inverse(marked, 1, 4), True)
    s.keys(["ctrl-k", "end", "ctrl-u"])
    save_and_quit(s)
    check("^K with a mark cuts the region and ^U pastes it as typed", s.run("cat m1"), " worldhello\n")

    edit(s, "m2")
    s.keys(["alt-i"])
    s.type("  a\nb")
    save_and_quit(s)
    check("Alt+I turns auto-indent on", s.run("cat m2"), "  a\n  b\n")

    edit(s, "m4")
    s.type("ab")
    s.keys(["tab"])
    s.type("c")
    s.keys(["alt-o"])
    s.type("d")
    s.keys(["tab"])
    s.type("e")
    save_and_quit(s)
    check("Alt+O: a Tab types a tab, then (toggled) spaces to the next stop, lined up as the tab",
          s.run("cat -T m4"), "ab^Icd  e\n")

    edit(s, "m5")
    s.keys(["alt-o"])
    s.type("ab")
    s.keys(["tab"])
    s.type("c")
    save_and_quit(s)
    check("with tabs to spaces on, Tab fills to the next stop", s.run("cat -T m5"), "ab  c\n")

    edit(s, "m3")
    start = find_cursor(look(s))
    s.keys(["alt-n"])
    gutter = find_cursor(look(s))
    check("Alt+N adds the line-number gutter, shifting the text by its width",
          (start, gutter), ((1, 0), (1, 2)))  # one digit and a separator
    s.keys(["alt-n"])
    check("...and removes it again", find_cursor(look(s)), (1, 0))
    quit_editor(s)

    # ------------------------------------------------------------------ the keypad
    edit(s, "k1")
    s.keys(["kp_4", "kp_5", "kp_enter", "kp_6"])  # NumLock is on at boot: digits, and Enter
    save_and_quit(s)
    check("the keypad types digits (NumLock on) and its Enter is Enter", s.run("cat k1"), "45\n6\n")

    edit(s, "k2")
    s.type("ab")
    s.keys(["num_lock", "kp_4"])  # NumLock off: 4 is Left
    s.type("X")
    s.keys(["num_lock"])
    save_and_quit(s)
    check("with NumLock off the keypad navigates", s.run("cat k2"), "aXb\n")

    # ------------------------------------------------------------------ a large file
    s.run("seq 1 100000 > big")
    edit(s, "big")
    s.keys(["alt-slash"])
    s.type("Z")
    save_and_quit(s)
    check("a file several times the size of the heap's first chunk loads and saves",
          s.run("tail -n 1 big"), "100000Z\n")
    check("...with every line before it intact", s.run("wc -l big"), "100000 big\n")

    # ------------------------------------------------------------------ the alternate screen
    s.run("clear")
    before = nonblank_rows(look(s))
    edit(s, "e6")
    s.keys(["down", "right"])
    during = nonblank_rows(look(s))
    quit_editor(s)
    after = nonblank_rows(look(s))
    check("while editing, the screen is the editor's", during > before + 1, True)
    check("after leaving, what was there before is back: the echoed command and the new prompt",
          after, before + 1)
    check("the shell works", s.run("echo ok"), "ok\n")
