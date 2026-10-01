//! `env` -- prints every exported environment variable, one `NAME=VALUE` per line. GNU/POSIX's
//! `env`'s "run a command in a modified environment" form (`env NAME=VALUE cmd`, `-i`, `-u`) is
//! deliberately not implemented here -- the shell's own `NAME=VALUE cmd` syntax, once it exists,
//! covers that job instead; this `env` only ever prints.
#![no_std]
#![no_main]

use userlib::{Args, Env, ExitCode};

userlib::entry_with_env!(run);

const USAGE: &str = "env";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: Args, env: Env) -> ExitCode {
    if args.nth(1) == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    for pair in env {
        userlib::write(1, pair.as_bytes());
        userlib::write(1, b"\n");
    }
    ExitCode(0)
}
