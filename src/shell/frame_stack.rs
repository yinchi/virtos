//! The shell's own scoping mechanism: a stack of frames, each holding its own environment
//! variables, with two distinct ways to create a new one from a copy of the current one --
//! `with_child` (only exported variables survive, for `sh FILE`) and `with_subshell` (every
//! variable, exported or not, survives, for each pipeline stage) -- plus `source`/`.`, which push
//! nothing at all and run directly against the current frame, so their effects stick in the
//! caller. Mirrors the reference lab's own frame-stack mechanism (`exec`-style vs. `fork`-style
//! scoping), simplified for a shell that only ever runs one frame's code at a time (never truly
//! concurrent processes).
//!
//! The working directory itself still lives in `fs::cwd`, not here -- see that module's own doc
//! comment for why (`syscall` depends only on `fs`, never on this interactive layer). `with_child`/
//! `with_subshell` only take a *snapshot* of it going in, restoring `fs::cwd` to that snapshot
//! coming back out -- enough to stop a child/subshell's own `cd` leaking back to its caller,
//! without `fs::cwd` itself needing to know frames exist at all.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// One environment variable: a name, a value, and whether it's exported (visible in a launched
/// program's own `envp`). Every variable the `export`/`unset` builtins create goes through
/// `apply_export`, so `exported` is always `true` today; `apply_set` (bare `NAME=value`, no
/// `export`) is the one case that can leave it `false`, and has no caller yet -- built now
/// anyway, matching this project's standing precedent of carrying a real mechanism before its
/// first producer exists (`exec::argplan`'s own envp support, long before anything produced a
/// real environment).
#[derive(Clone, Debug, PartialEq)]
struct Var {
    name: String,
    value: String,
    exported: bool,
}

/// One level of shell scope: its own environment variables.
struct ShellFrame {
    vars: Vec<Var>,
}

/// SAFETY (every access, via `frames`): single core, and the shell runs strictly sequentially --
/// same reasoning `shell::PIPE_COUNTER`/`fs::cwd`'s own `CWD` already rest on.
static mut FRAMES: Vec<ShellFrame> = Vec::new();

/// The current frame stack, guaranteed non-empty -- a root frame is pushed on first access.
#[allow(clippy::deref_addrof)]
fn frames() -> &'static mut Vec<ShellFrame> {
    // SAFETY: see FRAMES.
    let frames = unsafe { &mut *(&raw mut FRAMES) };
    if frames.is_empty() {
        frames.push(ShellFrame { vars: Vec::new() });
    }
    frames
}

fn top() -> &'static ShellFrame {
    frames()
        .last()
        .expect("frames() always returns a non-empty stack")
}

fn top_mut() -> &'static mut ShellFrame {
    frames()
        .last_mut()
        .expect("frames() always returns a non-empty stack")
}

/// `[A-Za-z_][A-Za-z0-9_]*` -- matches the reference lab's own identifier rule exactly.
/// `pub(super)` so `lexer`'s own `$NAME`/`${NAME}` scanning can reuse the same rule rather than
/// duplicating it -- the one thing a variable *reference* and a variable *definition* both need
/// to agree on.
pub(super) fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `export NAME[=VALUE]`: always marks `name` exported, assigning `value` first if given.
/// Exporting a name that isn't set yet, with no value, does nothing -- matches the lab's own
/// `export_var` exactly. `Err` if `name` isn't a valid identifier.
fn apply_export(vars: &mut Vec<Var>, name: &str, value: Option<&str>) -> Result<(), ()> {
    if !is_valid_name(name) {
        return Err(());
    }
    if let Some(existing) = vars.iter_mut().find(|v| v.name == name) {
        existing.exported = true;
        if let Some(value) = value {
            existing.value = value.to_string();
        }
    } else if let Some(value) = value {
        vars.push(Var {
            name: name.to_string(),
            value: value.to_string(),
            exported: true,
        });
    }
    Ok(())
}

/// Bare `NAME=VALUE` assignment: keeps an existing variable's exported flag, defaults a brand new
/// one to unexported. `run_segment`'s own call, for a segment that's nothing but a leading
/// assignment prefix with no command following it.
fn apply_set(vars: &mut Vec<Var>, name: &str, value: &str) -> Result<(), ()> {
    if !is_valid_name(name) {
        return Err(());
    }
    if let Some(existing) = vars.iter_mut().find(|v| v.name == name) {
        existing.value = value.to_string();
    } else {
        vars.push(Var {
            name: name.to_string(),
            value: value.to_string(),
            exported: false,
        });
    }
    Ok(())
}

