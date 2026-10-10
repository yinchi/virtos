"""Output in columns: `ls` on the terminal (and `-C`/`-x`/`-1`), and `column` (fill and `-t`).
The console is 80 columns wide, so a layout is a plain function of the names: the expected text
is written out, or computed from the same rule (GNU `ls`'s: the most columns that fit, each as
wide as its widest entry, two spaces between). `ls | cat` is one per line, as on any pipe."""

NAMES = [f"f{n:02}" for n in range(1, 21)]
WIDE = ["日" * 20 + "1", "日" * 20 + "2"]  # 41 columns each, 21 characters each

SYSTEM_FILES = {
    **{f"data/cols/{name}": "x" for name in NAMES},
    **{f"data/wide/{name}": "x" for name in WIDE},
    # Sixteen directories: 3 columns each (78 wide in one row), but `-F` makes them 4 (94: two rows).
    **{f"data/dirs/d{n:02}/x": "x" for n in range(1, 17)},
    "data/list20.txt": "".join(f"{name}\n" for name in NAMES),
    "data/table.txt": "name size\nalpha 1\n\n  b   22 extra\n",
    "data/colon.txt": "a:b::c\nxx:y:z:w\n",
}


def reference(names, width, across=False):
    """The layout, computed by an independent, brute-force reading of the rule."""
    best = None
    for per_line in range(len(names), 0, -1):
        if across:
            rows = [names[i:i + per_line] for i in range(0, len(names), per_line)]
            cols = [[r[c] for r in rows if c < len(r)] for c in range(per_line)]
        else:
            nrows = -(-len(names) // per_line)
            cols = [names[i:i + nrows] for i in range(0, len(names), nrows)]
            rows = [[c[r] for c in cols if r < len(c)] for r in range(nrows)]
        widths = [max(len(x) for x in c) for c in cols if c]
        if sum(widths) + 2 * (len(widths) - 1) <= width or per_line == 1:
            best = (rows, widths)
            break
    rows, widths = best
    lines = []
    for row in rows:
        cells = [cell.ljust(widths[i]) for i, cell in enumerate(row)]
        lines.append("  ".join(cells).rstrip())
    return "\n".join(lines) + "\n"


def run(ctx):
    s, check = ctx.s, ctx.check
    down = reference(NAMES, 80)
    check("the reference is what was worked out by hand",
          down.split("\n")[0], "f01  f03  f05  f07  f09  f11  f13  f15  f17  f19")

    # ------------------------------------------------------------------------ ls
    check("on the terminal ls fills columns, down the columns first", s.run("ls /data/cols"), down)
    check("...and one per line through a pipe", s.run("ls /data/cols | cat"), "\n".join(NAMES) + "\n")
    check("...and into a file", s.run("ls /data/cols > /tmp/l1") + s.run("cat /tmp/l1"), "\n".join(NAMES) + "\n")
    check("-1 forces one per line on the terminal", s.run("ls -1 /data/cols"), "\n".join(NAMES) + "\n")
    check("-x fills across the rows", s.run("ls -x /data/cols"), reference(NAMES, 80, across=True))
    check("-C forces columns through a pipe, 80 wide", s.run("ls -C /data/cols | cat"), down)
    check("the last of -1/-C wins", s.run("ls -1 -C /data/cols"), down)
    check("...in the other order too", s.run("ls -C -1 /data/cols"), "\n".join(NAMES) + "\n")
    check("-l is one per line whatever else is asked", s.run("ls -l -C /data/cols").count("\n"), 20)
    check("sixteen 3-column names share one row", s.run("ls -x /data/dirs | wc -l"), "1\n")
    check("-F's marks count toward the width: the same names with '/' no longer fit one row",
          s.run("ls -xF /data/dirs | wc -l"), "2\n")
    check("two operands: each listing is laid out on its own",
          s.run("ls /data/cols /data/wide").count("f01  f03"), 1)
    check("names are measured in screen columns: 41-column names stack",
          s.run("ls /data/wide"), "\n".join(WIDE) + "\n")
    check("...but the same names would share a row if only characters counted",
          len(WIDE[0]) * 2 + 2 <= 80, True)

    # ---------------------------------------------------------------------- column
    check("column fills like ls does", s.run("column /data/list20.txt"), down)
    check("...from standard input", s.run("cat /data/list20.txt | column"), down)
    check("-c sets the width", s.run("column -c 20 /data/list20.txt"), reference(NAMES, 20))
    check("-x fills across", s.run("column -x -c 20 /data/list20.txt"), reference(NAMES, 20, across=True))
    check("-t aligns whitespace-separated fields and skips empty lines",
          s.run("column -t /data/table.txt"), "name   size\nalpha  1\nb      22    extra\n")
    check("-t -s splits on the character, keeping empty fields",
          s.run("column -t -s : /data/colon.txt"), "a   b     c\nxx  y  z  w\n")
    check("-o sets what goes between columns", s.run("column -t -o ' | ' /data/table.txt"),
          "name  | size\nalpha | 1\nb     | 22   | extra\n")
    check("-c with something that is not a width is refused", s.run("column -c 0 /data/list20.txt").startswith("column: invalid columns argument '0'"), True)
    check("a missing file is reported", s.run("column /nosuch"), "column: /nosuch: No such file or directory\n")
    check("--help says what it takes", s.run("column --help").startswith("usage: column "), True)
