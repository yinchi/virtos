//! `mv src dst` -- renames or moves `src` to `dst`. Exactly one source and one destination, no
//! directory-destination support (`mv file existing_dir/`) and no replacing an existing file --
//! GNU's fuller `mv`(1) probes the destination with `stat` first to decide between those, which
//! this project doesn't have yet; `rename` itself already refuses if `dst` names anything at all.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "mv src dst";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: Args) -> ExitCode {
    let mut argv = args.skip(1);
    let (src, dst, extra) = (argv.next(), argv.next(), argv.next());
    if src == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    let (Some(src), Some(dst)) = (src, dst) else {
        userlib::write(2, b"mv: usage: mv src dst\n");
        return ExitCode(1);
    };
    if extra.is_some() {
        userlib::write(2, b"mv: usage: mv src dst\n");
        return ExitCode(1);
    }
    if src == dst {
        userlib::write(2, b"mv: src and dst must differ\n");
        return ExitCode(1);
    }

    if userlib::rename(src, dst) < 0 {
        userlib::write(2, b"mv: cannot move '");
        userlib::write(2, src.as_bytes());
        userlib::write(2, b"' to '");
        userlib::write(2, dst.as_bytes());
        userlib::write(2, b"'\n");
        return ExitCode(1);
    }
    ExitCode(0)
}
