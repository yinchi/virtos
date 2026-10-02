"""Two volumes are labelled SYSTEM (the HOME disk is relabelled): a label is not a unique name, so
the kernel refuses to boot rather than pick one -- it could boot from, and write to, the wrong disk."""

HOME_LABEL = "SYSTEM"
BOOT_FAILS = True


def run(ctx):
    s, check = ctx.s, ctx.check
    log = s.wait_for_log("Panic occurred!")
    check("both devices are listed as labelled SYSTEM, neither as the root",
          log.count("(labelled SYSTEM, like another volume)") == 2 and "(root)" not in log, True)
    check("the refusal says why", "cannot choose a root volume: 2 FAT volumes are labelled SYSTEM (exactly one must be)" in log, True)
    check("the shell never starts", "Fstab:" in log or "Environment:" in log, False)
