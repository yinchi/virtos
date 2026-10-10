"""Start-up with nothing to read: no /etc/environment, no /etc/fstab, no profile. Each is a note on
the serial log, never fatal; the shell starts in / with its default prompt and an empty
environment, and the HOME disk is found but not mounted until asked for."""

SYSTEM_FILES = {"etc/environment": None, "etc/fstab": None}
HOME_FILES = {"hello.txt": "from home\n"}


def run(ctx):
    s, check = ctx.s, ctx.check
    log = s.boot_log

    check("a missing fstab is a note", "Fstab: /etc/fstab: not found -- nothing to mount." in log, True)
    check("a missing environment is a note",
          "Environment: /etc/environment: not found -- starting empty." in log, True)
    check("the shell starts in /", s.run("pwd"), "/\n")
    check("the environment is empty (the default PS1 is a shell variable, not exported)", s.run("env"), "")
    check("the default prompt is the working directory and # (there is only root)", s.log().endswith("/# "), True)
    check("the HOME disk is there but not mounted", s.run("lsblk"),
          "SIZE  LABEL   UUID       MOUNTPOINT\n64M   SYSTEM  0000-0000  /\n64M   HOME    5E6F-7A8B\n")
    check("/root is just an empty directory of the system image", s.run("ls -1 -a /root"), "")
    check("mounting it by hand", s.run("mount LABEL=HOME /root") + s.run("cat /root/hello.txt"), "from home\n")
    check("cd with no argument and no HOME goes to /", s.run("cd /usr") + s.run("cd") + s.run("pwd"), "/\n")
