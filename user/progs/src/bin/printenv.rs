//! `printenv [NAME]...` -- with no operand, behaves exactly like `env` (every exported variable,
//! one `NAME=VALUE` per line). With one or more `NAME`s, prints each one's value only (no
//! `NAME=` prefix); a name that isn't set prints nothing for it and makes the whole command
//! exit `1` (GNU's own convention, not POSIX's, matching the reference lab's own `printenv`).
#![no_std]
#![no_main]

use userlib::{Args, Env, ExitCode};

userlib::entry_with_env!(run);

const USAGE: &str = "printenv [NAME]...";
const FLAGS: &[(&str, &str)] = &[];

/// `env`'s value for `name`, if set -- a linear scan same as every other small lookup here (the
/// lab's own `env::var` does the same, for the same reason: too few variables for anything
/// fancier to matter).
fn find(env: Env, name: &str) -> Option<&'static str> {
    env.filter_map(|pair| pair.split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn run(args: Args, env: Env) -> ExitCode {
    let plain = match progs::cli::plain("printenv", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    if plain.count == 0 {
        for pair in env {
            userlib::write(1, pair.as_bytes());
            userlib::write(1, b"\n");
        }
        return ExitCode(0);
    }
    let mut ok = true;
    for name in progs::cli::operands(args) {
        match find(env, name) {
            Some(value) => {
                userlib::write(1, value.as_bytes());
                userlib::write(1, b"\n");
            }
            None => ok = false,
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
