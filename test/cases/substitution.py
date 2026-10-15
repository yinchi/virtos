"""Command substitution: `$(cmd)` and backticks. The inside is a subshell whose stdout is the
value: every trailing newline is stripped, a quoted result is one word, an unquoted one is split on
blanks (and vanishes if empty). `$((` is refused, nesting is capped, output is capped. A
redirection target is expanded too -- variables and substitutions, never split."""

SYSTEM_FILES = {
    "data/words.txt": "alpha beta\ngamma\n\n\n",
    "data/two.txt": "one\ntwo\n",
}


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("cd /tmp")

    # ----------------------------------------------------------------- the basics
    check("a substitution is replaced by its output", s.run("echo $(echo hi)"), "hi\n")
    check("backticks do the same", s.run("echo `echo hi`"), "hi\n")
    check("it glues to the text around it", s.run("echo a$(echo X)b"), "aXb\n")
    check("every trailing newline goes", s.run("echo \"[$(cat /data/words.txt)]\""), "[alpha beta\ngamma]\n")
    check("unquoted, the result splits on blanks and newlines", s.run("echo $(cat /data/words.txt)"),
          "alpha beta gamma\n")
    check("...into separate arguments", s.run("echo $(cat /data/two.txt) | wc -w"), "2\n")
    check("quoted, it is one word", s.run("cat \"$(echo /data/two.txt)\""), "one\ntwo\n")
    check("an empty unquoted result is no argument at all", s.run("echo a $(true) b"), "a b\n")
    check("an empty quoted one is an empty argument", s.run("echo a \"$(true)\" b"), "a  b\n")
    check("inner newlines of a quoted result survive", s.run("echo \"$(cat /data/two.txt)\""), "one\ntwo\n")
    check("single quotes make it text", s.run("echo '$(echo hi)'"), "$(echo hi)\n")
    check("so does a backslash before the dollar of a quoted one", s.run("echo \"\\$(echo hi)\""), "$(echo hi)\n")

    # ------------------------------------------------------------------- nesting
    check("they nest", s.run("echo $(echo $(echo deep))"), "deep\n")
    check("an inner quote does not end the outer one", s.run("echo \"$(echo \"a  b\")\""), "a  b\n")
    check("a ) in inner quotes is not the end", s.run("echo $(echo ')' x)"), ") x\n")
    check("backticks nest when escaped", s.run("echo `echo \\`echo in\\``"), "in\n")
    check("a substitution is a whole list", s.run("echo $(echo a; echo b && echo c)"), "a b c\n")
    check("...with pipes and redirects inside", s.run("echo $(cat /data/two.txt | wc -l)"), "2\n")

    # ------------------------------------------------------- variables and scoping
    check("a variable is seen inside", s.run("X=1 ; echo $(echo $X)"), "1\n")
    check("an assignment inside does not leak out", s.run("X=1 ; echo $(X=2 ; echo $X) $X"), "2 1\n")
    check("a cd inside does not leak out", s.run("cd /tmp ; echo $(cd /data ; pwd) $(pwd)"), "/data /tmp\n")
    check("an export inside does not leak out", s.run("echo $(export Q=1 ; echo set) ; echo [$Q]"), "set\n[]\n")
    check("an assignment value is not split", s.run("A=$(cat /data/words.txt) ; echo \"$A\""), "alpha beta\ngamma\n")
    check("...nor is a mixed one", s.run("A=x$(echo 1 2)y ; echo \"$A\""), "x1 2y\n")

    # ------------------------------------------------------------------- status
    check("a command with no word takes the substitution's status", s.status("X=$(false)")[1], 1)
    check("...0 for a successful one", s.status("X=$(true)")[1], 0)
    check("...the last one counts", s.status("X=$(false)$(true)")[1], 0)
    check("a plain assignment is 0", s.status("X=a")[1], 0)
    check("a command with a word has its own status", s.status("echo $(false)")[1], 0)
    check("$? seen inside is the one before", s.run("false ; echo $(echo $?)"), "1\n")
    check("...and the substitution does not change the outer one", s.run("false ; echo $(true) $?"), "1\n")
    check("$(exit-less failure) status shows in &&", s.run("X=$(false) && echo ran || echo failed"), "failed\n")
    check("a failed program inside is just its status", s.status("X=$(nosuchcmd)")[1], 127)

    # ------------------------------------------------------------ stdout and stderr
    check("stderr is not captured", s.run("echo \"[$(cat /nosuch)]\""),
          "cat: /nosuch: No such file or directory\n[]\n")
    s.run("cd /tmp")
    s.run("cd /data")  # OLDPWD is now /tmp
    check("a builtin's stdout is captured", s.run("echo \"[$(cd -)]\""), "[/tmp]\n")
    check("...without moving the shell", s.run("pwd"), "/data\n")
    s.run("cd - > /tmp/cdout")  # back to /tmp; what it prints goes to the file
    check("cd - writes to its redirect", s.run("cat /tmp/cdout"), "/tmp\n")
    check("...and into a pipe", s.run("cd /data ; cd - | cat"), "/tmp\n")
    s.run("cd /tmp")

    # --------------------------------------------------------- redirection targets
    check("a variable is a redirect target", s.run("F=/tmp/t1 ; echo x > $F ; cat /tmp/t1"), "x\n")
    check("a substitution is a redirect target", s.run("echo y > $(echo /tmp/t2) ; cat /tmp/t2"), "y\n")
    check("...in input too", s.run("cat < $(echo /data/two.txt)"), "one\ntwo\n")
    check("a target with blanks is ambiguous", s.run("F='a b' ; echo x > $F"), "$F: ambiguous redirect\n")
    check("an empty one is ambiguous", s.run("E= ; echo x > $E"), "$E: ambiguous redirect\n")
    check("...but quoted it names a file (and an empty name is not one)", s.run("echo x > \"$E\"").endswith("\n"), True)
    check("an ambiguous redirect runs nothing and is status 1", s.status("E= ; echo x > $E")[1], 1)

    # ---------------------------------------------------------------- the refusals
    check("$(( is refused by name", s.run("echo $((1+2))"), "'$((' is not supported yet\n")
    check("...also inside quotes", s.run("echo \"$((1))\""), "'$((' is not supported yet\n")
    check("an unterminated substitution is refused", s.run("echo $(echo a"), "unterminated command substitution\n")
    check("...and an unterminated backtick", s.run("echo `echo a"), "unterminated command substitution\n")
    check("a syntax error inside runs nothing on the line", s.run("echo before ; echo $(echo a &&)"),
          "no command after '&&'\n")
    check("a comment inside swallows the )", s.run("echo $(echo a # c)"), "unterminated command substitution\n")
    check("an empty one is just empty", s.run("echo [$()]"), "[]\n")

    # ------------------------------------------------------------------ the caps
    deep = "echo hi"
    for _ in range(17):
        deep = f"echo $({deep})"
    # The innermost command that cannot run its substitution reports and fails; each level above
    # it just sees an empty result, so the outermost `echo` prints an empty line.
    check("nesting past the cap is refused", s.run(deep), "command substitution nested too deeply\n\n")
    ok = "echo hi"
    for _ in range(5):
        ok = f"echo $({ok})"
    check("a reasonable nesting works", s.run(ok), "hi\n")
    # 1 MiB cap: a file of exactly that much is fine, one byte more is not.
    s.run("cd /tmp")
    s.run("seq 1 200000 > big")  # about 1.2 MB
    check("output past the cap is an error, and the command does not run",
          s.run("echo $(cat /tmp/big) | wc -c").startswith("command substitution: output too large\n"), True)
    check("the temp file is gone", s.run("ls -1 -a /tmp").count(".pipe"), 0)
    check("the shell goes on", s.run("echo ok"), "ok\n")
