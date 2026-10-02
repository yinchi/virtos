"""hello, clear, reboot and poweroff: the programs that act on the machine itself."""

import time


def run(ctx):
    s, check = ctx.s, ctx.check

    check("hello", s.run("hello"), "hello from EL0!\n")
    check("hello ignores its arguments, --help included", s.run("hello --help"), "hello from EL0!\n")
    s.run("clear")
    check("clear leaves the shell working", s.run("echo after-clear"), "after-clear\n")

    # ---------------------------------------------------------------- reboot
    s.run("echo survives > /root/before")
    s.start("reboot")
    s.wait_until(lambda t: t.count("Hello Uart!") >= 1, "the kernel to boot again")
    s.wait_prompt()
    check("reboot restarts the machine (a second boot banner)", s.log().count("Hello Uart!"), 2)
    check("the shell is back in HOME", s.run("pwd"), "/root\n")
    check("files written before the reboot survive it", s.run("cat /root/before"), "survives\n")
    check("the clock is still real after a reboot", s.run("date -u +%Y", timeout=90).strip().isdigit(), True)

    # ---------------------------------------------------------------- poweroff
    s.start("poweroff")
    deadline = time.time() + 20
    while s.qemu.poll() is None and time.time() < deadline:
        time.sleep(0.1)
    check("poweroff makes QEMU exit", s.qemu.poll(), 0)
