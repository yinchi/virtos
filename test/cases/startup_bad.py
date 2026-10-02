"""Start-up with bad configuration: malformed environment and fstab lines, an unusable HOME. Each
problem is one note on the serial log, naming the line; the good lines still count."""

SYSTEM_FILES = {
    "etc/environment": "HOME=/nosuch\nbad line\n1X=2\n=v\nFOO = bar\nGOOD=1\n",
    "etc/fstab": (
        "LABEL=HOME /root vfat defaults\n"
        "LABEL=HOME /mnt vfat ro\n"
        "short line\n"
        "LABEL=GONE /tmp vfat nofail\n"
        "LABEL=GONE /usr vfat defaults\n"
        "LABEL=HOME /nosuchdir vfat auto\n"
        "UUID=5E6F-7A8B /mnt vfat noauto\n"
        "LABEL=SYSTEM / vfat defaults\n"
        "LABEL=HOME / vfat defaults\n"
        "UUID=nonsense /x vfat defaults\n"
    ),
}


def run(ctx):
    s, check = ctx.s, ctx.check
    log = s.boot_log

    def note(text):
        return text in log

    check("a line with no =", note("Environment: /etc/environment: line 2: expected NAME=VALUE (no '=') -- ignored."), True)
    check("an invalid name", note("Environment: /etc/environment: line 3: not a valid variable name -- ignored."), True)
    check("an empty name", note("Environment: /etc/environment: line 4: expected NAME=VALUE (empty name) -- ignored."), True)
    check("the good lines count", note("Environment: 3 variable(s) from /etc/environment."), True)
    check("blanks around the name are trimmed, the value is literal", s.run("printenv FOO"), " bar\n")
    check("a HOME that is not a directory is a note",
          note("Environment: cannot enter HOME=/nosuch: not a directory -- staying in /."), True)
    check("...and the shell starts in /", s.run("pwd"), "/\n")

    check("ro is refused", note("Fstab: /etc/fstab: line 2: read-only mounts are not supported -- ignored."), True)
    check("a short line", note("Fstab: /etc/fstab: line 3: expected <source> <mount point> <type> <options> -- ignored."), True)
    check("a missing volume with nofail is silent", note("LABEL=GONE on /tmp"), False)
    check("a missing volume otherwise is a note",
          note("Fstab: /etc/fstab: line 5: LABEL=GONE on /usr: no volume has that label or ID -- skipped."), True)
    check("a volume already mounted", note("Fstab: /etc/fstab: line 6: LABEL=HOME on /nosuchdir: Device or resource busy -- skipped."), True)
    check("noauto lines are left alone", note("/mnt"), False)
    check("a root line naming the root is fine", note("LABEL=SYSTEM on /"), False)
    check("a root line naming another volume is ignored",
          note("Fstab: /etc/fstab: line 9: LABEL=HOME on /: the root is chosen at boot (the volume labelled SYSTEM), not from this file -- ignored."), True)
    check("a bad source", note("line 10"), True)
    check("one mount happened", note("Fstab: 1 mount(s) from /etc/fstab."), True)
    check("it is the HOME disk at /root", s.run("mount"), "LABEL=SYSTEM on / type vfat\nLABEL=HOME on /root type vfat\n")
