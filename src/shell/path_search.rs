//! Where a bare command name is looked for: the directories of `$PATH`, in order. Pure `no_std` + `alloc`, so it is
//! tested on the host; `launch.rs` turns each candidate into a lookup.
//!
//! The rules:
//! - `PATH` **unset** means `/usr/bin`, so a shell with no environment (or a test that sets none) finds the programs as
//!   it always has. **Set but empty** means no directory at all: nothing is found.
//! - The value is split at `:`. An **empty entry** (`a::b`, a leading or trailing `:`) is skipped: POSIX would search the
//!   working directory there, which is a good way to run the wrong program by accident.
//! - A **relative entry** (`bin`, `./tools`) is kept relative; the caller resolves it against the working directory
//!   when it looks, as bash does.
//! - In each directory the name is tried exactly as typed: `dir/name`.
//! - A name containing `/` is a path and is never searched for: `launch.rs` handles it before asking here.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// What `PATH` means when it is not set.
pub const DEFAULT_PATH: &str = "/usr/bin";

/// The directories `path` names, in order (`None` is unset), empty entries dropped.
pub fn directories(path: Option<&str>) -> Vec<&str> {
    path.unwrap_or(DEFAULT_PATH)
        .split(':')
        .filter(|dir| !dir.is_empty())
        .collect()
}

/// The paths to try for the command `name`, in order: `dir/name` for each directory.
pub fn candidates(path: Option<&str>, name: &str) -> Vec<String> {
    let mut found = Vec::new();
    for dir in directories(path) {
        // `/` alone must not become `//name`.
        let dir = if dir == "/" {
            ""
        } else {
            dir.strip_suffix('/').unwrap_or(dir)
        };
        found.push(format!("{dir}/{name}"));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(path: Option<&str>, name: &str) -> Vec<String> {
        candidates(path, name)
    }

    #[test]
    fn unset_is_usr_bin() {
        assert_eq!(c(None, "cat"), ["/usr/bin/cat"]);
        assert_eq!(directories(None), ["/usr/bin"]);
    }

    #[test]
    fn set_but_empty_is_nowhere() {
        assert!(c(Some(""), "cat").is_empty());
        assert!(c(Some(":"), "cat").is_empty());
        assert!(c(Some("::"), "cat").is_empty());
    }

    #[test]
    fn directories_are_tried_in_order() {
        assert_eq!(c(Some("/a:/b"), "x"), ["/a/x", "/b/x"]);
    }

    #[test]
    fn empty_entries_are_skipped_not_the_working_directory() {
        assert_eq!(c(Some(":/a"), "x"), ["/a/x"]);
        assert_eq!(c(Some("/a:"), "x"), ["/a/x"]);
        assert_eq!(c(Some("/a::/b"), "x"), ["/a/x", "/b/x"]);
    }

    #[test]
    fn relative_entries_stay_relative() {
        assert_eq!(c(Some("bin:./tools"), "x"), ["bin/x", "./tools/x"]);
        assert_eq!(c(Some(".."), "x"), ["../x"]);
    }

    #[test]
    fn a_trailing_slash_does_not_double() {
        assert_eq!(c(Some("/bin/"), "x"), ["/bin/x"]);
        assert_eq!(c(Some("/"), "x"), ["/x"]);
        assert_eq!(c(Some("dir/"), "x"), ["dir/x"]);
    }
}
