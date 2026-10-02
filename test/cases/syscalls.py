"""The system calls no shell command can make badly: an unknown number, pointers outside the
program's memory, descriptors that are not open, arguments that are out of range -- and what a
program that faults does to the shell. `test/progs`' `badcalls` makes each call and prints the
negated errno it got; the table below is what each one should be."""

TEST_PROGS = ["badcalls", "fault"]
# A volume nothing mounts: the mount calls that fail on their target need one.
EXTRA_DISKS = [{"label": "SPARE", "id": "1234abcd"}]

# Linux's numbers (`abi::errno`).
ENOENT, EBADF, EACCES, EFAULT, EBUSY, EEXIST, EXDEV, ENOTDIR, EISDIR, EINVAL, EMFILE, ERANGE = (
    -2, -9, -13, -14, -16, -17, -18, -20, -21, -22, -24, -34)
ENODEV, ENOSYS, ENOTEMPTY = -19, -38, -39

EXPECTED = [
    # ---------------------------------------------------------------- no such call
    ("unknown-syscall", ENOSYS),
    ("syscall-zero", ENOSYS),
    # ------------------------------------------------------- pointers outside the program
    ("write-low-pointer", EFAULT),
    ("write-kernel-pointer", EFAULT),
    ("write-wrapping-range", EFAULT),
    ("write-null-empty", EFAULT),
    ("read-low-pointer", EFAULT),
    ("open-low-path", EFAULT),
    ("mkdir-kernel-path", EFAULT),
    ("unlink-low-path", EFAULT),
    ("rename-low-new", EFAULT),
    ("getcwd-low-buffer", EFAULT),
    ("stat-low-output", EFAULT),
    ("mount-low-source", EFAULT),
    ("blkinfo-low-output", EFAULT),
    ("utimensat-low-times", EFAULT),
    # ------------------------------------------------------------------- bad descriptors
    ("write-fd-not-open", EBADF),
    ("write-fd-out-of-range", EBADF),
    ("read-fd-not-open", EBADF),
    ("close-fd-not-open", EBADF),
    ("close-fd-out-of-range", EBADF),
    ("write-to-stdin", EBADF),
    ("read-from-stdout", EBADF),
    ("getdents-on-stdout", ENOTDIR),
    # -------------------------------------------------------------------------- open
    ("open-bad-flags", EINVAL),
    ("open-not-utf8", EINVAL),
    ("open-missing", ENOENT),
    ("open-through-a-file", ENOTDIR),
    ("open-write-missing-directory", ENOENT),
    ("open-write-a-directory", EISDIR),
    ("write-to-a-read-only-file", EBADF),
    ("getdents-on-a-file", ENOTDIR),
    ("read-a-directory", EISDIR),
    ("write-a-directory", EISDIR),
    ("getdents-buffer-too-small", EINVAL),
    ("close-a-file", 0),
    ("close-it-twice", EBADF),
    ("close-a-directory", 0),
    ("open-until-full-count", 8),  # the slots above the three standard ones
    ("open-when-full", EMFILE),
    # ------------------------------------------------------------------ paths
    ("mkdir-existing", EEXIST),
    ("mkdir-root", EEXIST),
    ("mkdir-mount-point", EEXIST),
    ("mkdir-missing-parent", ENOENT),
    ("unlink-missing", ENOENT),
    ("unlink-root", EBUSY),
    ("unlink-mount-point", EBUSY),
    ("unlink-non-empty-directory", ENOTEMPTY),
    ("rename-missing", ENOENT),
    ("rename-into-itself", EINVAL),
    ("rename-across-volumes", EXDEV),
    ("rename-mount-point", EBUSY),
    ("rename-onto-mount-point", EBUSY),
    ("getcwd-too-small", ERANGE),
    ("stat-missing", ENOENT),
    ("stat-through-a-file", ENOTDIR),
    # ------------------------------------------------------------ chmod, utimensat
    ("chmod-directory-bit", EINVAL),
    ("chmod-bit-clear-too", EINVAL),
    ("chmod-more-than-a-byte", EINVAL),
    ("chmod-the-root", EINVAL),
    ("chmod-missing", ENOENT),
    ("utimensat-missing", ENOENT),
    ("utimensat-the-root", EINVAL),
    ("utimensat-bad-nanoseconds", EINVAL),
    ("utimensat-before-1980", EINVAL),
    ("utimensat-after-2107", EINVAL),
    # -------------------------------------------------------------- mount, umount
    ("mount-not-a-source", EINVAL),
    ("mount-no-such-volume", ENODEV),
    ("mount-already-mounted", EBUSY),
    ("mount-target-missing", ENOENT),
    ("mount-target-a-file", ENOTDIR),
    ("umount-not-a-mount-point", EINVAL),
    ("umount-the-root", EBUSY),
    ("umount-nothing", EINVAL),
    # ---------------------------------------------------------- blkinfo, reboot
    ("blkinfo-past-the-last", ENODEV),
    ("reboot-unknown-command", EINVAL),
    ("reboot-wide-command", EINVAL),
]


def run(ctx):
    s, check = ctx.s, ctx.check

    s.run("chmod +x /tests/badcalls /tests/fault")
    s.run("cd /")
    out = s.run("/tests/badcalls", timeout=60)
    got = {}
    for line in out.splitlines():
        name, _, value = line.rpartition(" ")
        got[name] = int(value)
    for name, want in EXPECTED:
        check(name, got.get(name), want)
    check("every call printed a result, and nothing else did", [n for n in got if n not in dict(EXPECTED)], [])
    check("the program ran to its end", s.run("echo $?"), "0\n")
    check("the kernel is still alive and the shell too", s.run("echo ok"), "ok\n")

    # ------------------------------------------------------------------ deliberate faults
    for how, what in (("null", "a write to address 0"), ("jump", "a jump to address 0"),
                      ("kernel", "a read of kernel memory"), ("below", "a read just below the program's memory")):
        out, status = s.status(f"/tests/fault {how}")
        check(f"{what} is a segmentation fault", out.startswith("Segmentation fault (address "), True)
        check(f"...status 139", status, 139)
        check(f"...and the shell goes on", s.run("echo ok"), "ok\n")
    check("without an argument it does nothing and exits 2", s.status("/tests/fault"), ("", 2))
