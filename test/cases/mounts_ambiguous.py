"""A label or a volume ID is not unique: a disk cloned from an image has the original's ID, and
nothing stops two disks having the same label. Mounting by a name that more than one volume has is
refused, never "the first"; a name that is unique still works."""

HOME_FILES = {"which": "home\n"}
# `twin` has HOME's label and its own ID; `clone` has HOME's ID and its own label.
EXTRA_DISKS = [
    {"label": "HOME", "id": "0000aaaa", "files": {"which": "twin\n"}},
    {"label": "CLONE", "id": "5e6f7a8b", "files": {"which": "clone\n"}},
]


def run(ctx):
    s, check = ctx.s, ctx.check
    log = s.boot_log

    check("the fstab line for LABEL=HOME is skipped, with the reason",
          "Fstab: /etc/fstab: line 14: LABEL=HOME on /root: more than one volume has that label or ID -- skipped (UUID= or unmount the twin)." in log, True)
    check("nothing was mounted", "Fstab: 0 mount(s) from /etc/fstab." in log, True)
    def rows(command):
        """The lines a listing prints, header first, the rest sorted: the kernel numbers devices in
        the order QEMU gives them, which is not the order they were attached in."""
        header, *rest = s.run(command).split("\n")[:-1]
        return [header, *sorted(rest)]

    check("lsblk shows all four volumes, none mounted but the root", rows("lsblk"),
          ["SIZE  LABEL   UUID       MOUNTPOINT",
           "64M   CLONE   5E6F-7A8B",
           "64M   HOME    0000-AAAA",
           "64M   HOME    5E6F-7A8B",
           "64M   SYSTEM  0000-0000  /"])
    s.run("mkdir /m1")
    s.run("mkdir /m2")
    s.run("mkdir /m3")

    # ---------------------------------------------------------------- refused
    msg = "more than one volume has that label or ID (see lsblk)"
    check("a label two volumes have", s.run("mount LABEL=HOME /m1"), f"mount: LABEL=HOME on /m1: {msg}\n")
    check("a volume ID two volumes have", s.run("mount UUID=5E6F-7A8B /m1"), f"mount: UUID=5E6F-7A8B on /m1: {msg}\n")
    check("the label's case does not get around it", s.run("mount LABEL=home /m1"), f"mount: LABEL=home on /m1: {msg}\n")
    check("...nor does lower-case hex", s.run("mount UUID=5e6f-7a8b /m1"), f"mount: UUID=5e6f-7a8b on /m1: {msg}\n")
    check("a refused mount mounts nothing", s.run("mount"), "LABEL=SYSTEM on / type vfat\n")

    # ---------------------------------------------------------------- unique names still work
    check("the twin, by its unique ID", s.run("mount UUID=0000-AAAA /m1") + s.run("cat /m1/which"), "twin\n")
    check("the clone, by its unique label", s.run("mount LABEL=CLONE /m2") + s.run("cat /m2/which"), "clone\n")
    check("mount lists each by a spelling that names it alone",
          sorted(s.run("mount").split("\n")[:-1]),
          ["LABEL=CLONE on /m2 type vfat", "LABEL=SYSTEM on / type vfat", "UUID=0000-AAAA on /m1 type vfat"])
    check("with one of them mounted, the name is still ambiguous (it names devices, not mounts)",
          s.run("mount LABEL=HOME /m3"), f"mount: LABEL=HOME on /m3: {msg}\n")
    s.run("cd /")
    s.run("umount /m1")
    s.run("umount /m2")
    check("unmounting does not make an ID unique: the devices are still there",
          s.run("mount UUID=5E6F-7A8B /m3"), f"mount: UUID=5E6F-7A8B on /m3: {msg}\n")
