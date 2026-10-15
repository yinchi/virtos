"""Parameter expansion beyond `$NAME`: `~`, `${NAME:-w}` and its relatives, `${#NAME}`, and the
positional parameters (`$1`.., `$#`, `$@`, `$*`, `$0`, `shift`) of a script, `sh FILE ARGS`,
`source FILE ARGS` and a command substitution's subshell."""

SYSTEM_FILES = {
    "root/.keep": "",
}


def script(s, path, *lines):
    s.run(f"echo '#!/bin/sh' > {path}")
    for line in lines:
        s.run(f"echo '{line}' >> {path}")
    s.run(f"chmod +x {path}")


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("cd /tmp")

    # ----------------------------------------------------------------------- tilde
    check("~ is $HOME", s.run("echo ~"), "/root\n")
    check("~/ and a path", s.run("echo ~/x"), "/root/x\n")
    check("~name is just text (one user)", s.run("echo ~root ~x/y"), "~root ~x/y\n")
    check("a tilde in the middle of a word is text", s.run("echo a~ a~b"), "a~ a~b\n")
    check("quoted, it is text", s.run("echo '~' \"~\" \\~"), "~ ~ ~\n")
    check("an assignment's value starts with one", s.run("D=~/bin ; echo $D"), "/root/bin\n")
    check("...but not after a colon", s.run("D=a:~ ; echo $D"), "a:~\n")
    check("it is not split", s.run("HOME='/a b' ; echo ~ | wc -w ; HOME=/root"), "2\n")
    check("with HOME unset it stays a tilde", s.run("unset HOME ; echo ~ ; export HOME=/root"), "~\n")
    check("cd ~ goes home", s.run("cd ~ ; pwd ; cd /tmp"), "/root\n")

    # -------------------------------------------------------------- ${NAME op word}
    s.run("S=set")
    s.run("E=")
    s.run("unset U")
    check("${NAME} is the value", s.run("echo ${S}x"), "setx\n")
    check(":- uses the word for unset", s.run("echo ${U:-dflt}"), "dflt\n")
    check(":- uses the word for empty", s.run("echo ${E:-dflt}"), "dflt\n")
    check("- uses the word only for unset", s.run("echo [${E-dflt}] [${U-dflt}]"), "[] [dflt]\n")
    check("...and the value when there is one", s.run("echo ${S:-dflt} ${S-dflt}"), "set set\n")
    check(":+ uses the word when there is a value", s.run("echo [${S:+alt}] [${U:+alt}] [${E:+alt}]"), "[alt] [] []\n")
    check("+ counts an empty value as one", s.run("echo [${E+alt}] [${U+alt}]"), "[alt] []\n")
    check(":= assigns", s.run("echo ${A:=made} ; echo $A"), "made\nmade\n")
    check("...but not over a value", s.run("echo ${A:=other} $A"), "made made\n")
    check(":? stops with a message", s.run("echo ${U:?is needed}"), "U: is needed\n")
    check("...and with a default one", s.run("echo ${U:?}"), "U: parameter null or not set\n")
    check("...and the command did not run", s.run("echo before ${U:?x} after").count("before"), 0)
    check("it is status 1", s.status("echo ${U:?x}")[1], 1)
    check("the word may hold other expansions", s.run("echo ${U:-$S-$(echo sub)}"), "set-sub\n")
    check("...and nest", s.run("echo ${U:-${U2:-deep}}"), "deep\n")
    check("...and quotes", s.run("echo ${U:-'a  b'}"), "a  b\n")
    check("the word is not expanded when unused", s.run("echo ${S:-$(echo ran >&2)}").count("ran"), 0)
    check("unquoted, a default with blanks splits", s.run("echo ${U:-a b} | wc -w"), "2\n")
    check("quoted, it does not", s.run("echo \"${U:-a  b}\""), "a  b\n")
    check("an unquoted default with a wildcard globs", s.run("cd /data ; echo ${U:-*.txt} ; cd /tmp").endswith(".txt\n"), True)
    check("${#NAME} is the length", s.run("echo ${#S} ${#E} ${#U}"), "3 0 0\n")
    check("a malformed expansion is refused", s.run("echo ${S"), "bad substitution\n")
    check("...as is an empty one", s.run("echo ${}"), "bad substitution\n")

    # ------------------------------------------------------------ positional parameters
    script(s, "/tmp/args", 'echo "$0|$1|$2|$#"', 'echo "$@"', 'echo $*', 'shift', 'echo "$1|$#"')
    check("a script sees its arguments", s.run("/tmp/args a b c"),
          "/tmp/args|a|b|3\na b c\na b c\nb|2\n")
    check("with none, $1 is empty and $# is 0", s.run("/tmp/args").splitlines()[0], "/tmp/args|||0")
    script(s, "/tmp/quoting", 'echo [$1] [$2]', 'echo "[$@]"', 'echo "[$*]"')
    check('"$@" keeps each argument whole', s.run("/tmp/quoting 'a b' c"), "[a b] [c]\n[a b c]\n[a b c]\n")
    script(s, "/tmp/countargs", 'echo $#', 'echo "$@" | wc -w')
    check("a quoted argument with a blank is one", s.run("/tmp/countargs 'x y' z"), "2\n3\n")
    script(s, "/tmp/atonly", 'echo "<$@>"')
    check('"$@" with none vanishes', s.run("/tmp/atonly"), "<>\n")
    script(s, "/tmp/lenargs", 'echo ${#1} ${#@}', 'echo ${2:-none} ${1:-none}')
    check("${#1} and ${#@}", s.run("/tmp/lenargs hello"), "5 1\nnone hello\n")
    check("$10 is $1 then a zero", s.run("echo '#!/bin/sh' > /tmp/ten ; echo 'echo $10' >> /tmp/ten ; chmod +x /tmp/ten ; /tmp/ten A"), "A0\n")
    script(s, "/tmp/ten2", 'echo ${10}')
    check("${10} is the tenth", s.run("/tmp/ten2 1 2 3 4 5 6 7 8 9 ten"), "ten\n")

    # ------------------------------------------------------------------ shift
    script(s, "/tmp/shifty", 'shift 2', 'echo $1 $#', 'shift 5 ; echo $?', 'shift x ; echo $?')
    check("shift N drops N", s.run("/tmp/shifty a b c d").splitlines()[0], "c 2")
    check("shift past the end is status 1 and drops nothing", s.run("/tmp/shifty a b c d").splitlines()[-4:],
          ["shift: shift count out of range", "1", "shift: x: numeric argument required", "2"])

    # ------------------------------------------------------ sh FILE ARGS, source FILE ARGS
    check("sh FILE ARGS", s.run("sh /tmp/countargs one two three"), "3\n3\n")
    check("...with $0 as the file", s.run("sh /tmp/args x").splitlines()[0], "/tmp/args|x||1")
    check("source FILE ARGS", s.run("source /tmp/countargs p q"), "2\n2\n")
    check("...restores the caller's own afterwards", s.run("echo [$#]"), "[0]\n")
    check("the arguments of a script do not leak", s.run("echo [$1]"), "[]\n")
    script(s, "/tmp/outer", 'echo "outer:$1"', '/tmp/inner "$1-x"', 'echo "outer again:$1"')
    script(s, "/tmp/inner", 'echo "inner:$1"')
    check("each script has its own", s.run("/tmp/outer v"), "outer:v\ninner:v-x\nouter again:v\n")

    # ----------------------------------------------------------- inside a substitution
    script(s, "/tmp/insub", 'echo "$(echo $1 $#)"')
    check("a substitution sees the script's parameters", s.run("/tmp/insub a b"), "a 2\n")
    check("at the prompt there are none", s.run("echo [$#] [$0]"), "[0] [sh]\n")
