"""The file utilities: ls, cat, head, tail, wc, echo, mkdir, rm, rmdir, cp, mv, touch, chmod,
stat, find, cmp, tee, hexdump, pwd and cd -- their flags, their error messages and their exit
statuses, against fixture files placed on the image."""

SYSTEM_FILES = {
    "data/lines.txt": "one\ntwo\nthree\nfour\nfive\n",
    "data/words.txt": "the quick brown fox\njumps over\nthe lazy dog\n",
    "data/noeol.txt": "a\nb",
    "data/blank.txt": "a\n\n\nb\n\tc\n",
    "data/tree/a.txt": "A\n",
    "data/tree/b/c.txt": "C\n",
    "data/tree/c2": "22\n",
    "data/tree/c10": "1010\n",
    "data/tree/.hidden": "h\n",
    "data/big.txt": "x" * 2500,
    "data/small.txt": "yy",
}


def run(ctx):
    s, check = ctx.s, ctx.check

    # ------------------------------------------------------------------ pwd and cd
    check("pwd starts in $HOME", s.run("pwd"), "/root\n")
    check("cd to a directory", s.run("cd /data/tree") + s.run("pwd"), "/data/tree\n")
    check("cd resolves .. lexically", s.run("cd b/..") + s.run("pwd"), "/data/tree\n")
    check("cd .. goes up", s.run("cd ..") + s.run("pwd"), "/data\n")
    check("cd with no argument goes to $HOME", s.run("cd") + s.run("pwd"), "/root\n")
    check("cd to a file is refused", s.run("cd /data/lines.txt"), "cd: /data/lines.txt: Not a directory\n")
    check("cd to nothing is refused", s.run("cd /nosuch"), "cd: /nosuch: No such file or directory\n")
    check("a refused cd leaves the directory alone", s.run("pwd"), "/root\n")
    check("cd's status", s.status("cd /nosuch")[1], 1)

    # ------------------------------------------------------------------ ls
    check("ls sorts names bytewise", s.run("ls /data/tree"), "a.txt\nb\nc10\nc2\n")
    check("ls hides dot-files", "hidden" in s.run("ls /data/tree"), False)
    check("ls -a shows them", s.run("ls -a /data/tree"), ".hidden\na.txt\nb\nc10\nc2\n")
    check("ls -r reverses", s.run("ls -r /data/tree"), "c2\nc10\nb\na.txt\n")
    check("ls -F marks directories", s.run("ls -F /data/tree"), "a.txt\nb/\nc10\nc2\n")
    check("ls -F marks executables", s.run("ls -F /usr/bin | head -n 2"), "cat*\nchmod*\n")
    check("ls -l shows attributes and size", s.run("ls -l /data/tree"),
          "-w-          2 a.txt\n"
          "dw-          0 b\n"
          "-w-          5 c10\n"
          "-w-          3 c2\n")
    check("ls -S sorts largest first", s.run("ls -S /data/tree"), "c10\nc2\na.txt\nb\n")
    check("ls -lh prints human sizes", s.run("ls -lh /data/big.txt"), "-w-   2.5K /data/big.txt\n")
    check("ls on a file lists the file", s.run("ls /data/lines.txt"), "/data/lines.txt\n")
    check("ls -d lists a directory itself", s.run("ls -d /data/tree"), "/data/tree\n")
    check("several operands get headers", s.run("ls /data/tree/b /data/small.txt"),
          "/data/small.txt\n\n/data/tree/b:\nc.txt\n")
    check("ls -R recurses", s.run("ls -R /data/tree"),
          "/data/tree:\na.txt\nb\nc10\nc2\n\n/data/tree/b:\nc.txt\n")
    check("ls of nothing", s.run("ls /nosuch"), "ls: /nosuch: No such file or directory\n")
    check("...has status 1", s.status("ls /nosuch")[1], 1)
    check("an unknown flag is refused", s.run("ls -z"), "ls: invalid option -- 'z'\nTry 'ls --help' for more information.\n")
    check("ls with no operand lists the working directory", s.run("cd /data/tree/b") + s.run("ls"), "c.txt\n")
    s.run("cd /root")

    # ------------------------------------------------------------------ cat
    check("cat prints a file", s.run("cat /data/lines.txt"), "one\ntwo\nthree\nfour\nfive\n")
    check("cat concatenates", s.run("cat /data/small.txt /data/noeol.txt"), "yya\nb")
    check("cat -n numbers lines", s.run("cat -n /data/noeol.txt"), "     1\ta\n     2\tb")
    check("cat -E marks line ends", s.run("cat -E /data/noeol.txt"), "a$\nb")
    check("cat -T shows tabs", s.run("cat -T /data/blank.txt").split("\n")[-2], "^Ic")
    check("cat -s squeezes blank lines", s.run("cat -s /data/blank.txt"), "a\n\nb\n\tc\n")
    check("cat -ns numbers after squeezing", s.run("cat -ns /data/blank.txt").split("\n")[:3],
          ["     1\ta", "     2\t", "     3\tb"])
    check("numbers continue across files (an unterminated line runs on into the next file)", s.run("cat -n /data/small.txt /data/noeol.txt"),
          "     1\tyya\n     2\tb")
    check("a missing file is reported and the rest are still read",
          s.run("cat /nosuch /data/small.txt"), "cat: /nosuch: No such file or directory\nyy")
    check("...with status 1", s.status("cat /nosuch")[1], 1)
    check("a lone - is stdin", s.run("echo piped | cat - /data/small.txt"), "piped\nyy")

    # ------------------------------------------------------------------ head and tail
    check("head defaults to ten lines", s.run("seq 20 | head").count("\n"), 10)
    check("head -n N", s.run("head -n 2 /data/lines.txt"), "one\ntwo\n")
    check("head -nN attached", s.run("head -n2 /data/lines.txt"), "one\ntwo\n")
    check("head -n 0 prints nothing", s.run("head -n 0 /data/lines.txt"), "")
    check("head -n -N is all but the last N", s.run("head -n -3 /data/lines.txt"), "one\ntwo\n")
    check("head -c N", s.run("head -c 5 /data/lines.txt"), "one\nt")
    check("head -n and -c together are refused", "mutually exclusive" in s.run("head -n 1 -c 2 /data/lines.txt"), True)
    check("head of several files has headers",
          s.run("head -n 1 /data/small.txt /data/noeol.txt"),
          "==> /data/small.txt <==\nyy\n==> /data/noeol.txt <==\na\n")
    check("head -q has none", s.run("head -q -n 1 /data/small.txt /data/noeol.txt"), "yya\n")
    check("head -v has one for a single file", s.run("head -v -n 1 /data/small.txt"), "==> /data/small.txt <==\nyy")
    check("a bad count is refused", s.run("head -n x /data/lines.txt"), "head: invalid number of lines: 'x'\n")
    check("tail -n N", s.run("tail -n 2 /data/lines.txt"), "four\nfive\n")
    check("tail -n +N starts at line N", s.run("tail -n +4 /data/lines.txt"), "four\nfive\n")
    check("tail -c N", s.run("tail -c 4 /data/lines.txt"), "ive\n")
    check("tail counts an unterminated last line", s.run("tail -n 1 /data/noeol.txt"), "b")
    check("tail with fewer lines than N prints all", s.run("tail -n 99 /data/small.txt"), "yy")
    check("tail of a missing file", s.run("tail /nosuch"), "tail: /nosuch: No such file or directory\n")
    check("head reads stdin", s.run("seq 5 | head -n 2"), "1\n2\n")

    # ------------------------------------------------------------------ wc
    check("wc counts lines, words and bytes", s.run("wc /data/words.txt"), "3 9 44 /data/words.txt\n")
    check("wc -l", s.run("wc -l /data/words.txt"), "3 /data/words.txt\n")
    check("wc -w", s.run("wc -w /data/words.txt"), "9 /data/words.txt\n")
    check("wc -c", s.run("wc -c /data/words.txt"), "44 /data/words.txt\n")
    check("wc -m counts characters", s.run("wc -m /data/words.txt"), "44 /data/words.txt\n")
    check("wc -L is the longest line", s.run("wc -L /data/words.txt"), "19 /data/words.txt\n")
    check("flags print in a fixed order", s.run("wc -c -l /data/words.txt"), "3 44 /data/words.txt\n")
    check("wc totals several files", s.run("wc -l /data/lines.txt /data/small.txt"),
          "5 /data/lines.txt\n0 /data/small.txt\n5 total\n")
    check("wc of stdin has no name", s.run("echo hi | wc"), "1 1 3\n")
    check("wc -l counts newlines, not lines", s.run("wc -l /data/noeol.txt"), "1 /data/noeol.txt\n")
    check("wc of a missing file", s.run("wc /nosuch"), "wc: /nosuch: No such file or directory\n")

    # ------------------------------------------------------------------ echo
    check("echo -n has no newline", s.run("echo -n a"), "a")
    check("echo -e interprets escapes", s.run("echo -e 'a\\tb\\\\c'"), "a\tb\\c\n")
    check("echo -e \\c stops all output", s.run("echo -e 'a\\cb'"), "a")
    check("echo -e octal and hex", s.run("echo -e '\\0101\\x42'"), "AB\n")
    check("echo -E leaves escapes", s.run("echo -E 'a\\tb'"), "a\\tb\n")
    check("echo -ne groups options", s.run("echo -ne 'x\\ty'"), "x\ty")
    check("a non-option dash word is text", s.run("echo -x -n"), "-x -n\n")
    check("echo --help is usage only as the first argument",
          s.run("echo --help").startswith("usage: echo"), True)
    check("...later it is text", s.run("echo a --help"), "a --help\n")

    # ------------------------------------------------------------------ mkdir, rmdir, rm
    check("mkdir creates", s.run("mkdir /tmp/d1") + s.run("ls -d /tmp/d1"), "/tmp/d1\n")
    check("mkdir of an existing name is an error", s.run("mkdir /tmp/d1"), "mkdir: cannot create directory '/tmp/d1': File exists\n")
    check("mkdir -p makes parents", s.run("mkdir -p /tmp/p/q/r") + s.run("ls /tmp/p/q"), "r\n")
    check("mkdir -p of an existing directory is fine", s.status("mkdir -p /tmp/p/q")[1], 0)
    check("mkdir without -p needs the parent", s.run("mkdir /tmp/x/y"), "mkdir: cannot create directory '/tmp/x/y': No such file or directory\n")
    check("rmdir removes an empty directory", s.run("rmdir /tmp/d1") + s.run("ls /tmp"), "p\n")
    check("rmdir refuses a non-empty one", s.run("rmdir /tmp/p"), "rmdir: failed to remove '/tmp/p': Directory not empty\n")
    check("rmdir refuses a file", s.run("rmdir /data/small.txt"), "rmdir: failed to remove '/data/small.txt': Not a directory\n")
    check("rm removes a file", s.run("echo x > /tmp/f") + s.run("rm /tmp/f") + s.run("ls /tmp"), "p\n")
    check("rm of a missing file is an error", s.run("rm /tmp/f"), "rm: cannot remove '/tmp/f': No such file or directory\n")
    check("rm -f ignores it", s.run("rm -f /tmp/f"), "")
    check("...with status 0", s.status("rm -f /tmp/f")[1], 0)
    check("rm of a directory needs -r", s.run("rm /tmp/p"), "rm: cannot remove '/tmp/p': Is a directory\n")
    check("rm -r removes a tree", s.run("rm -r /tmp/p") + s.run("ls /tmp"), "")
    check("rm refuses . and ..", s.run("rm -r .").startswith("rm: refusing to remove '.' or '..'"), True)
    check("rm refuses /", s.run("rm -r /"), "rm: it is dangerous to operate recursively on '/'\n")
    check("rm with no operand", s.run("rm").startswith("rm: missing operand"), True)

    # ------------------------------------------------------------------ cp
    check("cp copies a file", s.run("cp /data/lines.txt /tmp/c1") + s.run("cat /tmp/c1"), "one\ntwo\nthree\nfour\nfive\n")
    check("cp replaces an existing file", s.run("cp /data/small.txt /tmp/c1") + s.run("cat /tmp/c1"), "yy")
    check("cp into a directory keeps the name", s.run("mkdir /tmp/cd") + s.run("cp /data/small.txt /tmp/cd") + s.run("ls /tmp/cd"), "small.txt\n")
    check("cp of several files needs a directory", s.run("cp /data/small.txt /data/noeol.txt /tmp/c1"),
          "cp: target '/tmp/c1' is not a directory\n")
    check("cp of several files into a directory", s.run("cp /data/small.txt /data/noeol.txt /tmp/cd") + s.run("ls /tmp/cd"), "noeol.txt\nsmall.txt\n")
    check("cp refuses a directory without -r", s.run("cp /data/tree /tmp/t1"), "cp: -r not specified; omitting directory '/data/tree'\n")
    check("cp -r copies a tree", s.run("cp -r /data/tree /tmp/t1") + s.run("ls -aR /tmp/t1"),
          "/tmp/t1:\n.hidden\na.txt\nb\nc10\nc2\n\n/tmp/t1/b:\nc.txt\n")
    check("cp -r merges into an existing directory", s.status("cp -r /data/tree /tmp/t1")[1], 0)
    check("cp a file onto itself is refused", s.run("cp /tmp/c1 /tmp/c1"), "cp: '/tmp/c1' and '/tmp/c1' are the same file\n")
    check("cp a directory into itself is refused", s.run("cp -r /tmp/t1 /tmp/t1/b").startswith("cp: cannot copy a directory"), True)
    check("cp of a missing file", s.run("cp /nosuch /tmp/zz"), "cp: /nosuch: No such file or directory\n")
    check("...does not create the target", s.run("ls /tmp/zz"), "ls: /tmp/zz: No such file or directory\n")
    s.run("cp /nosuch /tmp/c1")
    check("an unreadable source does not truncate an existing target", s.run("cat /tmp/c1"), "yy")
    check("cp with one operand", s.run("cp /tmp/c1").startswith("cp: missing destination file operand after '/tmp/c1'"), True)

    # ------------------------------------------------------------------ mv
    check("mv renames", s.run("mv /tmp/c1 /tmp/m1") + s.run("ls /tmp/m1 /tmp/c1"), "ls: /tmp/c1: No such file or directory\n/tmp/m1\n")
    check("mv replaces a file", s.run("echo old > /tmp/m2") + s.run("mv /tmp/m1 /tmp/m2") + s.run("cat /tmp/m2"), "yy")
    check("a failing mv keeps the target", s.run("mv /tmp/nosuch /tmp/m2") + s.run("cat /tmp/m2"),
          "mv: cannot move '/tmp/nosuch' to '/tmp/m2': No such file or directory\nyy")
    check("mv into a directory", s.run("mv /tmp/m2 /tmp/cd") + s.run("ls /tmp/cd"), "m2\nnoeol.txt\nsmall.txt\n")
    check("mv of several files needs a directory", s.run("mv /tmp/cd /data/small.txt /tmp/nosuchdir"),
          "mv: target '/tmp/nosuchdir' is not a directory\n")
    s.run("echo f > /tmp/ff")
    s.run("mkdir /tmp/cd/ff")
    check("mv refuses to replace a directory with a file", s.run("mv /tmp/ff /tmp/cd"),
          "mv: cannot overwrite directory '/tmp/cd/ff' with non-directory\n")
    check("mv a directory into itself is refused", s.run("mv /tmp/t1 /tmp/t1/b/x"), "mv: cannot move '/tmp/t1' to a subdirectory of itself, '/tmp/t1/b/x'\n")
    check("mv a file onto itself is refused", s.run("mv /tmp/ff /tmp/ff"), "mv: '/tmp/ff' and '/tmp/ff' are the same file\n")
    check("mv moves a directory with its contents", s.run("mv /tmp/t1 /tmp/t2") + s.run("ls /tmp/t2/b"), "c.txt\n")

    # ------------------------------------------------------------------ chmod and stat
    check("a new file has no exec bit", s.run("ls -lF /tmp/ff"), "-w-          2 /tmp/ff\n")
    check("chmod +x sets it", s.run("chmod +x /tmp/ff") + s.run("ls -lF /tmp/ff"), "-wx          2 /tmp/ff*\n")
    check("chmod -x clears it", s.run("chmod -x /tmp/ff") + s.run("ls -l /tmp/ff"), "-w-          2 /tmp/ff\n")
    check("chmod -w makes it read-only", s.run("chmod -w /tmp/ff") + s.run("ls -l /tmp/ff"), "---          2 /tmp/ff\n")
    check("chmod +w undoes that", s.run("chmod +w /tmp/ff") + s.run("ls -l /tmp/ff"), "-w-          2 /tmp/ff\n")
    s.run("mkdir -p /tmp/cr/sub")
    s.run("echo 1 > /tmp/cr/a")
    s.run("echo 2 > /tmp/cr/sub/b")
    check("chmod -R recurses", s.run("chmod -R +x /tmp/cr") + s.run("ls -lR /tmp/cr"),
          "/tmp/cr:\n-wx          2 a\ndwx          0 sub\n\n/tmp/cr/sub:\n-wx          2 b\n")
    check("chmod of a missing file", s.run("chmod +x /nosuch"), "chmod: cannot change permissions of '/nosuch': No such file or directory\n")
    check("chmod needs a mode", s.run("chmod").startswith("chmod: missing"), True)
    out = s.run("stat /data/small.txt").split("\n")
    check("stat shows the name, size and type", out[:3], ["  File: /data/small.txt", "  Size: 2            Type: regular file", " Attrs: read-only=no  exec=no"])
    check("stat of a directory", s.run("stat /data/tree").split("\n")[1], "  Size: 0            Type: directory")
    check("stat of a missing file", s.run("stat /nosuch"), "stat: cannot stat '/nosuch': No such file or directory\n")

    # ------------------------------------------------------------------ find, cmp, tee, hexdump
    check("find lists a tree in name order", s.run("find /data/tree"),
          "/data/tree\n/data/tree/.hidden\n/data/tree/a.txt\n/data/tree/b\n/data/tree/b/c.txt\n/data/tree/c10\n/data/tree/c2\n")
    check("find -type d", s.run("find /data/tree -type d"), "/data/tree\n/data/tree/b\n")
    check("find -name globs the last component", s.run("find /data/tree -name 'c*'"), "/data/tree/b/c.txt\n/data/tree/c10\n/data/tree/c2\n")
    check("find -maxdepth/-mindepth", s.run("find /data/tree -mindepth 1 -maxdepth 1 -type f"),
          "/data/tree/.hidden\n/data/tree/a.txt\n/data/tree/c10\n/data/tree/c2\n")
    check("find of a missing path", s.run("find /nosuch"), "find: /nosuch: No such file or directory\n")
    check("find with an unknown predicate", s.run("find /data -frob"), "find: unknown predicate '-frob'\n")
    check("cmp of equal files is silent", s.run("cmp /data/lines.txt /data/lines.txt"), "")
    check("cmp names the first difference", s.run("cmp /data/small.txt /data/noeol.txt"),
          "/data/small.txt /data/noeol.txt differ: byte 1, line 1\n")
    check("cmp -s is silent but has a status", s.status("cmp -s /data/small.txt /data/noeol.txt"), ("", 1))
    check("cmp of a prefix", s.run("cmp /data/noeol.txt /data/lines.txt").startswith("/data/noeol.txt /data/lines.txt differ"), True)
    check("cmp of a missing file has status 2", s.status("cmp /nosuch /data/small.txt")[1], 2)
    check("tee copies to stdout and files", s.run("echo teed | tee /tmp/te1 /tmp/te2") + s.run("cat /tmp/te1 /tmp/te2"), "teed\nteed\nteed\n")
    check("tee -a appends", s.run("echo more | tee -a /tmp/te1") + s.run("cat /tmp/te1"), "more\nteed\nmore\n")
    check("hexdump", s.run("hexdump /data/small.txt"),
          "00000000  " + "79 79 ".ljust(49) + " |yy|\n00000002\n")