/// Removes `name`, set or not -- removing one that was never set is not an error (POSIX's own
/// rule for `unset`).
fn apply_unset(vars: &mut Vec<Var>, name: &str) {
    vars.retain(|v| v.name != name);
}

/// Only the exported variables of `vars`, cloned -- `with_child`'s own filter, matching real
/// `execve`'s envp-only inheritance (a child sees nothing the parent didn't choose to export).
fn only_exported(vars: &[Var]) -> Vec<Var> {
    vars.iter().filter(|v| v.exported).cloned().collect()
}

/// The current value of `name`, exported or not -- `shell::expand_argv`'s own variable lookup.
pub fn var(name: &str) -> Option<&'static str> {
    top()
        .vars
        .iter()
        .find(|v| v.name == name)
        .map(|v| v.value.as_str())
}

/// See `apply_export`.
pub fn export_var(name: &str, value: Option<&str>) -> Result<(), ()> {
    apply_export(&mut top_mut().vars, name, value)
}

/// See `apply_set`.
pub fn set_var(name: &str, value: &str) -> Result<(), ()> {
    apply_set(&mut top_mut().vars, name, value)
}

/// See `apply_unset`.
pub fn unset_var(name: &str) {
    apply_unset(&mut top_mut().vars, name);
}

/// Every exported variable, in storage order -- a launched program's own `envp`, directly.
pub fn exported() -> impl Iterator<Item = (&'static str, &'static str)> {
    top()
        .vars
        .iter()
        .filter(|v| v.exported)
        .map(|v| (v.name.as_str(), v.value.as_str()))
}

/// Applies each `(name, value)` in `overlay` to `vars` as always-exported (bash's own rule: a
/// prefix assignment is visible in the command's environment regardless of whether the name is
/// otherwise an exported shell variable) -- same mechanism as `apply_export`, just looped.
/// Infallible: `overlay`'s names already passed the identical `is_valid_name` check at lex time
/// (`lexer::detect_assign`), so `apply_export` can never actually reject one here.
fn apply_overlay(vars: &mut Vec<Var>, overlay: &[(String, String)]) {
    for (name, value) in overlay {
        let _ = apply_export(vars, name, Some(value));
    }
}

/// Applies `overlay` (each `(NAME, VALUE)` from a segment's leading assignment words,
/// `shell::mod`'s own `split_assignments`) to the current frame, runs `f`, then restores the
/// whole frame to what it was before -- a whole-`Vec` clone-and-restore, not the lab's own
/// per-name `saved_var`/`restore_var`: deliberately simpler, and equivalent for an *ordinary*
/// command or external program (neither can make a permanent change to `vars` that this restore
/// would need to keep) -- not equivalent, and known not to be, for a command that itself
/// permanently mutates `vars` (`export`/`unset`, or a `source`d script's own `export`): this
/// restore undoes that too, indiscriminately, exactly the bug caught (`run_segment`'s own call
/// site skips this function entirely when there's no overlay to apply, which is what makes a
/// prefix-less `export FOO=bar` stick). A prefix assignment in front of one of those specific
/// builtins (`FOO=bar export BAZ=qux`) would still lose `BAZ` once this function wraps it --
/// POSIX's own "special builtins keep a prefix assignment permanently" rule, not implemented --
/// a known, narrow gap, not silently assumed away. Real caller: `run_segment`, for a command with
/// a leading assignment prefix (`NAME=value cmd`).
pub fn with_overlay<R>(overlay: &[(String, String)], f: impl FnOnce() -> R) -> R {
    let saved = top().vars.clone();
    apply_overlay(&mut top_mut().vars, overlay);
    let result = f();
    top_mut().vars = saved;
    result
}

/// Pushes a new frame, runs `f` against it, then pops it back off -- closure-shaped so a caller
/// can't forget the matching pop. `vars` starts as a copy of only the current top's *exported*
/// variables, re-marked exported; `fs::cwd` is snapshotted going in and restored coming back out,
/// undoing whatever `f` did to it (see this module's own doc comment for why that's correct
/// without `fs::cwd` needing to know frames exist). Real, immediate caller: the `sh` builtin.
///
/// Gated like `fs::cwd` itself (`fs/mod.rs`'s own `#[cfg(target_os = "none")]` on its `cwd`
/// declaration) -- everything above this point in the file is pure `Vec<Var>` manipulation with
/// no such dependency, and stays host-testable.
#[cfg(target_os = "none")]
pub fn with_child<R>(f: impl FnOnce() -> R) -> R {
    push_and_run(only_exported(&top().vars), f)
}

