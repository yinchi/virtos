"""A profile that is a directory (not a script) is a note and is skipped."""

HOME_FILES = {".profile/inner": "x\n"}


def run(ctx):
    s, check = ctx.s, ctx.check
    check("the profile being a directory is a note", "Profile: /root/.profile: is a directory -- skipped." in s.boot_log, True)
    check("the shell is up and in HOME", s.run("pwd"), "/root\n")
