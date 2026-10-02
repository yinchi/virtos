"""The text filters -- seq, sort, uniq, tr, cut, fgrep -- on stdin (pipes) and on files, their
flags, and what each does with bad input."""

SYSTEM_FILES = {
    "data/fruit.txt": "pear\napple\nfig\napple\nbanana\nfig\nfig\n",
    "data/nums.txt": "10\n9\n100\n-5\n2.5\nabc\n",
    "data/csv.txt": "a,b,c\n1,2,3\nlonely\nx,,z\n",
    "data/utf8.txt": "héllo\n日本語\n",
    "data/text.txt": "Hello World\nhello again\nno match here\nHELLO\n",
    "data/dots.txt": "a.b\naxb\n",
}


def run(ctx):
    s, check = ctx.s, ctx.check

    # ---------------------------------------------------------------- wc on UTF-8 (the counts differ)
    check("wc -c counts bytes", s.run("wc -c /data/utf8.txt"), "17 /data/utf8.txt\n")
    check("wc -m counts characters", s.run("wc -m /data/utf8.txt"), "10 /data/utf8.txt\n")
    check("wc -L measures the longest line in characters", s.run("wc -L /data/utf8.txt"), "5 /data/utf8.txt\n")

    # ---------------------------------------------------------------- seq
    check("seq LAST", s.run("seq 3"), "1\n2\n3\n")
    check("seq FIRST LAST", s.run("seq 2 4"), "2\n3\n4\n")
    check("seq FIRST INCR LAST", s.run("seq 1 3 10"), "1\n4\n7\n10\n")
    check("seq counts down with a negative step", s.run("seq 3 -1 1"), "3\n2\n1\n")
    check("seq takes negative numbers", s.run("seq -2 0"), "-2\n-1\n0\n")
    check("seq with nothing to print", s.run("seq 5 1"), "")
    check("seq -s joins with a separator", s.run("seq -s , 3"), "1,2,3\n")
    check("seq -w pads with zeros", s.run("seq -w 8 11"), "08\n09\n10\n11\n")
    check("seq -w with a negative", s.run("seq -w -1 1"), "-1\n00\n01\n")
    check("seq refuses a non-integer", s.run("seq 1.5"), "seq: invalid integer argument: '1.5'\nTry 'seq --help' for more information.\n")
    check("seq refuses a zero step", s.run("seq 1 0 5"), "seq: invalid Zero increment value: '0'\nTry 'seq --help' for more information.\n")
    check("seq with no operand", s.run("seq"), "seq: missing operand\nTry 'seq --help' for more information.\n")
    check("seq with too many operands", s.run("seq 1 2 3 4"), "seq: extra operand '4'\nTry 'seq --help' for more information.\n")
    check("seq stops at the end of the integers",
          s.run("seq 9223372036854775806 9223372036854775807"), "9223372036854775806\n9223372036854775807\n")

    # ---------------------------------------------------------------- sort
    check("sort is bytewise", s.run("sort /data/fruit.txt"), "apple\napple\nbanana\nfig\nfig\nfig\npear\n")
    check("sort -r reverses", s.run("sort -r /data/fruit.txt"), "pear\nfig\nfig\nfig\nbanana\napple\napple\n")
    check("sort -u drops duplicates", s.run("sort -u /data/fruit.txt"), "apple\nbanana\nfig\npear\n")
    check("sort -n sorts by number", s.run("sort -n /data/nums.txt"), "-5\nabc\n2.5\n9\n10\n100\n")
    check("sort -rn", s.run("sort -rn /data/nums.txt"), "100\n10\n9\n2.5\nabc\n-5\n")
    check("without -n, 10 sorts before 9", s.run("sort /data/nums.txt").split("\n")[:3], ["-5", "10", "100"])
    check("capital letters sort before lower case (the C locale)", s.run("sort /data/text.txt"),
          "HELLO\nHello World\nhello again\nno match here\n")
    check("sort merges several files", s.run("sort /data/fruit.txt /data/csv.txt").split("\n")[:3], ["1,2,3", "a,b,c", "apple"])
    check("sort reads stdin", s.run("seq 3 | sort -r"), "3\n2\n1\n")
    check("sort of a missing file", s.run("sort /nosuch"), "sort: cannot read: /nosuch\n")
    check("...has status 2", s.status("sort /nosuch")[1], 2)
    check("a last line with no newline is a line", s.run("echo -n b | sort -r"), "b\n")
    check("an unknown flag", s.run("sort -z"), "sort: invalid option -- 'z'\nTry 'sort --help' for more information.\n")

    # ---------------------------------------------------------------- uniq
    check("uniq collapses adjacent duplicates", s.run("uniq /data/fruit.txt"), "pear\napple\nfig\napple\nbanana\nfig\n")
    check("uniq -c counts", s.run("sort /data/fruit.txt | uniq -c"),
          "      2 apple\n      1 banana\n      3 fig\n      1 pear\n")
    check("uniq -d prints only repeats", s.run("sort /data/fruit.txt | uniq -d"), "apple\nfig\n")
    check("uniq -u prints only singletons", s.run("sort /data/fruit.txt | uniq -u"), "banana\npear\n")
    check("uniq of stdin", s.run("seq 2 | uniq"), "1\n2\n")
    check("uniq with two files is refused", s.run("uniq a b"), "uniq: extra operand 'b'\nTry 'uniq --help' for more information.\n")
    check("the classic frequency pipeline", s.run("sort /data/fruit.txt | uniq -c | sort -rn | head -n 1"), "      3 fig\n")

    # ---------------------------------------------------------------- tr
    check("tr translates a range", s.run("echo hello | tr a-z A-Z"), "HELLO\n")
    check("tr translates positionally", s.run("echo abc | tr abc xyz"), "xyz\n")
    check("a short SET2 is padded with its last character", s.run("echo abcd | tr abcd xy"), "xyyy\n")
    check("tr -d deletes", s.run("echo hello | tr -d l"), "heo\n")
    check("tr -s squeezes", s.run("echo aabbcc | tr -s a-c"), "abc\n")
    check("tr -ds deletes the first set, then squeezes the second", s.run("echo aabbxx | tr -ds a x"), "bbx\n")
    check("tr classes", s.run("echo 'a1 b2' | tr -d '[:digit:]'"), "a b\n")
    check("tr escapes", s.run("echo 'a b' | tr ' ' '\\n'"), "a\nb\n")
    check("tr leaves non-ASCII alone", s.run("tr a-z A-Z < /data/utf8.txt"), "HéLLO\n日本語\n")
    check("tr with one set and no -d is refused", s.run("tr a"), "tr: missing operand after 'a'\nTry 'tr --help' for more information.\n")
    check("tr with a reversed range", s.run("tr z-a x"), "tr: range endpoints in 'z-a' are in reverse order\n")
    check("tr with a bad class", s.run("tr '[:nosuch:]' x"), "tr: invalid character class in '[:nosuch:]'\n")
    check("tr with no operand", s.run("tr"), "tr: missing operand\nTry 'tr --help' for more information.\n")

    # ---------------------------------------------------------------- cut
    check("cut -d -f picks fields", s.run("cut -d , -f 2 /data/csv.txt"), "b\n2\nlonely\n\n")
    check("cut -s skips lines with no delimiter", s.run("cut -s -d , -f 1 /data/csv.txt"), "a\n1\nx\n")
    check("cut -f lists and ranges", s.run("cut -d , -f 1,3 /data/csv.txt").split("\n")[:2], ["a,c", "1,3"])
    check("cut -f N-", s.run("cut -d , -f 2- /data/csv.txt").split("\n")[:2], ["b,c", "2,3"])
    check("cut -c picks characters", s.run("echo abcdef | cut -c 2-4"), "bcd\n")
    check("cut -c counts UTF-8 characters", s.run("cut -c 1-3 /data/utf8.txt"), "hél\n日本語\n")
    check("cut with the default tab delimiter", s.run("echo -e 'a\\tb\\tc' | cut -f 2"), "b\n")
    check("cut with no list is refused", s.run("cut /data/csv.txt"), "cut: you must specify a list of characters or fields\nTry 'cut --help' for more information.\n")
    check("cut with both lists is refused", s.run("cut -c 1 -f 1 x"), "cut: only one type of list may be specified\nTry 'cut --help' for more information.\n")
    check("cut with a zero position", s.run("cut -f 0 x"), "cut: fields are numbered from 1\nTry 'cut --help' for more information.\n")
    check("cut with a decreasing range", s.run("cut -f 3-1 x"), "cut: invalid decreasing range\nTry 'cut --help' for more information.\n")
    check("cut -s with -c is refused", s.run("cut -s -c 1 x").startswith("cut: suppressing non-delimited lines"), True)
    check("a multi-character delimiter is refused", s.run("cut -d ab -f 1 x"), "cut: the delimiter must be a single character\nTry 'cut --help' for more information.\n")

    # ---------------------------------------------------------------- fgrep
    check("fgrep selects matching lines", s.run("fgrep hello /data/text.txt"), "hello again\n")
    check("fgrep -i ignores case", s.run("fgrep -i hello /data/text.txt"), "Hello World\nhello again\nHELLO\n")
    check("fgrep -v inverts", s.run("fgrep -v -i hello /data/text.txt"), "no match here\n")
    check("fgrep -n numbers", s.run("fgrep -n match /data/text.txt"), "3:no match here\n")
    check("fgrep -c counts", s.run("fgrep -c -i hello /data/text.txt"), "3\n")
    check("fgrep -l names files", s.run("fgrep -l apple /data/fruit.txt /data/csv.txt"), "/data/fruit.txt\n")
    check("fgrep -q is silent", s.run("fgrep -q fig /data/fruit.txt"), "")
    check("fgrep treats . as a plain character", s.run("fgrep a.b /data/dots.txt"), "a.b\n")
    check("fgrep -e takes several patterns", s.run("fgrep -e pear -e banana /data/fruit.txt"), "pear\nbanana\n")
    check("several files get name prefixes", s.run("fgrep fig /data/fruit.txt /data/csv.txt"),
          "/data/fruit.txt:fig\n/data/fruit.txt:fig\n/data/fruit.txt:fig\n")
    check("fgrep reads stdin", s.run("seq 12 | fgrep 1"), "1\n10\n11\n12\n")
    check("fgrep status: 0 on a match", s.status("fgrep fig /data/fruit.txt")[1], 0)
    check("fgrep status: 1 on none", s.status("fgrep zzz /data/fruit.txt")[1], 1)
    check("fgrep status: 2 on an unreadable file", s.status("fgrep x /nosuch")[1], 2)
    check("fgrep of a missing file", s.run("fgrep x /nosuch"), "fgrep: /nosuch: cannot open\n")
    check("fgrep with no pattern", s.run("fgrep"), "fgrep: missing operand\nTry 'fgrep --help' for more information.\n")
