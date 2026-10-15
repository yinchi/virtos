"""Command lists: `;`, `&&`, `||` and `!`. A list runs its `;`-separated parts one after another
whatever each one's status; `&&`/`||` bind tighter than `;` (and looser than `|`), are left
associative, and look at the status of everything to their left; a skipped command leaves that
status as it was; `!` negates a pipeline's status. Quoted or escaped operators are ordinary text."""


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("cd /tmp")

    # --------------------------------------------------------------------- `;`
    check("`;` runs one command after another", s.run("echo a ; echo b"), "a\nb\n")
    check("...with no blanks around it", s.run("echo a;echo b"), "a\nb\n")
    check("...and a trailing one is fine", s.run("echo a;"), "a\n")
    check("the next command runs whatever the last one's status", s.run("false ; echo $?"), "1\n")
    check("a failing command does not stop the list", s.run("nosuchcmd ; echo after").endswith("after\n"), True)
    check("an assignment is visible to the next part", s.run("X=1 ; echo $X"), "1\n")
    check("a `cd` in one part applies to the next", s.run("cd /tmp ; pwd"), "/tmp\n")

    # --------------------------------------------------------------- `&&` and `||`
    check("&& runs the next command after success", s.run("true && echo yes"), "yes\n")
    check("&& skips it after failure", s.run("false && echo no"), "")
    check("|| runs the next command after failure", s.run("false || echo fallback"), "fallback\n")
    check("|| skips it after success", s.run("true || echo no"), "")
    check("a chain is left to right: false && x || y runs y", s.run("false && echo x || echo y"), "y\n")
    check("...and true || x && y runs y too", s.run("true || echo x && echo y"), "y\n")
    check("...and true && false || z runs z", s.run("true && false || echo z"), "z\n")
    check("$? in a later part is the earlier part's status", s.run("false || echo $?"), "1\n")
    check("...and 0 after a success", s.run("true && echo $?"), "0\n")
    check("a skipped command leaves the status as it was", s.run("false && true ; echo $?"), "1\n")
    check("...also for ||", s.run("true || false ; echo $?"), "0\n")
    check("the status of a list is its last command's", s.status("true ; false")[1], 1)
    check("a pipe binds tighter than &&", s.run("echo hi | cat && echo done"), "hi\ndone\n")
    check("redirections belong to their own command", s.run("echo a > f1 && cat f1"), "a\n")
    s.run("rm -f never")
    s.run("false && echo x > never")
    check("a skipped command's redirection is not applied", s.run("ls -1 never").endswith("No such file or directory\n"), True)
    check("a failed command's status is the program's own", s.status("false || false")[1], 1)
    check("... and 127 for one that is not found", s.status("true && nosuchcmd")[1], 127)

    # ------------------------------------------------------------------------ `!`
    check("! negates failure to success", s.run("! false && echo negated"), "negated\n")
    check("! negates success to failure", s.run("! true || echo ok"), "ok\n")
    check("! covers the whole pipeline", s.run("! echo a | grep_not_a_command ; echo $?").endswith("0\n"), True)
    check("! negates twice to nothing", s.status("! ! false")[1], 1)
    check("its status is 0 or 1 whatever the command's", s.status("! nosuchcmd")[1], 0)
    check("a ! that is not first is an argument", s.run("echo !"), "!\n")
    check("...and so is one inside a word", s.run("echo a! !b"), "a! !b\n")

    # ------------------------------------------------------------ quoting and scripts
    check("a quoted operator is text", s.run("echo ';' '&&' \"||\""), "; && ||\n")
    check("an escaped one too", s.run("echo a\;b"), "a;b\n")
    s.run("echo 'false || echo from-script ; echo second' > s1.sh")
    check("a script's line may be a list", s.run("sh s1.sh"), "from-script\nsecond\n")

    # -------------------------------------------------------------------- errors
    check("a trailing && asks for a command", s.run("echo a &&"), "no command after '&&'\n")
    check("...and a trailing ||", s.run("echo a ||"), "no command after '||'\n")
    check("a leading && has nothing to its left", s.run("&& echo a"), "no command next to an operator\n")
    check("a doubled ; is refused", s.run("echo a ; ; echo b"), "no command next to an operator\n")
    check("a leading ; is refused", s.run("; echo a"), "no command next to an operator\n")
    check(";; is not supported, and says so", s.run("echo a ;; echo b"), "';;' is not supported yet\n")
    check("a lone & is still not supported", s.run("echo a & echo b"), "'&' is not supported yet\n")
    check("a syntax error is status 2", s.status("echo a &&")[1], 2)
    check("...and runs nothing", s.run("echo before && ; echo after"), "no command next to an operator\n")
