"""Time: date (flags, $TZ), stat (times shown in $TZ), touch (-d, -t, -r, -c), and the clock that
stamps files. These programs carry the whole time-zone database and take a few seconds to load."""

import harness

SLOW = harness.SLOW

# A fixed instant: 2001-09-09 01:46:40 UTC, which is 2001-09-08 21:46:40 EDT in Toronto.
EPOCH = 1000000000

SYSTEM_FILES = {"data/f": "x"}


def run(ctx):
    s, check = ctx.s, ctx.check

    def run(cmd):
        return s.run(cmd, timeout=SLOW)

    # ---------------------------------------------------------------- date
    check("date -u -d @N", run(f"date -u -d @{EPOCH}"), "Sun Sep  9 01:46:40 UTC 2001\n")
    check("with no TZ the local zone is UTC", run(f"date -d @{EPOCH}"), "Sun Sep  9 01:46:40 UTC 2001\n")
    check("$TZ names the local zone, with its abbreviation", run(f"TZ=America/Toronto date -d @{EPOCH}"),
          "Sat Sep  8 21:46:40 EDT 2001\n")
    check("a zone's standard-time abbreviation", run("TZ=America/Toronto date -d @1010000000"),
          "Wed Jan  2 14:33:20 EST 2002\n")
    check("an unknown $TZ means UTC", run(f"TZ=Nowhere/Land date -d @{EPOCH}"), "Sun Sep  9 01:46:40 UTC 2001\n")
    check("a leading colon in $TZ is ignored", run(f"TZ=:Asia/Tokyo date -d @{EPOCH}"), "Sun Sep  9 10:46:40 JST 2001\n")
    check("-u beats $TZ", run(f"TZ=Asia/Tokyo date -u -d @{EPOCH}"), "Sun Sep  9 01:46:40 UTC 2001\n")
    check("date +FORMAT", run(f"date -u -d @{EPOCH} +%Y-%m-%d"), "2001-09-09\n")
    check("date +FORMAT with several conversions", run(f"date -u -d @{EPOCH} '+%H:%M:%S %j %a %b %%'"), "01:46:40 252 Sun Sep %\n")
    check("date +%s", run(f"date -d @{EPOCH} +%s"), f"{EPOCH}\n")
    check("date -I", run(f"date -u -d @{EPOCH} -I"), "2001-09-09\n")
    check("date -I=seconds has the real offset", run(f"TZ=America/Toronto date -d @{EPOCH} -Iseconds"), "2001-09-08T21:46:40-04:00\n")
    check("date -I=hours", run(f"date -u -d @{EPOCH} -Ihours"), "2001-09-09T01+00:00\n")
    check("date -R", run(f"date -u -d @{EPOCH} -R"), "Sun, 09 Sep 2001 01:46:40 +0000\n")
    check("the epoch", run("date -u -d @0"), "Thu Jan  1 00:00:00 UTC 1970\n")
    check("a negative time", run("date -u -d @-1"), "Wed Dec 31 23:59:59 UTC 1969\n")
    check("date -d needs @N", run("date -d yesterday"), "date: invalid date 'yesterday'\n")
    check("two formats are refused", run("date -R -I"), "date: multiple output formats specified\n")
    check("a bad -I precision", run("date -Iweeks"), "date: invalid argument 'weeks' for '--iso-8601'\nTry 'date --help' for more information.\n")
    check("a stray operand", run("date tomorrow"), "date: invalid date 'tomorrow'\n")
    check("the default format has the current zone and year",
          run("date -u").split()[-2:] == ["UTC", run("date -u +%Y").strip()], True)

    # ---------------------------------------------------------------- touch and stat
    def stat_line(path, label, tz=""):
        out = run(f"{tz}stat {path}").split("\n")
        return [l for l in out if l.startswith(label)][0]

    check("touch creates a file", run("touch /tmp/t1") + run("ls /tmp"), "t1\n")
    check("...empty", run("stat /tmp/t1").split("\n")[1], "  Size: 0            Type: regular file")
    year = run("date -u +%Y").strip()
    check("a new file is stamped with the clock (now, not 1980)", stat_line("/tmp/t1", "Create").startswith(f"Create: {year}"), True)
    run("touch -d 2020-05-01T12:00:00Z /tmp/t1")
    check("touch -d with a Z sets UTC", stat_line("/tmp/t1", "Modify"), "Modify: 2020-05-01 12:00:00 UTC")
    check("stat shows times in $TZ", stat_line("/tmp/t1", "Modify", "TZ=America/Toronto "), "Modify: 2020-05-01 08:00:00 EDT")
    run("TZ=America/Toronto touch -d '2024-05-01 12:00' /tmp/t1")
    check("touch -d without a Z is local time ($TZ)", stat_line("/tmp/t1", "Modify"), "Modify: 2024-05-01 16:00:00 UTC")
    run("touch -t 202401021530.10 /tmp/t1")
    check("touch -t", stat_line("/tmp/t1", "Modify"), "Modify: 2024-01-02 15:30:10 UTC")
    run("touch -d @1000000000 /tmp/t1")
    check("touch -d @N", stat_line("/tmp/t1", "Modify"), "Modify: 2001-09-09 01:46:40 UTC")
    run("touch -r /tmp/t1 /tmp/t2")
    check("touch -r copies a time (and creates the file)", stat_line("/tmp/t2", "Modify"), "Modify: 2001-09-09 01:46:40 UTC")
    check("touch -c does not create", run("touch -c /tmp/t3") + run("ls /tmp"), "t1\nt2\n")
    run("touch -c /tmp/t1")
    check("touch -c on an existing file touches it", stat_line("/tmp/t1", "Modify").startswith(f"Modify: {year}"), True)
    check("an odd second is dropped (FAT keeps two-second steps)", run("touch -d @1000000001 /tmp/t1") + stat_line("/tmp/t1", "Modify"), "Modify: 2001-09-09 01:46:40 UTC")
    check("a time before 1980 is refused", run("touch -d 1970-01-01Z /tmp/t1"), "touch: setting times of '/tmp/t1': Invalid argument\n")
    check("a time after 2107 is refused", run("touch -d 2108-01-01Z /tmp/t1"), "touch: setting times of '/tmp/t1': Invalid argument\n")
    check("a bad date", run("touch -d nonsense /tmp/t1"), "touch: invalid date format 'nonsense'\n")
    check("a bad stamp", run("touch -t 13 /tmp/t1"), "touch: invalid date format '13'\n")
    check("-d and -r together", run("touch -d @0 -r /tmp/t1 /tmp/t1"), "touch: cannot specify times from more than one source\nTry 'touch --help' for more information.\n")
    check("touch -r of a missing file", run("touch -r /nosuch /tmp/t1"), "touch: failed to get attributes of '/nosuch'\n")
    check("touch needs a file", run("touch"), "touch: missing file operand\nTry 'touch --help' for more information.\n")
    check("a directory can be touched", run("mkdir /tmp/d") + run("touch -d @1000000000 /tmp/d") + stat_line("/tmp/d", "Modify"), "Modify: 2001-09-09 01:46:40 UTC")
    check("mv keeps the modify time", run("mv /tmp/t1 /tmp/t1b") + stat_line("/tmp/t1b", "Modify"), "Modify: 2001-09-09 01:46:40 UTC")
    run("echo hi > /tmp/w")
    check("writing a file stamps it with the clock", stat_line("/tmp/w", "Modify").startswith(f"Modify: {year}"), True)
    check("stat of a missing file", run("stat /nosuch"), "stat: cannot stat '/nosuch': No such file or directory\n")
