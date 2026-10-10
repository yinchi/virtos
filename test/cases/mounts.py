"""Mounting: lsblk, mount, umount, the rules a mount follows, and what the errors say."""

HOME_FILES = {"hello.txt": "from home\n", "dir/inner.txt": "inner\n"}

LSBLK = "SIZE  LABEL   UUID       MOUNTPOINT\n64M   SYSTEM  0000-0000  /\n64M   HOME    5E6F-7A8B  /root\n"


def run(ctx):
    s, check = ctx.s, ctx.check

    check("lsblk lists both devices", s.run("lsblk"), LSBLK)
    check("lsblk -b prints bytes", s.run("lsblk -b"),
          "SIZE      LABEL   UUID       MOUNTPOINT\n67108864  SYSTEM  0000-0000  /\n67108864  HOME    5E6F-7A8B  /root\n")
    check("mount with no operands lists the mounts", s.run("mount"),
          "LABEL=SYSTEM on / type vfat\nLABEL=HOME on /root type vfat\n")
    check("the root volume's own contents", s.run("ls -1 /"), "README\netc\nroot\ntest\ntmp\nusr\n")
    check("the HOME volume answers for /root", s.run("ls -1 /root"), "dir\nhello.txt\n")
    check("...and reads work through the mount", s.run("cat /root/hello.txt /root/dir/inner.txt"), "from home\ninner\n")
    check("stat of the mount point is a directory", s.run("stat /root").split("\n")[1], "  Size: 0            Type: directory")
    check("a mount point is not a file to remove", s.run("rm -r /root/dir") + s.run("ls -1 /root"), "hello.txt\n")

    # ------------------------------------------------------------ busy
    check("umount is refused while the working directory is in the volume",
          s.run("umount /root"), "umount: /root: Device or resource busy\n")
    check("...with status 1", s.status("umount /root")[1], 1)
    s.run("cd /")
    check("umount is refused while a file on it is open (a redirect target)",
          s.run("umount /root > /root/log"), "umount: /root: Device or resource busy\n")
    check("the root cannot be unmounted", s.run("umount /"), "umount: /: Device or resource busy\n")
    check("umount of a plain directory", s.run("umount /tmp"), "umount: /tmp: not a mount point\n")
    check("umount of nothing", s.run("umount /nosuch"), "umount: /nosuch: not a mount point\n")
    check("umount with no operand", s.run("umount").startswith("umount: missing operand"), True)

    # ------------------------------------------------------------ unmount and mount again
    check("umount works once nothing holds the volume", s.run("umount /root"), "")
    check("the mount is gone from the list", s.run("mount"), "LABEL=SYSTEM on / type vfat\n")
    check("...and lsblk shows no mount point", s.run("lsblk").endswith("64M   HOME    5E6F-7A8B\n"), True)
    check("/root is the image's empty directory again", s.run("ls -1 /root"), "")
    check("mount by label", s.run("mount LABEL=HOME /root") + s.run("ls -1 /root"),
          "hello.txt\nlog\n")  # `log` is the file the busy umount's redirect created
    check("a mount already made cannot be repeated", s.run("mount LABEL=HOME /tmp"),
          "mount: LABEL=HOME on /tmp: Device or resource busy\n")
    s.run("cd /")
    s.run("umount /root")
    s.run("mkdir /mnt")
    check("mount by UUID, at another directory", s.run("mount UUID=5E6F-7A8B /mnt") + s.run("ls -1 /mnt"), "hello.txt\nlog\n")
    check("a volume is mounted once: its old place is empty", s.run("ls -1 /root"), "")
    check("mount lists the new place", s.run("mount"), "LABEL=SYSTEM on / type vfat\nLABEL=HOME on /mnt type vfat\n")
    s.run("umount /mnt")

    # ------------------------------------------------------------ errors
    check("no volume has that label", s.run("mount LABEL=NOPE /mnt"), "mount: LABEL=NOPE on /mnt: no volume has that label or ID\n")
    check("no volume has that UUID", s.run("mount UUID=0000-1111 /mnt"), "mount: UUID=0000-1111 on /mnt: no volume has that label or ID\n")
    check("a source that is neither spelling", s.run("mount /dev/vdb /mnt"),
          "mount: /dev/vdb on /mnt: not a LABEL= or UUID= source, or not a FAT filesystem\n")
    check("a malformed UUID", s.run("mount UUID=12 /mnt"),
          "mount: UUID=12 on /mnt: not a LABEL= or UUID= source, or not a FAT filesystem\n")
    check("a target that does not exist", s.run("mount LABEL=HOME /nosuch"), "mount: LABEL=HOME on /nosuch: No such file or directory\n")
    check("a target that is a file", s.run("mount LABEL=HOME /etc/fstab"), "mount: LABEL=HOME on /etc/fstab: Not a directory\n")
    check("the root volume cannot be mounted again", s.run("mount LABEL=SYSTEM /mnt"),
          "mount: LABEL=SYSTEM on /mnt: Device or resource busy\n")
    check("a one-operand mount", s.run("mount LABEL=HOME").startswith("mount: missing operand after 'LABEL=HOME'"), True)
    check("a three-operand mount", s.run("mount a b c").startswith("mount: extra operand 'c'"), True)
    check("a target name too long", s.run("mount LABEL=HOME /" + "x" * 300), "mount: LABEL=HOME on /" + "x" * 300 + ": File name too long\n")
    check("a failed mount leaves the table alone", s.run("mount"), "LABEL=SYSTEM on / type vfat\n")
    check("lsblk with a bad flag", s.run("lsblk -z"), "lsblk: invalid option -- 'z'\nTry 'lsblk --help' for more information.\n")
