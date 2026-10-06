"""`true`, `false`, `crash`, and the `-v`/`-n`/`-d` flags of `mkdir`, `rm` and `cp`."""

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
