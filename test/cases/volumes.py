"""Working across two volumes: cp, mv (which copies and removes where `rename` cannot), what survives
an unmount, and what ends up on the HOME disk."""

HOME_FILES = {"inhome.txt": "home data\n", "tree/a": "A\n", "tree/sub/b": "B\n"}


def run(ctx):
    s, check = ctx.s, ctx.check

    s.run("echo sys > /tmp/onsys.txt")
    check("cp from one volume to the other", s.run("cp /tmp/onsys.txt /root/copied.txt") + s.run("cat /root/copied.txt"), "sys\n")
    check("...and back", s.run("cp /root/inhome.txt /tmp/back.txt") + s.run("cat /tmp/back.txt"), "home data\n")
    check("cp -r across volumes", s.run("cp -r /root/tree /tmp/treecopy") + s.run("find /tmp/treecopy"),
          "/tmp/treecopy\n/tmp/treecopy/a\n/tmp/treecopy/sub\n/tmp/treecopy/sub/b\n")

    # ------------------------------------------------------------ mv across volumes
    check("mv a file to the other volume", s.run("mv /tmp/onsys.txt /root/moved.txt") + s.run("cat /root/moved.txt"), "sys\n")
    check("...the source is gone", s.run("ls -1 /tmp/onsys.txt"), "ls: /tmp/onsys.txt: No such file or directory\n")
    check("...and no temporary is left", s.run("ls -1 -a /root"), "copied.txt\ninhome.txt\nmoved.txt\ntree\n")
    s.run("echo old > /tmp/target")
    s.run("echo new > /root/src")
    check("mv replaces an existing file on the other volume", s.run("mv /root/src /tmp/target") + s.run("cat /tmp/target"), "new\n")
    s.run("echo keep > /tmp/keep")
    check("a mv that fails keeps the target", s.run("mv /root/nosuch /tmp/keep") + s.run("cat /tmp/keep"),
          "mv: cannot stat '/root/nosuch': No such file or directory\nkeep\n")
    check("mv a whole tree across volumes", s.run("mv /root/tree /tmp/tree2") + s.run("find /tmp/tree2"),
          "/tmp/tree2\n/tmp/tree2/a\n/tmp/tree2/sub\n/tmp/tree2/sub/b\n")
    check("...the source tree is gone", s.run("ls -1 /root/tree"), "ls: /root/tree: No such file or directory\n")
    s.run("echo 1 > /root/x1")
    s.run("echo 2 > /root/y1")
    s.run("mkdir /tmp/into")
    check("mv several files into a directory on the other volume",
          s.run("mv /root/x1 /root/y1 /tmp/into") + s.run("ls -1 /tmp/into"), "x1\ny1\n")
    check("mv a directory into a directory on the other volume",
          s.run("mkdir /root/d") + s.run("echo q > /root/d/q") + s.run("mv /root/d /tmp/into") + s.run("cat /tmp/into/d/q"), "q\n")
    s.run("echo f > /root/f")
    s.run("mkdir /tmp/into/d/f")
    check("mv refuses to replace a directory on the other volume with a file",
          s.run("mv /root/f /tmp/into/d"), "mv: cannot overwrite directory '/tmp/into/d/f' with non-directory\n")

    # ------------------------------------------------------------ what a move keeps
    s.run("echo '#' > /root/script")
    s.run("chmod +x /root/script")
    s.run("touch -d 2020-05-01T12:00:00Z /root/script")
    check("mv across volumes keeps the executable bit", s.run("mv /root/script /tmp/script") + s.run("ls -lF /tmp/script"),
          "-wx          2 /tmp/script*\n")
    check("...and the modify time", s.run("stat /tmp/script").split("\n")[3], "Modify: 2020-05-01 12:00:00 UTC")
    s.run("echo x > /root/plainfile")
    s.run("touch -d 2021-01-02T03:04:06Z /root/plainfile")
    s.run("cp /root/plainfile /tmp/cp1")
    check("cp does not keep the time", s.run("stat /tmp/cp1").split("\n")[3].startswith("Modify: 2021"), False)
    check("cp -p keeps it", s.run("cp -p /root/plainfile /tmp/cp2") + s.run("stat /tmp/cp2").split("\n")[3], "Modify: 2021-01-02 03:04:06 UTC")

    # ------------------------------------------------------------ mv's flags across volumes
    s.run("echo one > /tmp/xv")
    s.run("echo two > /root/xv")
    check("mv -n will not replace on the other volume", s.status("mv -n /tmp/xv /root/xv"), ("mv: not replacing '/root/xv'\n", 1))
    check("mv -bv across volumes keeps the old file beside the new one",
          s.run("mv -bv /tmp/xv /root/xv") + s.run("cat /root/xv /root/xv~"),
          "renamed '/tmp/xv' -> '/root/xv' (backup: '/root/xv~')\none\ntwo\n")
    check("...and the source is gone", s.run("ls -1 /tmp/xv"), "ls: /tmp/xv: No such file or directory\n")
    s.run("rm /root/xv /root/xv~")

    # ------------------------------------------------------------ mount points cannot be moved
    check("a mount point cannot be moved", s.run("mv /root /tmp/r"), "mv: cannot move '/root' to '/tmp/r': Device or resource busy\n")
    check("moving into a mount point is fine: it is the directory the volume's root is",
          s.run("mv /tmp/cp1 /root") + s.run("cat /root/cp1"), "x\n")
    check("the volume is intact", s.run("ls -1 /root").split("\n")[:2], ["copied.txt", "cp1"])

    # ------------------------------------------------------------ persistence
    s.run("echo persistent > /root/persist.txt")
    s.run("cd /")
    s.run("umount /root")
    s.run("mount LABEL=HOME /root")
    check("a file written before an unmount is there after a remount", s.run("cat /root/persist.txt"), "persistent\n")
    s.run("echo disk > /root/verify.txt")


def verify_disk(ctx):
    import harness
    check = ctx.check
    check("the HOME disk holds what was written to /root", harness.get_file(ctx.home_img, "verify.txt"), b"disk\n")
    check("...and what was moved there", harness.get_file(ctx.home_img, "moved.txt"), b"sys\n")
    check("...and not what was moved away", harness.get_file(ctx.home_img, "tree/a"), None)
    check("...and no temporary", harness.get_file(ctx.home_img, ".mv-partial"), None)
    check("the system disk holds the file moved to it", harness.get_file(ctx.system_img, "tmp/target"), b"new\n")
