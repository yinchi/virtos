//! The kernel's one working directory. There's no process model, so this is global state, not
//! per-program -- the same reasoning `syscall::fd`'s single fd table already rests on ("at most
//! one program is ever resident"). Read by every path-taking syscall (`syscall::fd`'s `open`/
//! `mkdir`/`unlink`/`rename`/`stat`, via `absolute`) and written only by the shell's `cd` builtin
//! (`shell::run`) -- kept here, in `fs`, rather than in `shell`, so `syscall` depends only on `fs`
//! (which it already leans on heavily), not on the interactive REPL layer above it.

use alloc::string::String;
use alloc::vec::Vec;

/// Empty means the root -- consistent with `fs::path::resolve`'s own "no components" convention,
/// so nothing here needs a separate "just started, not yet set" case.
///
/// SAFETY (every access, via `cwd`): single core, and every syscall runs with IRQs masked
/// (`vectors.s`'s trap entry); the shell's own write (`set`) happens between programs, never
/// concurrently with one.
static mut CWD: String = String::new();

#[allow(clippy::deref_addrof)]
fn cwd() -> &'static mut String {
    // SAFETY: see CWD.
    unsafe { &mut *(&raw mut CWD) }
}

/// The working directory as an absolute path -- `"/"` at the root, never empty (unlike the
/// internal `CWD` representation, which uses "" for exactly that case).
pub fn get() -> String {
    if cwd().is_empty() {
        String::from("/")
    } else {
        cwd().clone()
    }
}

/// Sets the working directory to `path`, which must already be absolute and normalized (`absolute`
/// produces exactly that shape). The caller (the shell's `cd`) is responsible for having already
/// checked it names a real directory -- this just records it.
pub fn set(path: String) {
    *cwd() = path;
}

/// Resolves `path` to an absolute path: unchanged (up to normalization) if it already starts with
/// `/`, otherwise joined onto the working directory. Normalizes `.`/`..` lexically component by
/// component, not by touching the filesystem -- `..` past the root just stays at the root, the
/// same as a real shell's `cd ../../..` from near the top. This is what lets every path-taking
/// syscall accept a path relative to wherever the shell's `cd` last left it, not just an absolute
/// one, without each syscall re-deriving this itself.
pub fn absolute(path: &str) -> String {
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        cwd().split('/').filter(|c| !c.is_empty()).collect()
    };
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    if parts.is_empty() {
        return String::from("/");
    }
    let mut result = String::new();
    for part in parts {
        result.push('/');
        result.push_str(part);
    }
    result
}
