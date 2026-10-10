"""UTF-8 text on the console: what a program writes is decoded (a character may be split between
two writes, a bad byte is one replacement character), wide glyphs take two cells, and a prompt may
hold any text. The serial log has the decoded text; the display is checked with `screendump`."""

from harness import Screen

SPLIT = "x" * 511 + "日" + "\n"  # `cat` reads 512 bytes at a time: the glyph's bytes straddle two reads

SYSTEM_FILES = {
    "etc/environment": "HOME=/root\nPS1='\\w日本> '\n",
    "data/hello.txt": "héllo 日本語 wörld\n",
    "data/split.txt": SPLIT,
    "data/bad.bin": b"A\xffB\xc3(C\xe6\x97\n",  # a stray byte, a lead byte cut short, a three-byte start cut at the end
    "data/fffd.txt": "�\n",
    "data/ff.txt": b"\xff\n",
    "data/wide.txt": "日a\n",
    "data/日本語.txt": "named in Japanese\n",
}


def run(ctx):
    s, check = ctx.s, ctx.check

    check("a prompt may hold wide characters", s.run("pwd"), "/root\n")
    check("cat of UTF-8 text", s.run("cat /data/hello.txt"), "héllo 日本語 wörld\n")
    check("a character split between two reads is whole", s.run("cat /data/split.txt"), SPLIT)
    check("bad bytes each become one replacement character, and the rest goes on",
          s.run("cat /data/bad.bin"), "A�B�(C�\n")
    check("wc -m counts characters, wc -c bytes", s.run("wc -m /data/hello.txt"), "16 /data/hello.txt\n")
    check("ls shows a name that is not ASCII", s.run("ls /data").count("日本語.txt"), 1)

    # ------------------------------------------------------------------ the display
    s.run("clear")
    s.run("cat /data/fffd.txt /data/ff.txt /data/wide.txt")
    w, h, pixels = s.screendump()
    screen = Screen(w, h, pixels)
    # Row 0 holds the prompt (`/root日本> `: five cells, two wide glyphs, then `> `) and the typed
    # command; the three lines of output follow it.
    check("the prompt's wide characters are two cells each",
          [screen.drawn(0, c) for c in range(5, 10)], [True] * 5)
    check("...and the two glyphs are four different halves",
          len({screen.cell(0, c) for c in range(5, 9)}), 4)
    check("a replacement character is drawn", screen.drawn(1, 0), True)
    check("a bad byte draws the very same glyph as U+FFFD", screen.cell(2, 0), screen.cell(1, 0))
    check("a wide glyph covers its cell and the next", (screen.drawn(3, 0), screen.drawn(3, 1)), (True, True))
    check("the narrow character after it is in the third cell", (screen.drawn(3, 2), screen.drawn(3, 3)), (True, False))
    check("the two halves of a wide glyph are not the same picture", screen.cell(3, 0) != screen.cell(3, 1), True)

    # ------------------------------------------------------------------ the line editor with such a prompt
    long = "a" * 170  # more than a row: the line wraps, after a prompt with wide glyphs on its first row
    check("a typed line that wraps past a wide-glyph prompt runs as typed", s.run(f"echo {long}"), long + "\n")
    check("...and the prompt after it is intact", s.run("pwd"), "/root\n")
