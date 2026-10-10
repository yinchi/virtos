"""Edges: input larger than one read chunk, FAT's case-insensitive and long names, names with
blanks, every character the keyboard can type, and a pre-existing file where a pipeline wants to put
its temporary."""

import string

LONG = "".join(f"line {i}\n" for i in range(1, 301))  # 2,592 bytes: several 512-byte reads
SYSTEM_FILES = {
    "data/long.txt": LONG,
    "data/MixedCase.TXT": "mixed\n",
}


def run(ctx):
    s, check = ctx.s, ctx.check

    # ---------------------------------------------------------------- input larger than a chunk
    check("wc counts a multi-chunk file", s.run("wc /data/long.txt"), "300 600 2592 /data/long.txt\n")
    check("cat -n numbers every line across chunk boundaries",
          s.run("cat -n /data/long.txt").split("\n")[-2], "   300\tline 300")
    check("...without losing or repeating one", s.run("cat -n /data/long.txt | wc -l"), "300\n")
    check("cat reproduces the file exactly", s.run("cat /data/long.txt | cmp - /data/long.txt"), "")
    check("tail takes the end of a multi-chunk file", s.run("tail -n 2 /data/long.txt"), "line 299\nline 300\n")
    check("tail -c across chunks", s.run("tail -c 9 /data/long.txt"), "line 300\n")
    check("head takes the start", s.run("head -n 2 /data/long.txt"), "line 1\nline 2\n")
    check("sort handles many lines", s.run("sort -n /data/long.txt | head -n 1"), "line 1\n")
    check("sort -r of 300 lines puts the bytewise largest first", s.run("sort -r /data/long.txt | head -n 1"), "line 99\n")
    check("uniq -c over many lines", s.run("sort /data/long.txt | uniq -c | wc -l"), "300\n")
    check("fgrep over many lines", s.run("fgrep -c 'line 1' /data/long.txt"), "111\n")
    check("a pipeline carries a multi-chunk stream", s.run("cat /data/long.txt | cat | wc -l"), "300\n")
    check("cp of a multi-chunk file is exact", s.run("cp /data/long.txt /tmp/long2") + s.run("cmp /data/long.txt /tmp/long2"), "")
    check("tr over many lines", s.run("tr a-z A-Z < /data/long.txt | head -n 1"), "LINE 1\n")
    check("hexdump over many rows", s.run("hexdump /data/long.txt | tail -n 1"), "00000a20\n")

    # ---------------------------------------------------------------- names
    check("lookups are case-insensitive", s.run("cat /DATA/LONG.TXT | head -n 1"), "line 1\n")
    check("...and a name keeps its case as stored", s.run("ls -1 /data").split("\n")[0], "MixedCase.TXT")
    check("a mixed-case name is found in any case", s.run("cat /data/mixedcase.txt"), "mixed\n")
    check("a name with a blank, quoted", s.run("echo spaced > '/tmp/a b'") + s.run("cat '/tmp/a b'"), "spaced\n")
    check("...and ls shows it", s.run("ls -1 /tmp"), "a b\nlong2\n")
    check("a name with a blank, escaped", s.run("cat /tmp/a\\ b"), "spaced\n")
    long_name = "a-rather-long-file-name-that-needs-a-long-entry.txt"
    check("a long name works", s.run(f"echo long > /tmp/{long_name}") + s.run(f"cat /tmp/{long_name}"), "long\n")
    check("...and is listed whole", s.run("ls -1 /tmp"), f"a b\n{long_name}\nlong2\n")
    check("a name with dots and dashes", s.run("echo d > /tmp/x.y-z_1") + s.run("cat /tmp/x.y-z_1"), "d\n")
    check("two names differing only in case are the same file", s.run("echo one > /tmp/Same") + s.run("echo two > /tmp/SAME") + s.run("cat /tmp/same"), "two\n")

    # ---------------------------------------------------------------- every typable character
    printable = [c for c in string.printable if c not in "\t\n\r\x0b\x0c" and c != "'"]
    text = "".join(printable)
    check("every printable ASCII character can be typed (inside single quotes)", s.run(f"echo '{text}'"), text + "\n")
    check("a single quote can be typed inside double quotes", s.run("echo \"it's\""), "it's\n")

    # ---------------------------------------------------------------- the pipeline's temporary
    s.run("echo user-data > /tmp/.pipe0")
    check("a pipeline works with a file at its first temporary name", s.run("echo hello | cat"), "hello\n")
    check("...and does not touch that file", s.run("cat /tmp/.pipe0"), "user-data\n")
