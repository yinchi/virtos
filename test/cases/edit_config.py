"""`edit` and `$HOME/.editrc`: the settings it reads at start-up (`TABSIZE`, `LINENOS`,
`AUTOINDENT`)."""

from cases.edit import COLS, ROWS, STATUS_ROW, edit, find_cursor, look, quit_editor, save_and_quit  # noqa: F401

HOME_FILES = {".editrc": "TABSIZE=2\nLINENOS=1\nAUTOINDENT=1\nTABSTOSPACES=1\n"}


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("cd /tmp")

    edit(s, "c1")
    check("LINENOS=1 starts with the gutter on", find_cursor(look(s)), (1, 2))
    s.keys(["tab"])
    check("TABSIZE=2: Tab moves the cursor two columns", find_cursor(look(s)), (1, 4))
    s.type("a")
    s.keys(["ret"])
    s.type("b")
    save_and_quit(s)
    check("TABSTOSPACES=1: Tab typed spaces (two, to the stop)...", s.run("cat -T c1"), "  a\n  b\n")
    # (...and AUTOINDENT=1 copied that indent onto the second line.)
