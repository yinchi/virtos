"""A profile with failing commands: it runs to the end, what it prints reaches the console before
the first prompt, and $? is left as its last command set it."""

HOME_FILES = {".profile": "echo from-profile\nnosuchcmd\ncat /nosuch\nPR=1\n"}


def run(ctx):
    s, check = ctx.s, ctx.check
    log = s.boot_log

    check("what the profile prints is on the log before the first prompt", "from-profile\n" in log, True)
    check("a failing line is reported and the profile goes on",
          "nosuchcmd: command not found\ncat: /nosuch: No such file or directory\n" in log, True)
    check("the last line ran", s.run("echo $PR"), "1\n")
    check("$? is the profile's last command's status (an assignment: 0)", s.run("echo $?"), "0\n")
    check("the shell is in HOME", s.run("pwd"), "/root\n")
