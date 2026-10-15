"""Filename expansion: `*`, `?`, `[…]` (ranges, `!`/`^`, POSIX classes) in unquoted words. Matches
are sorted bytewise and become separate arguments; a wildcard never matches `/`; a leading `.` must
be written; a pattern that matches nothing stays as typed; quoted or escaped characters are
literal -- also in the result of an expansion, which is a pattern only when it is unquoted.
Matching is case-sensitive."""

SYSTEM_FILES = {
    "g/a.c": "1",
    "g/b.c": "2",
    "g/Z.c": "3",
    "g/ab.c": "4",
    "g/c1.txt": "5",
    "g/c2.txt": "6",
    "g/c10.txt": "7",
    "g/.hidden": "8",
    "g/with space.txt": "9",
    "g/litx": "10",
    "g/lit[x]": "15",
    "g/sub/x.c": "11",
    "g/sub/y.h": "12",
    "g/sub/deep/z.c": "13",
    "g/other/q.c": "14",
}


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("cd /g")

    # ------------------------------------------------------------------ the basics
    check("* matches any run, sorted bytewise", s.run("echo *.c"), "Z.c a.c ab.c b.c\n")
    check("? is exactly one character", s.run("echo ?.c"), "Z.c a.c b.c\n")
    check("[abc] is a set", s.run("echo [ab].c"), "a.c b.c\n")
    check("[a-b] is a range", s.run("echo [a-b].c"), "a.c b.c\n")
    check("[!a] is a complement", s.run("echo [!a].c"), "Z.c b.c\n")
    check("[^a] is too", s.run("echo [^a].c"), "Z.c b.c\n")
    check("POSIX classes", s.run("echo c[[:digit:]].txt"), "c1.txt c2.txt\n")
    check("...and a class inside a negated set", s.run("echo c[![:alpha:]]*.txt"), "c1.txt c10.txt c2.txt\n")
    check("matches are separate arguments", s.run("echo *.txt | wc -w"), "5\n")

    # --------------------------------------------------------------------- dot names
    check("* skips dot-names", s.run("echo *").count(".hidden"), 0)
    check(".* matches them", s.run("echo .*"), ".hidden\n")
    check("? and [] do not match a leading dot either", s.run("echo ?hidden [.]hidden"), "?hidden [.]hidden\n")

    # ---------------------------------------------------------------- directories
    check("a wildcard does not match a slash", s.run("echo s*/x.c"), "sub/x.c\n")
    check("...so one level is one component", s.run("echo */*.c"), "other/q.c sub/x.c\n")
    check("a pattern in the middle of a path", s.run("echo */deep/*.c"), "sub/deep/z.c\n")
    check("an absolute pattern stays absolute", s.run("echo /g/s*/*.h"), "/g/sub/y.h\n")
    check("a trailing slash keeps only directories", s.run("echo */"), "other/ sub/\n")
    check("...from another directory too", s.run("cd sub ; echo *.c ; cd .."), "x.c\n")

    # ---------------------------------------------------------------- no match
    check("a pattern that matches nothing stays as typed", s.run("echo *.zzz"), "*.zzz\n")
    check("...also in a directory that is not there", s.run("echo nosuch/*.c"), "nosuch/*.c\n")
    check("matching is case-sensitive", s.run("echo *.C"), "*.C\n")

    # --------------------------------------------------------------------- quoting
    check("double quotes make a wildcard literal", s.run('echo "*.c"'), "*.c\n")
    check("single quotes do too", s.run("echo '*.c'"), "*.c\n")
    check("a backslash does", s.run("echo \\*.c"), "*.c\n")
    check("a quoted part next to a wildcard", s.run('echo "a"*.c'), "a.c ab.c\n")
    # (FAT forbids `*` and `?` in names, but allows `[`: `lit[x]` and `litx` both exist.)
    check("unquoted, [x] is a set: it finds litx, not lit[x]", s.run("echo lit[x]"), "litx\n")
    check("an escaped bracket is the character", s.run("echo lit\\[x]"), "lit[x]\n")
    check("...so is a quoted one, and it is that file", s.run('cat "lit[x]"'), "15")
    check("a name with a blank is one argument", s.run("echo with*.txt | wc -w"), "2\n")
    check("...and survives as one for a program", s.run("cat with*.txt"), "9")
    check("a command word can be a pattern", s.run("/usr/bin/ec?o hi"), "hi\n")

    # ------------------------------------------------------------------ expansions
    check("an unquoted variable's value is a pattern", s.run("P='*.c' ; echo $P"), "Z.c a.c ab.c b.c\n")
    check("a quoted one is not", s.run("P='*.c' ; echo \"$P\""), "*.c\n")
    check("an unquoted substitution's result is a pattern", s.run("echo $(echo '*.c')"), "Z.c a.c ab.c b.c\n")
    check("a quoted one is not", s.run("echo \"$(echo '*.c')\""), "*.c\n")
    check("a value with blanks splits and each piece globs", s.run("P='a.c b.c' ; echo $P"), "a.c b.c\n")
    check("...with a wildcard in one piece", s.run("P='sub/*.c a.c' ; echo $P"), "sub/x.c a.c\n")
    check("an assignment's value is never globbed", s.run("X=*.c ; echo \"$X\""), "*.c\n")
    s.run("echo old > /tmp/f1")
    s.run("echo new > /tmp/f[1]")
    check("a redirect target is not globbed: f[1] is a name, not f1", s.run("cat /tmp/f1"), "old\n")
    check("...it made a file of that name", s.run("cat '/tmp/f[1]'"), "new\n")

    # ---------------------------------------------------------------- composition
    check("ls takes the expansion as operands", s.run("ls -1 -d s* o*"), "other\nsub\n")
    check("a failing command with a pattern still sees the names", s.run("cat *.zzz").endswith("No such file or directory\n"), True)
