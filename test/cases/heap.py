"""`brk` and the user heap: the break moves in both directions with byte-exact results, grown
memory is zero (also across a shrink and across programs), requests outside the heap's range are
refused with the old break, a program's heap is gone once it ends, and `alloc` works for several
MiB. `test/progs`' `heap` makes the calls and prints one `name result` line each."""

TEST_PROGS = ["heap"]

EXPECTED = [
    "break-is-page-aligned-or-at-least-above-zero",
    "brk-zero-is-stable",
    "grow-by-a-page-and-a-byte-returns-the-request",
    "the-first-new-page-is-zero",
    "the-second-new-page-is-zero",
    "shrink-returns-the-request",
    "regrow-returns-the-request",
    "regrown-memory-above-the-shrink-is-zero",
    "regrown-second-page-is-zero",
    "below-the-heap-is-refused",
    "past-the-ceiling-is-refused",
    "exactly-the-ceiling-is-allowed",
    "back-down-after-the-ceiling",
    "an-8-mib-vec-is-intact",
    "the-freed-space-is-reusable",
]


def run(ctx):
    s, check = ctx.s, ctx.check

    s.run("chmod +x /tests/heap")
    out = s.run("/tests/heap", timeout=60)
    got = {}
    for line in out.splitlines():
        name, _, value = line.rpartition(" ")
        got[name] = int(value)
    for name in EXPECTED:
        check(name, got.get(name), 1)
    # The two values that are not booleans: a byte survives below a shrink, and one written to the
    # last mapped byte reads back.
    check("the last mapped byte is writable", got.get("the-last-mapped-byte-is-writable"), 7)
    check("memory below the shrink survived", got.get("memory-below-the-shrink-survived"), 9)
    check("a syscall pointer into an unmapped hole is EFAULT", got.get("syscall-pointer-into-an-unmapped-hole"), -14)
    check("a syscall may not write into the program's read-only code", got.get("syscall-write-into-read-only-code"), -14)
    check("nothing else printed", [n for n in got if n not in EXPECTED and not n.startswith(("the-last", "memory-below", "syscall-pointer", "syscall-write"))], [])

    # ------------------------------------------------------- one program's heap is not the next's
    addr = s.run("/tests/heap dirty").strip()
    check("a program can grow and fill its heap", addr.isdigit(), True)
    out, status = s.status(f"/tests/heap peek {addr}")
    check("the next program cannot read the previous program's heap", out.startswith("Segmentation fault (address "), True)
    check("...status 139", status, 139)
    check("a fresh program's grown heap is zero, not the previous one's bytes", s.run("/tests/heap clean"), "true\n")
    check("the shell goes on", s.run("echo ok"), "ok\n")