/// Same shape as `with_child`, but `vars` starts as a full copy of the current top -- every
/// variable, exported or not (matches `fork`'s own full-memory-copy semantics). Caller: each
/// pipeline stage.
#[cfg(target_os = "none")]
pub fn with_subshell<R>(f: impl FnOnce() -> R) -> R {
    push_and_run(top().vars.clone(), f)
}

#[cfg(target_os = "none")]
fn push_and_run<R>(vars: Vec<Var>, f: impl FnOnce() -> R) -> R {
    let cwd = crate::fs::cwd::get();
    frames().push(ShellFrame { vars });
    let result = f();
    frames().pop();
    crate::fs::cwd::set(cwd);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn var(name: &str, value: &str, exported: bool) -> Var {
        Var {
            name: name.to_string(),
            value: value.to_string(),
            exported,
        }
    }

    #[test]
    fn export_creates_and_marks_exported() {
        let mut vars = Vec::new();
        assert_eq!(apply_export(&mut vars, "FOO", Some("bar")), Ok(()));
        assert_eq!(vars, [var("FOO", "bar", true)]);
    }

    #[test]
    fn export_with_no_value_on_an_unset_name_is_a_no_op() {
        let mut vars = Vec::new();
        assert_eq!(apply_export(&mut vars, "FOO", None), Ok(()));
        assert!(vars.is_empty());
    }

    #[test]
    fn export_on_an_existing_unexported_var_marks_it_exported_without_a_value() {
        let mut vars = vec![var("FOO", "old", false)];
        assert_eq!(apply_export(&mut vars, "FOO", None), Ok(()));
        assert_eq!(vars, [var("FOO", "old", true)]);
    }

    #[test]
    fn export_rejects_invalid_names() {
        let mut vars = Vec::new();
        assert_eq!(apply_export(&mut vars, "1abc", Some("x")), Err(()));
        assert_eq!(apply_export(&mut vars, "a-b", Some("x")), Err(()));
        assert!(vars.is_empty());
    }

    #[test]
    fn set_creates_a_new_var_unexported() {
        let mut vars = Vec::new();
        assert_eq!(apply_set(&mut vars, "FOO", "bar"), Ok(()));
        assert_eq!(vars, [var("FOO", "bar", false)]);
    }

    #[test]
    fn set_on_an_existing_var_keeps_its_exported_flag() {
        let mut vars = vec![var("FOO", "old", true)];
        assert_eq!(apply_set(&mut vars, "FOO", "new"), Ok(()));
        assert_eq!(vars, [var("FOO", "new", true)]);
    }

    #[test]
    fn unset_removes_a_var() {
        let mut vars = vec![var("FOO", "bar", true), var("BAZ", "qux", false)];
        apply_unset(&mut vars, "FOO");
        assert_eq!(vars, [var("BAZ", "qux", false)]);
    }

    #[test]
    fn unsetting_a_name_that_was_never_set_is_not_an_error() {
        let mut vars = vec![var("FOO", "bar", true)];
        apply_unset(&mut vars, "NOSUCH");
        assert_eq!(vars, [var("FOO", "bar", true)]);
    }

    #[test]
    fn only_exported_filters_out_unexported_vars() {
        let vars = vec![
            var("A", "1", true),
            var("B", "2", false),
            var("C", "3", true),
        ];
        assert_eq!(
            only_exported(&vars),
            [var("A", "1", true), var("C", "3", true)]
        );
    }

    #[test]
    fn overlay_marks_a_new_var_exported() {
        let mut vars = Vec::new();
        apply_overlay(&mut vars, &[("FOO".to_string(), "bar".to_string())]);
        assert_eq!(vars, [var("FOO", "bar", true)]);
    }

    #[test]
    fn overlay_marks_an_existing_unexported_var_exported_for_its_new_value() {
        let mut vars = vec![var("FOO", "old", false)];
        apply_overlay(&mut vars, &[("FOO".to_string(), "new".to_string())]);
        assert_eq!(vars, [var("FOO", "new", true)]);
    }

    #[test]
    fn overlay_applies_several_assignments_in_order() {
        let mut vars = Vec::new();
        apply_overlay(
            &mut vars,
            &[
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "2".to_string()),
            ],
        );
        assert_eq!(vars, [var("A", "1", true), var("B", "2", true)]);
    }
}
