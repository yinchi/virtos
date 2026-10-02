"""The normal start-up: /etc/fstab mounts HOME at /root, /etc/environment is read (quotes, comments,
every variable exported), the shell enters $HOME, and ~/.profile runs before the first prompt."""

SYSTEM_FILES = {
    "etc/environment": (
        "# a comment\n"
        "HOME=/root\n"
        "PS1='\\w> '\n"
        "TZ=Asia/Tokyo\n"
        "Q=\"quoted value\"\n"
        "S='  spaced  '\n"
        "EQ=a=b\n"
        "LIT=$HOME\\n\n"
        "\n"
    ),
}
HOME_FILES = {".profile": "PROFVAR=set\nexport PROFEXP=1\ncd /tmp\n"}


def run(ctx):
    s, check = ctx.s, ctx.check
    log = s.boot_log

    check("the fstab line mounted one volume", "Fstab: 1 mount(s) from /etc/fstab." in log, True)
    check("seven variables were read", "Environment: 7 variable(s) from /etc/environment." in log, True)
    check("nothing at start-up was a problem", ("ignored" in log) or ("skipped" in log), False)
    check("the profile cd'd, so the shell starts in /tmp", s.run("pwd"), "/tmp\n")
    check("the profile's assignment is a shell variable", s.run("echo $PROFVAR"), "set\n")
    check("...not exported", s.run("printenv PROFVAR"), "")
    check("the profile's export is in the environment", s.run("printenv PROFEXP"), "1\n")
    check("every /etc/environment variable is exported", s.run("printenv TZ"), "Asia/Tokyo\n")
    check("a double-quoted value loses its quotes", s.run("printenv Q"), "quoted value\n")
    check("a single-quoted value keeps its blanks", s.run("printenv S"), "  spaced  \n")
    check("only the first = splits the line", s.run("printenv EQ"), "a=b\n")
    check("the value is literal: no $ expansion, no escapes", s.run("printenv LIT"), "$HOME\\n\n")
    check("a comment is not a variable", s.run("printenv comment"), "")
    check("cd with no argument goes to $HOME", s.run("cd") + s.run("pwd"), "/root\n")
    check("HOME is the mounted volume, not the image's empty directory",
          s.run("ls -a /root"), ".profile\n")
    check("both volumes are mounted", s.run("mount"), "LABEL=SYSTEM on / type vfat\nLABEL=HOME on /root type vfat\n")
    s.run("cd /usr")
    check("the prompt shows the working directory and uses the PS1 from the environment",
          s.log().endswith("/usr> "), True)
