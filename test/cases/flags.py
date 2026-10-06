"""`true`, `false`, `crash`, the `-v`/`-n`/`-d` flags of `mkdir`, `rm` and `cp`, and `mv`'s flags."""

SYSTEM_FILES = {
    "data/tree/a.txt": "A\n",
    "data/tree/b/c.txt": "C\n",
    "data/one": "1\n",
}


def lines(text):
    return sorted(text.splitlines())


def run(ctx):
    s, check = ctx.s, ctx.check

    # ------------------------------------------------------------------ true, false
    check("true exits 0", s.status("true"), ("", 0))
    check("false exits 1", s.status("false"), ("", 1))
    check("both ignore their arguments", (s.status("true x -y --z")[1], s.status("false x -y --z")[1]), (0, 1))
    check("true --help", s.run("true --help"), "usage: true\n")
    check("false --help", s.run("false --help"), "usage: false\n")

    # ------------------------------------------------------------------ crash
    out, status = s.status("crash")
    check("crash says so, then faults", out.startswith("about to crash\nSegmentation fault (address 0xffff800000000000"), True)
    check("...status 139", status, 139)
    check("...and the shell goes on", s.run("echo ok"), "ok\n")

    # ------------------------------------------------------------------ mkdir -v
    check("mkdir -v says what it made", s.run("mkdir -v /tmp/v1"), "mkdir: created directory '/tmp/v1'\n")
    check("mkdir -pv says each level made, not the ones there already",
          s.run("mkdir -pv /tmp/v1/a/b"), "mkdir: created directory '/tmp/v1/a'\nmkdir: created directory '/tmp/v1/a/b'\n")
    check("mkdir -v says nothing of a failure but the error", s.run("mkdir -v /tmp/v1"),
          "mkdir: cannot create directory '/tmp/v1': File exists\n")
    check("without -v it stays quiet", s.run("mkdir /tmp/v2"), "")

    # ------------------------------------------------------------------ rm -v, -d
    s.run("echo x > /tmp/f1")
    check("rm -v says what it removed", s.run("rm -v /tmp/f1"), "removed '/tmp/f1'\n")
    check("rm -v of nothing says only the error", s.run("rm -v /tmp/f1"), "rm: cannot remove '/tmp/f1': No such file or directory\n")
    check("rm -fv of nothing says nothing", s.run("rm -fv /tmp/f1"), "")
    check("rm -d removes an empty directory", s.run("rm -dv /tmp/v2"), "removed directory '/tmp/v2'\n")
    check("rm -d refuses a full one", s.run("rm -d /tmp/v1"), "rm: cannot remove '/tmp/v1': Directory not empty\n")
    check("rm without -d refuses a directory", s.run("mkdir /tmp/v3") + s.run("rm /tmp/v3"),
          "rm: cannot remove '/tmp/v3': Is a directory\n")
    check("rm -d of a plain file works too", s.run("echo y > /tmp/f2") + s.run("rm -d /tmp/f2") + s.run("ls /tmp/f2"),
          "ls: /tmp/f2: No such file or directory\n")
    check("rm -rv says each, contents before their directory",
          s.run("rm -rv /tmp/v1").splitlines(),
          ["removed directory '/tmp/v1/a/b'", "removed directory '/tmp/v1/a'", "removed directory '/tmp/v1'"])
    s.run("cp -r /data/tree /tmp/t1")
    out = s.run("rm -rv /tmp/t1")
    check("rm -rv of a tree names every file and directory", lines(out), sorted([
        "removed '/tmp/t1/a.txt'", "removed '/tmp/t1/b/c.txt'", "removed directory '/tmp/t1/b'",
        "removed directory '/tmp/t1'"]))
    check("...a directory only after what is in it",
          out.splitlines().index("removed directory '/tmp/t1/b'") > out.splitlines().index("removed '/tmp/t1/b/c.txt'")
          and out.splitlines()[-1] == "removed directory '/tmp/t1'", True)

    # ------------------------------------------------------------------ cp -v, -n
    check("cp -v says each copy", s.run("cp -v /data/one /tmp/c1"), "'/data/one' -> '/tmp/c1'\n")
    check("cp -v into a directory says the real target", s.run("mkdir /tmp/cd") + s.run("cp -v /data/one /tmp/cd"),
          "'/data/one' -> '/tmp/cd/one'\n")
    check("cp -rv says the directory, then its entries", lines(s.run("cp -rv /data/tree /tmp/c2")), sorted([
        "'/data/tree' -> '/tmp/c2'", "'/data/tree/a.txt' -> '/tmp/c2/a.txt'",
        "'/data/tree/b' -> '/tmp/c2/b'", "'/data/tree/b/c.txt' -> '/tmp/c2/b/c.txt'"]))
    s.run("echo mine > /tmp/keep")
    check("cp -n leaves an existing file alone, silently", s.run("cp -nv /data/one /tmp/keep") + s.run("cat /tmp/keep"), "mine\n")
    check("cp -n still copies a missing one", s.run("cp -nv /data/one /tmp/new") + s.run("cat /tmp/new"),
          "'/data/one' -> '/tmp/new'\n1\n")
    check("cp without -n replaces it", s.run("cp /data/one /tmp/keep") + s.run("cat /tmp/keep"), "1\n")
    s.run("echo mine > /tmp/c2/a.txt")
    check("cp -rn merges, keeping what is there",
          s.run("cp -rn /data/tree /tmp/c2") + s.run("cat /tmp/c2/a.txt"), "mine\n")
    check("--no-clobber and --verbose are the long forms", s.run("cp --no-clobber --verbose /data/one /tmp/new"), "")

    # ------------------------------------------------------------------ mv flags
    def put(path, text="x"):
        s.run(f"echo {text} > {path}")

    s.run("mkdir /tmp/m")
    put("/tmp/m/a", "new")
    put("/tmp/m/b", "old")
    check("mv -v says each move", s.run("mv -v /tmp/m/a /tmp/m/c"), "renamed '/tmp/m/a' -> '/tmp/m/c'\n")
    s.run("mv /tmp/m/c /tmp/m/a")

    # -n
    check("mv -n will not replace a file", s.status("mv -n /tmp/m/a /tmp/m/b"), ("mv: not replacing '/tmp/m/b'\n", 1))
    check("...and moves nothing", s.run("cat /tmp/m/a /tmp/m/b"), "new\nold\n")
    check("mv -n moves to a name that is free", s.run("mv -n /tmp/m/a /tmp/m/free") + s.run("cat /tmp/m/free"), "new\n")
    s.run("mv /tmp/m/free /tmp/m/a")
    check("the last of -n and -f wins", s.run("mv -n -f /tmp/m/a /tmp/m/b") + s.run("cat /tmp/m/b"), "new\n")
    put("/tmp/m/a", "new")
    put("/tmp/m/b", "old")
    check("...-f then -n", s.status("mv -f -n /tmp/m/a /tmp/m/b"), ("mv: not replacing '/tmp/m/b'\n", 1))
    check("--no-clobber is -n", s.status("mv --no-clobber /tmp/m/a /tmp/m/b")[1], 1)

    # -i (the answer comes from stdin)
    check("mv -i asks, and y replaces", s.run("echo y | mv -i /tmp/m/a /tmp/m/b") + s.run("cat /tmp/m/b"),
          "mv: overwrite '/tmp/m/b'? new\n")
    put("/tmp/m/a", "newer")
    check("anything but y declines (status 1, nothing moves)",
          s.status("echo n | mv -i /tmp/m/a /tmp/m/b")[1], 1)
    check("...", s.run("cat /tmp/m/a /tmp/m/b"), "newer\nnew\n")
    check("mv -i says nothing when the target is free", s.run("mv -i /tmp/m/a /tmp/m/free") + s.run("cat /tmp/m/free"), "newer\n")
    s.run("rm /tmp/m/free")
    put("/tmp/m/a", "new")
    put("/tmp/m/b", "old")

    # -u
    s.run("touch -d @1000000000 /tmp/m/old.txt")
    s.run("touch -d @1500000000 /tmp/m/new.txt")
    s.run("touch -d @1500000000 /tmp/m/same.txt")
    check("mv -u skips a source that is older, silently and with status 0", s.status("mv -u /tmp/m/old.txt /tmp/m/new.txt"), ("", 0))
    check("...both are still there", s.run("ls /tmp/m"), "a\nb\nnew.txt\nold.txt\nsame.txt\n")
    check("mv -u skips one that is as old", s.status("mv -u /tmp/m/same.txt /tmp/m/new.txt"), ("", 0))
    check("mv -u replaces an older target", s.run("mv -u /tmp/m/new.txt /tmp/m/old.txt") + s.run("ls /tmp/m"), "a\nb\nold.txt\nsame.txt\n")
    check("mv -u moves to a free name", s.run("mv -u /tmp/m/same.txt /tmp/m/free") + s.run("ls /tmp/m"), "a\nb\nfree\nold.txt\n")
    s.run("rm /tmp/m/free /tmp/m/old.txt")

    # -b
    check("mv -b keeps the replaced file as NAME~", s.run("mv -b /tmp/m/a /tmp/m/b") + s.run("cat /tmp/m/b /tmp/m/b~"), "new\nold\n")
    put("/tmp/m/a", "n2")
    check("mv -bv says so", s.run("mv -bv /tmp/m/a /tmp/m/b"), "renamed '/tmp/m/a' -> '/tmp/m/b' (backup: '/tmp/m/b~')\n")
    check("...the older backup is replaced by the newer", s.run("cat /tmp/m/b /tmp/m/b~"), "n2\nnew\n")
    put("/tmp/m/a", "n3")
    check("mv -b to a free name makes no backup", s.run("mv -b /tmp/m/a /tmp/m/free") + s.run("ls /tmp/m"), "b\nb~\nfree\n")
    s.run("rm /tmp/m/free /tmp/m/b~")

    # -t and -T
    s.run("mkdir /tmp/m/dir")
    put("/tmp/m/p")
    put("/tmp/m/q")
    check("mv -t moves every operand into the directory", s.run("mv -t /tmp/m/dir /tmp/m/p /tmp/m/q") + s.run("ls /tmp/m/dir"), "p\nq\n")
    check("mv --target-directory is -t", s.run("mv --target-directory=/tmp/m /tmp/m/dir/p") + s.run("ls /tmp/m"), "b\ndir\np\n")
    check("mv -t needs a directory", s.status("mv -t /tmp/m/b /tmp/m/p"),
          ("mv: target directory '/tmp/m/b': Not a directory\n", 1))
    check("...that exists", s.status("mv -t /tmp/m/nodir /tmp/m/p"),
          ("mv: target directory '/tmp/m/nodir': No such file or directory\n", 1))
    check("mv -t with no source", s.run("mv -t /tmp/m/dir").startswith("mv: missing file operand\n"), True)
    check("-t and -T together are refused", s.run("mv -t /tmp/m/dir -T /tmp/m/p").startswith(
        "mv: cannot combine --target-directory (-t) and --no-target-directory (-T)\n"), True)
    check("mv -T will not move into a directory", s.run("mv -T /tmp/m/p /tmp/m/dir"),
          "mv: cannot overwrite directory '/tmp/m/dir' with non-directory\n")
    check("mv -T renames to a free name", s.run("mv -T /tmp/m/p /tmp/m/p2") + s.run("ls /tmp/m"), "b\ndir\np2\n")
    check("mv -T takes one source only", s.run("mv -T /tmp/m/p2 /tmp/m/b /tmp/m/x").startswith("mv: extra operand '/tmp/m/b'\n"), True)
    check("a destination with a trailing slash must be a directory", s.run("mv /tmp/m/p2 /tmp/m/b/"),
          "mv: cannot move '/tmp/m/p2' to '/tmp/m/b/': Not a directory\n")
    check("...a directory's is fine", s.run("mv /tmp/m/p2 /tmp/m/dir/") + s.run("ls /tmp/m/dir"), "p2\nq\n")

    # -u, -n and -v together, and a directory in the way whatever the flags
    put("/tmp/m/k", "k")
    check("mv -nuv: the target exists and is not older: not replaced", s.status("mv -nuv /tmp/m/k /tmp/m/b")[1], 1)
    s.run("mkdir /tmp/m/dd")
    check("a directory in the way is refused with -f (-T: else k would move into it)", s.run("mv -fT /tmp/m/k /tmp/m/dd"),
          "mv: cannot overwrite directory '/tmp/m/dd' with non-directory\n")
    check("...and with -b", s.run("mv -bT /tmp/m/k /tmp/m/dd") + s.run("ls /tmp/m"),
          "mv: cannot overwrite directory '/tmp/m/dd' with non-directory\nb\ndd\ndir\nk\n")
    check("a failing move with -b keeps the target in place", s.run("mv -b /tmp/m/nosuch /tmp/m/b").startswith("mv: cannot move"), True)
