//! Command-line flag parsing shared by more than one of this crate's programs, on `getargs` --
//! the same crate the reference lab's own `progs_r12::cli` uses for this exact job, and the same
//! shapes it builds there: `opts`/`next`/`invalid`/`operands` for the general case (flags
//! checked in one pass, since `getargs`'s `next_arg` -- unlike `next_opt` -- keeps recognizing
//! options *after* a positional has already gone by, matching GNU's own `rm a -r`, which
//! recurses; then operands walked again in a second pass, which is why this only fits a program
//! none of whose own options takes a value), plus `plain` for the shape that repeats most: a
//! program with no option besides `--help`, summarized by how many operands it got and its
//! first/second/last (`cat`/`rm`/`stat`/`hexdump`/`wc`'s exact shape; `cp`/`mv` too, just reading
//! `count`/`first`/`last` themselves rather than the operand text). `chmod` still can't use any
//! of this for its own mode operand, which looks like an option (`-x`) but isn't one -- same
//! reason the lab's own `cli.rs` gives for writing it by hand there too.
//!
//! A completely different, unrelated problem from shell tokenization (`shlex`/the real shell's
//! own PEG grammar, `plans/USERSPACE_PLAN.md`'s step 10): by the time a program sees its own
//! `argv`, the shell has already split the line into words, so there's no quoting/escaping left
//! for this layer to worry about -- just recognizing which of those already-separate words are
//! flags, and reporting the one that was supposed to be a flag but wasn't.

use core::iter::Skip;

use alloc::format;
use alloc::vec::Vec;
use getargs::{Arg, Error, Opt, Options};
use userlib::{Args, ExitCode};

/// A parser over a program's arguments after `argv[0]`.
pub type Opts = Options<&'static str, Skip<Args>>;

/// A fresh parser over `args` (which still holds `argv[0]`).
pub fn opts(args: Args) -> Opts {
    Options::new(args.skip(1))
}

/// Prints `<prog>: <msg>` to stderr, GNU's usual shape for an argument error.
fn report(prog: &str, msg: &str) {
    userlib::write(2, prog.as_bytes());
    userlib::write(2, b": ");
    userlib::write(2, msg.as_bytes());
    userlib::write(2, b"\n");
}

/// Prints `Try '<prog> --help' for more information.` to stderr -- GNU's own trailer on an
/// argument error, used only by the diagnostics below (new output this project is only just
/// growing; existing messages elsewhere keep their own wording unchanged).
fn try_help(prog: &str) {
    userlib::write(2, b"Try '");
    userlib::write(2, prog.as_bytes());
    userlib::write(2, b" --help' for more information.\n");
}

/// The next option or operand, interleaved correctly even across an already-seen positional
/// (`next_arg`, not `next_opt` -- see this module's own doc comment for why that distinction is
/// the whole reason to use it). A parse error (a value given to an option that doesn't take one,
/// or missing from one that does) has already been reported, and comes back as the exit status
/// to return.
pub fn next(prog: &str, opts: &mut Opts) -> Result<Option<Arg<&'static str>>, ExitCode> {
    opts.next_arg().map_err(|e| option_error(prog, e))
}

/// The value of the option just returned by `next`: `-n 5`, `-n5`, `--lines 5` and `--lines=5`
/// all give `5`.
pub fn value(prog: &str, opts: &mut Opts) -> Result<&'static str, ExitCode> {
    opts.value().map_err(|e| option_error(prog, e))
}

/// Reports `arg` as not one of the program's options -- `invalid option -- 'x'` for a short one,
/// `unrecognized option '--name'` for a long one (GNU's own two different wordings for the two
/// cases). A positional reaching here means the caller already knows it has no business being
/// one (an operand count already checked, say), so it's reported as `unrecognized argument`.
pub fn invalid(prog: &str, arg: Arg<&str>) -> ExitCode {
    match arg {
        Arg::Short(c) => report(prog, &format!("invalid option -- '{c}'")),
        Arg::Long(name) => report(prog, &format!("unrecognized option '--{name}'")),
        Arg::Positional(operand) => {
            report(prog, &format!("unrecognized argument '{operand}'"));
        }
    }
    try_help(prog);
    ExitCode(1)
}

