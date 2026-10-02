"""The shortest end-to-end check: boot, run a command, a pipeline, and read a file back."""


def run(ctx):
    s, check = ctx.s, ctx.check
    check("echo prints its arguments", s.run("echo hello world"), "hello world\n")
    check("a pipeline runs", s.run("seq 3 | sort -r"), "3\n2\n1\n")
    check("a redirect writes a file", s.run("echo hi > /tmp/f"), "")
    check("...and cat reads it back", s.run("cat /tmp/f"), "hi\n")
