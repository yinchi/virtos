"""What each failure of a file system call is called: the negated errno the kernel returns, as the
programs word it (`strerror`'s text). One check per distinct error, through the program that makes
the call, so a kernel that went back to one catch-all error would fail here."""

SYSTEM_FILES = {
    "data/file.txt": "text\n",
    "data/dir/inner": "i\n",
}

NOENT = "No such file or directory"


def run(ctx):
    s, check = ctx.s, ctx.check

    # ------------------------------------------------------------------ ENOENT, ENOTDIR, EISDIR
    check("open of a missing file: ENOENT", s.run("cat /data/nosuch"), f"cat: /data/nosuch: {NOENT}\n")
    check("a file in the middle of a path: ENOTDIR", s.run("cat /data/file.txt/x"), "cat: /data/file.txt/x: Not a directory\n")
    check("a missing directory in the middle: ENOENT", s.run("cat /nodir/x"), f"cat: /nodir/x: {NOENT}\n")
    check("read of a directory: EISDIR", s.run("cat /data/dir"), "cat: /data/dir: Is a directory\n")
    check("stat of a path through a file: ENOTDIR", s.run("stat /data/file.txt/x"), "stat: cannot stat '/data/file.txt/x': Not a directory\n")

    # ------------------------------------------------------------------ redirects
    check("a > onto a directory: EISDIR", s.run("echo x > /data/dir"), "/data/dir: Is a directory\n")
    check("a < from a directory: EISDIR", s.run("cat < /data/dir"), "/data/dir: Is a directory\n")
    check("a > into a missing directory: ENOENT", s.run("echo x > /nodir/f"), f"/nodir/f: {NOENT}\n")
    check("a > through a file: ENOTDIR", s.run("echo x > /data/file.txt/f"), "/data/file.txt/f: Not a directory\n")

    # ------------------------------------------------------------------ mkdir, rmdir, rm
    check("mkdir of an existing file: EEXIST", s.run("mkdir /data/file.txt"), "mkdir: cannot create directory '/data/file.txt': File exists\n")
    check("mkdir of the root: EEXIST", s.run("mkdir /"), "mkdir: cannot create directory '/': File exists\n")
    check("mkdir of a mount point: EEXIST", s.run("mkdir /root"), "mkdir: cannot create directory '/root': File exists\n")
    check("mkdir through a file: ENOTDIR", s.run("mkdir /data/file.txt/d"), "mkdir: cannot create directory '/data/file.txt/d': Not a directory\n")
    check("rmdir of a non-empty directory: ENOTEMPTY", s.run("rmdir /data/dir"), "rmdir: failed to remove '/data/dir': Directory not empty\n")
    check("rmdir of a mount point: EBUSY", s.run("rmdir /root"), "rmdir: failed to remove '/root': Device or resource busy\n")
    check("rmdir of nothing: ENOENT", s.run("rmdir /data/nosuch"), f"rmdir: failed to remove '/data/nosuch': {NOENT}\n")
    check("rm of a directory without -r: EISDIR", s.run("rm /data/dir"), "rm: cannot remove '/data/dir': Is a directory\n")
    check("rm -f of nothing is silent, of a path through a file is not", s.run("rm -f /data/nosuch /data/file.txt/x"),
          "rm: cannot remove '/data/file.txt/x': Not a directory\n")

    # ------------------------------------------------------------------ mv (rename)
    s.run("echo f > /tmp/f")
    s.run("mkdir /tmp/d")
    check("rename of nothing: ENOENT", s.run("mv /tmp/nosuch /tmp/g"), f"mv: cannot move '/tmp/nosuch' to '/tmp/g': {NOENT}\n")
    check("rename into a missing directory: ENOENT", s.run("mv /tmp/f /tmp/nodir/g"), f"mv: cannot move '/tmp/f' to '/tmp/nodir/g': {NOENT}\n")
    check("a directory over a file: EEXIST", s.run("mv /tmp/d /tmp/f"), "mv: cannot move '/tmp/d' to '/tmp/f': File exists\n")
    check("a directory into itself: EINVAL", s.run("mv /tmp/d /tmp/d/x"), "mv: cannot move '/tmp/d' to a subdirectory of itself, '/tmp/d/x'\n")
    check("none of that changed anything", s.run("ls /tmp"), "d\nf\n")

    # ------------------------------------------------------------------ chmod, touch, cd
    check("chmod of nothing: ENOENT", s.run("chmod +x /data/nosuch"), f"chmod: cannot change permissions of '/data/nosuch': {NOENT}\n")
    check("chmod of the root: EINVAL", s.run("chmod +x /"), "chmod: cannot change permissions of '/': Invalid argument\n")
    check("touch in a missing directory: ENOENT", s.run("touch /nodir/f"), f"touch: cannot touch '/nodir/f': {NOENT}\n")
    check("touch of the root: EINVAL", s.run("touch /"), "touch: setting times of '/': Invalid argument\n")
    check("touch -c through a file is still an error", s.run("touch -c /data/file.txt/x"),
          "touch: cannot touch '/data/file.txt/x': Not a directory\n")
    check("touch -c of nothing is silent", s.run("touch -c /data/nosuch"), "")
    check("cd through a file: ENOTDIR", s.run("cd /data/file.txt/x"), "cd: /data/file.txt/x: Not a directory\n")
    check("source of a directory", s.run("source /data/dir"), "source: /data/dir: is a directory\n")
    check("source of the root", s.run("source /"), "source: /: Is a directory\n")
    check("an out-of-range time is EINVAL", s.run("touch -d @99999999999 /data/file.txt"),
          "touch: setting times of '/data/file.txt': Invalid argument\n")