/// GNU's wording for the two ways an option's value can be wrong.
fn option_error(prog: &str, error: Error<&str>) -> ExitCode {
    match error {
        Error::RequiresValue(Opt::Short(c)) => {
            report(prog, &format!("option requires an argument -- '{c}'"));
        }
        Error::RequiresValue(Opt::Long(name)) => {
            report(prog, &format!("option '--{name}' requires an argument"));
        }
        Error::DoesNotRequireValue(Opt::Long(name)) => {
            report(
                prog,
                &format!("option '--{name}' doesn't allow an argument"),
            );
        }
        Error::DoesNotRequireValue(Opt::Short(c)) => {
            report(prog, &format!("option '-{c}' doesn't allow an argument"));
        }
        // `getargs::Error` is `#[non_exhaustive]`; both variants it actually has are matched
        // above.
        _ => report(prog, "invalid option"),
    }
    try_help(prog);
    ExitCode(1)
}

/// The operands of a program none of whose options takes a value, in order; options are skipped
/// (a first pass, via `next`/`plain`, already checked them).
pub struct Operands(Opts);

/// An iterator over the operands in `args` (which still holds `argv[0]`).
pub fn operands(args: Args) -> Operands {
    Operands(opts(args))
}

impl Iterator for Operands {
    type Item = &'static str;

    fn next(&mut self) -> Option<&'static str> {
        loop {
            match self.0.next_arg() {
                Ok(Some(Arg::Positional(operand))) => return Some(operand),
                Ok(Some(_)) | Err(_) => continue,
                Ok(None) => return None,
            }
        }
    }
}

/// The operands of a program whose only option is `--help`: how many, and the first, second and
/// last (several programs need the first/last for a usage message -- `cp`/`mv`'s own "missing
/// destination operand after 'first'" -- without re-walking the whole list; `second` is unused
/// so far but costs nothing to keep, matching the lab's own `Plain`).
pub struct Plain {
    pub count: usize,
    pub first: Option<&'static str>,
    pub second: Option<&'static str>,
    pub last: Option<&'static str>,
}

/// Validates the arguments of a program with no options besides `--help` and summarizes its
/// operands. `Err` is the status to return: `--help` (printed with `usage` and `flags`) or a
/// problem already reported (an unrecognized option). The operands themselves, if needed in
/// full (more than `first`/`second`/`last`), are then walked again with `operands`.
pub fn plain(
    prog: &str,
    usage: &str,
    flags: &[(&str, &str)],
    args: Args,
) -> Result<Plain, ExitCode> {
    let mut opts = opts(args);
    let mut summary = Plain {
        count: 0,
        first: None,
        second: None,
        last: None,
    };
    while let Some(arg) = next(prog, &mut opts)? {
        match arg {
            Arg::Long("help") => return Err(crate::help(usage, flags)),
            Arg::Positional(operand) => {
                summary.count += 1;
                match summary.count {
                    1 => summary.first = Some(operand),
                    2 => summary.second = Some(operand),
                    _ => {}
                }
                summary.last = Some(operand);
            }
            other => return Err(invalid(prog, other)),
        }
    }
    Ok(summary)
}

/// Parses `head`'s and `tail`'s shared shape: an optional `-n`/`--lines COUNT` (attached or
/// separate, `-n5`/`-n 5`/`--lines=5`/`--lines 5` all work), defaulting to `default` if absent,
/// followed by the remaining positional arguments (the file list). One single pass, building the
/// file list as positionals are seen -- *not* `next` followed by a separate `operands` walk: `-n`
/// takes a value, and `operands`'s own "skip options" pass never calls `value` to consume it, so
/// it would see that value as a stray positional of its own (this module's own doc comment notes
/// exactly this is why `count_args`-shaped parsing can't reuse `operands`). A flag written after
/// a file name (`head file.txt -n 5`) is still recognized either way, the same as any other GNU
/// program. `Err` is the exit status to return, already reported (or, for `--help`, already
/// printed, exit `0`).
pub fn line_count(
    prog: &str,
    usage: &str,
    flags: &[(&str, &str)],
    args: Args,
    default: usize,
) -> Result<(usize, Vec<&'static str>), ExitCode> {
    let mut opts = opts(args);
    let mut count = default;
    let mut paths = Vec::new();
    while let Some(arg) = next(prog, &mut opts)? {
        match arg {
            Arg::Long("help") => return Err(crate::help(usage, flags)),
            Arg::Short('n') | Arg::Long("lines") => {
                let text = value(prog, &mut opts)?;
                let Ok(n) = text.parse() else {
                    report(prog, &format!("invalid number of lines: '{text}'"));
                    return Err(ExitCode(1));
                };
                count = n;
            }
            Arg::Positional(operand) => paths.push(operand),
            other => return Err(invalid(prog, other)),
        }
    }
    Ok((count, paths))
}
