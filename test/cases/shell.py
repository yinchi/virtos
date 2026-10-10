"""The shell language: quoting, `$VAR` expansion and field splitting, assignments, the exit status,
redirection, pipelines, and the errors a bad line gives. Programs are only used as probes here;
their own behavior is in `utils.py` and `filters.py`."""


def run(ctx):
    s, check = ctx.s, ctx.check

    # ---------------------------------------------------------------- words and quoting
    check("words are split on blanks", s.run("echo a    b   c"), "a b c\n")
    check("double quotes keep blanks", s.run('echo "a   b"'), "a   b\n")
    check("single quotes are literal", s.run("echo 'a $X \\n b'"), "a $X \\n b\n")
    check("a quoted operator is text", s.run("echo '|' '>' \"<\""), "| > <\n")
    check("a backslash escapes the next character", s.run("echo a\\ b"), "a b\n")
    check("adjacent quotes make one word", s.run("echo a'b'\"c\"d"), "abcd\n")
    check("an empty quoted word is an argument", s.run("echo '' x"), " x\n")
    check("a comment is ignored", s.run("echo a # not printed"), "a\n")
    check("a # inside a word is text", s.run("echo a#b"), "a#b\n")
    check("a blank line does nothing", s.run(""), "")

    # ---------------------------------------------------------------- variables
    check("an assignment is silent", s.run("X=hello"), "")
    check("$X expands", s.run("echo $X"), "hello\n")
    check("${X} expands, and delimits the name", s.run("echo ${X}y"), "helloy\n")
    check("$X inside double quotes expands", s.run('echo "<$X>"'), "<hello>\n")
    check("$X inside single quotes does not", s.run("echo '<$X>'"), "<$X>\n")
    check("an unset variable vanishes", s.run("echo a $NOSUCH b"), "a b\n")
    check("a quoted unset variable is one empty argument", s.run('echo a "$NOSUCH" b'), "a  b\n")
    s.run("W='/nosuch1 /nosuch2'")
    check("an unquoted multi-word value splits into fields",
          s.run("cat $W"), "cat: /nosuch1: No such file or directory\ncat: /nosuch2: No such file or directory\n")
    check("a quoted one stays a single field",
          s.run('cat "$W"'), "cat: /nosuch1 /nosuch2: No such file or directory\n")
    check("a $ not followed by a name is literal", s.run("echo $ $-"), "$ $-\n")
    check("a variable can be reassigned", s.run("X=two") + s.run("echo $X"), "two\n")
    check("assignments see earlier ones", s.run("A=1 B=$A") + s.run("echo $A $B"), "1 1\n")

    # ---------------------------------------------------------------- export, unset, env
    check("an unexported variable is not in a program's environment", s.run("printenv X"), "")
    check("...and printenv says so with its status", s.status("printenv X")[1], 1)
    check("export marks it", s.run("export X") + s.run("printenv X"), "two\n")
    check("export NAME=VALUE assigns and exports", s.run("export Y=why") + s.run("printenv Y"), "why\n")
    check("a one-shot assignment is in that command's environment",
          s.run("ONLY=here printenv ONLY"), "here\n")
    check("...and gone afterwards", s.run("printenv ONLY"), "")
    check("unset removes", s.run("unset Y") + s.run("printenv Y"), "")
    check("unsetting an unset name is not an error", s.status("unset NOSUCH")[1], 0)
    check("export refuses an invalid name",
          s.run("export 1X=2").strip().startswith("export:"), True)
    check("env lists the exported variables",
          "X=two" in s.run("env").split("\n") and "HOME=/root" in s.run("env").split("\n"), True)

    # ---------------------------------------------------------------- exit status
    check("$? after success", s.status("echo hi")[1], 0)
    check("$? after a program's failure", s.status("cat /nosuch")[1], 1)
    check("$? after an unknown command", s.status("nosuchcmd")[1], 127)
    check("an unknown command is reported", s.run("nosuchcmd"), "nosuchcmd: command not found\n")
    check("$? after a syntax error", s.status("echo 'unterminated")[1], 2)
    s.run("cat /nosuch")
    s.run("")
    check("a blank line leaves $? alone", s.run("echo $?"), "1\n")
    s.run("cat /nosuch")
    check("the status is expandable", s.run("echo status=$?"), "status=1\n")

    # ---------------------------------------------------------------- redirection
    check("> creates", s.run("echo one > /tmp/r"), "")
    check("> truncates", s.run("echo two > /tmp/r") + s.run("cat /tmp/r"), "two\n")
    check(">> appends", s.run("echo three >> /tmp/r") + s.run("cat /tmp/r"), "two\nthree\n")
    check(">> creates a missing file", s.run("echo new >> /tmp/r2") + s.run("cat /tmp/r2"), "new\n")
    check("< feeds stdin", s.run("wc -l < /tmp/r"), "2\n")
    check("2> captures stderr", s.run("cat /nosuch 2> /tmp/e") + s.run("cat /tmp/e"),
          "cat: /nosuch: No such file or directory\n")
    check("stderr is not in stdout without 2>&1",
          s.run("cat /nosuch > /tmp/o"), "cat: /nosuch: No such file or directory\n")
    check("...so the file is empty", s.run("wc -c < /tmp/o"), "0\n")
    check("2>&1 before > duplicates the old stdout: the message stays on the console",
          s.run("cat /nosuch 2>&1 > /tmp/o2"), "cat: /nosuch: No such file or directory\n")
    check("...and the file is empty", s.run("wc -c < /tmp/o2"), "0\n")
    check("> f 2>&1 sends both to f",
          s.run("cat /nosuch > /tmp/o3 2>&1") + s.run("cat /tmp/o3"), "cat: /nosuch: No such file or directory\n")
    check("redirections can come before the command", s.run("> /tmp/r4 echo made") + s.run("cat /tmp/r4"), "made\n")
    check("a redirection alone creates the file", s.run("> /tmp/r5") + s.run("wc -c < /tmp/r5"), "0\n")
    check("an unreadable < target is reported", s.run("cat < /tmp/nosuch"), "/tmp/nosuch: No such file or directory\n")
    check("...with status 1", s.status("cat < /tmp/nosuch")[1], 1)
    check("an uncreatable > target is reported", s.run("echo x > /tmp/nodir/f"), "/tmp/nodir/f: No such file or directory\n")
    check("...for a builtin too", s.run("cd > /tmp/nodir/f"), "/tmp/nodir/f: No such file or directory\n")
    check("a failed redirect leaves the earlier ones in effect",
          s.run("echo x > /tmp/ok1 < /tmp/nosuch") + s.run("wc -c < /tmp/ok1"), "/tmp/nosuch: No such file or directory\n0\n")
    check("a command whose redirect failed does not run", s.run("echo ran > /tmp/ok2 < /tmp/nosuch") and s.run("cat /tmp/ok2"), "")

    # ---------------------------------------------------------------- pipelines
    check("a two-stage pipeline", s.run("echo hello | cat"), "hello\n")
    check("a three-stage pipeline", s.run("seq 5 | tail -n 3 | head -n 1"), "3\n")
    check("the pipeline's status is the last stage's", s.status("cat /nosuch | wc -l")[1], 0)
    check("stderr is not piped", s.run("cat /nosuch | wc -l"), "cat: /nosuch: No such file or directory\n0\n")
    check("2>&1 pipes stderr too", s.run("cat /nosuch 2>&1 | wc -l"), "1\n")
    check("a stage's own redirect beats the pipe", s.run("echo x > /tmp/p1 | wc -c"), "0\n")
    check("an unknown first stage still lets the next run", s.run("nosuchcmd | wc -c"),
          "nosuchcmd: command not found\n0\n")
    check("pipelines leave no temporary files behind", s.run("ls -1 -a /tmp").count(".pipe"), 0)

    # ---------------------------------------------------------------- syntax errors
    check("a redirection with no target", s.run("echo >"), "no file name after '>'\n")
    check("...is status 2", s.status("echo >")[1], 2)
    check("`;` is not supported, and says so", s.run("echo a ; echo b"), "';' is not supported yet\n")
    check("a trailing pipe is refused", s.run("echo a |"), "no command next to '|'\n")
    check("a leading pipe is refused", s.run("| echo a"), "no command next to '|'\n")
    check("...with status 2, not silently ignored", s.status("echo a |")[1], 2)
    check("an unterminated quote", s.run("echo 'x"), "unterminated quote\n")
    check(">& needs 1 or 2", s.run("echo a >&3"), "a '>&' target must be 1 or 2\n")
