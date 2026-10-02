"""No volume is labelled SYSTEM (the system image is relabelled): the kernel refuses to boot rather
than guess which disk is the system -- the first FAT volume might be the HOME disk."""

SYSTEM_LABEL = "OTHER"
BOOT_FAILS = True


def run(ctx):
    s, check = ctx.s, ctx.check
    log = s.wait_for_log("Panic occurred!")
    check("the device table is listed, with no root marked", "(root)" in log, False)
    check("the refusal says why", "cannot choose a root volume: no FAT volume is labelled SYSTEM (exactly one must be)" in log, True)
    check("the shell never starts", "Fstab:" in log or "Environment:" in log, False)
