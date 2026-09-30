//! Command-line flag parsing shared by more than one of this crate's programs, on `getargs` --
//! the same crate the reference lab's own `progs_r12::cli` uses for this exact job. A completely
//! different, unrelated problem from shell tokenization (`shlex`/the real shell's own PEG grammar,
//! `plans/USERSPACE_PLAN.md`'s step 10): by the time a program sees its own `argv`, the shell has
//! already split the line into words, so there's no quoting/escaping left for this layer to worry
//! about -- just recognizing which of those already-separate words are flags.

use core::iter::Skip;

use alloc::format;
use alloc::vec::Vec;
use getargs::{Opt, Options};
use userlib::{Args, ExitCode};

/// A parser over a program's arguments after `argv[0]`.
type Opts = Options<&'static str, Skip<Args>>;

/// Prints `<prog>: <msg>` to stderr, GNU's usual shape for an argument error.
fn report(prog: &str, msg: &str) {
    userlib::write(2, prog.as_bytes());
    userlib::write(2, b": ");
    userlib::write(2, msg.as_bytes());
    userlib::write(2, b"\n");
}

/// Parses `head`/`tail`'s shared shape: an optional `-n`/`--lines COUNT` (attached or separate,
/// `-n5`/`-n 5`/`--lines=5`/`--lines 5` all work -- `getargs` handles that uniformly), defaulting
/// to `default` if absent, followed by the remaining positional arguments (the file list). `Err` is
/// the exit status to return, already reported (or, for `--help`, already printed to stdout via
/// `progs::help(usage, flags)`, exit `0`) -- the caller only needs to propagate it, the same shape
/// the reference lab's own `cli::status` gives its callers.
pub fn line_count(
    prog: &str,
    usage: &str,
    flags: &[(&str, &str)],
    args: Args,
    default: usize,
) -> Result<(usize, Vec<&'static str>), ExitCode> {
    let mut opts: Opts = Options::new(args.skip(1));
    let mut count = default;
    loop {
        match opts.next_opt() {
            Ok(Some(Opt::Long("help"))) => return Err(crate::help(usage, flags)),
            Ok(Some(Opt::Short('n') | Opt::Long("lines"))) => {
                let Ok(value) = opts.value() else {
                    report(prog, "option requires an argument -- 'n'");
                    return Err(ExitCode(1));
                };
                let Ok(n) = value.parse() else {
                    report(prog, &format!("invalid number of lines: '{value}'"));
                    return Err(ExitCode(1));
                };
                count = n;
            }
            Ok(Some(opt)) => {
                report(prog, &format!("invalid option -- '{opt}'"));
                return Err(ExitCode(1));
            }
            Ok(None) => break,
            Err(e) => {
                report(prog, &format!("{e}"));
                return Err(ExitCode(1));
            }
        }
    }
    Ok((count, opts.positionals().collect()))
}
