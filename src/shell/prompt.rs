//! The shell prompt: `$PS1` with a few backslash escapes filled in, drawn fresh each time a prompt
//! is shown -- pure, so it is tested on the host.
//!
//! A deliberately small version of bash's `PS1`:
//! - `PS1` **unset or empty** gives the default, `> `.
//! - `\w` is the working directory (the whole path: there is no `~`), `\W` its last component
//!   (`/` for the root), `\$` a `#` (there is only root), and `\\` a backslash. **Any other
//!   backslash sequence stays as typed**, both characters.
//! - Nothing else is interpreted: no `$` expansion (bash expands `$VAR` in a prompt; here the
//!   value is text with those four escapes), no `\n` (a prompt of one row keeps the line editor's
//!   layout simple), no colours.
//! - **Control characters are dropped**, so a prompt cannot move the cursor or clear the screen;
//!   every other character stays, non-ASCII included (the console draws UTF-8, wide glyphs take
//!   two cells, and the line editor's layout counts cells, `console::input_layout`).
//! - A prompt over `MAX_PROMPT` characters keeps its **last** `MAX_PROMPT` (a long directory under
//!   `\w` must not leave no room to type, and the end is where the `> ` is).

use alloc::string::String;

/// What the prompt is when `PS1` is unset or empty.
pub const DEFAULT_PROMPT: &str = "> ";

/// What the shell sets `PS1` to at startup.
pub const DEFAULT_SHELL_PS1: &str = r"\w\$ ";

/// The longest prompt drawn, in characters.
pub const MAX_PROMPT: usize = 128;

/// The last component of the absolute path `cwd` (`/` for the root).
fn base_name(cwd: &str) -> &str {
    cwd.rsplit('/').find(|part| !part.is_empty()).unwrap_or("/")
}

/// The prompt for `ps1` (the variable's value, `None` if unset) while the working directory is
/// `cwd`.
pub fn render(ps1: Option<&str>, cwd: &str) -> String {
    let template = match ps1 {
        Some(text) if !text.is_empty() => text,
        _ => return String::from(DEFAULT_PROMPT),
    };
    let mut out = String::new();
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.clone().next() {
            Some('w') => out.push_str(cwd),
            Some('W') => out.push_str(base_name(cwd)),
            Some('$') => out.push('#'),
            Some('\\') => out.push('\\'),
            // Not one of ours: the backslash is text, and what follows is looked at on its own.
            _ => {
                out.push('\\');
                continue;
            }
        }
        chars.next(); // the escape's second character
    }
    out.retain(|c| !c.is_control());
    if out.is_empty() {
        return String::from(DEFAULT_PROMPT);
    }
    let count = out.chars().count();
    if count > MAX_PROMPT {
        out = out.chars().skip(count - MAX_PROMPT).collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn r(ps1: &str, cwd: &str) -> String {
        render(Some(ps1), cwd)
    }

    #[test]
    fn unset_or_empty_is_the_default() {
        assert_eq!(render(None, "/root"), "> ");
        assert_eq!(r("", "/root"), "> ");
    }

    #[test]
    fn plain_text_is_the_prompt() {
        assert_eq!(r("$ ", "/"), "$ ");
        assert_eq!(r("root@qemu> ", "/x"), "root@qemu> ");
    }

    #[test]
    fn w_is_the_whole_directory() {
        assert_eq!(r("\\w> ", "/root"), "/root> ");
        assert_eq!(r("\\w> ", "/"), "/> ");
        assert_eq!(r("[\\w]", "/a/b/c"), "[/a/b/c]");
        assert_eq!(r("\\w\\w", "/a"), "/a/a");
    }

    #[test]
    fn capital_w_is_the_last_component() {
        assert_eq!(r("\\W> ", "/root/bin"), "bin> ");
        assert_eq!(r("\\W> ", "/root"), "root> ");
        assert_eq!(r("\\W> ", "/"), "/> ");
        assert_eq!(r("\\W> ", "/a/b/"), "b> "); // a trailing slash does not make an empty name
    }

    #[test]
    fn dollar_is_a_hash() {
        assert_eq!(r("\\$ ", "/"), "# ");
        assert_eq!(r("\\w\\$ ", "/x"), "/x# ");
    }

    #[test]
    fn a_double_backslash_is_one() {
        assert_eq!(r("a\\\\b", "/"), "a\\b");
        // ...and the second one is not the start of another escape.
        assert_eq!(r("\\\\w", "/x"), "\\w");
    }

    #[test]
    fn other_backslashes_stay_as_typed() {
        assert_eq!(r("\\n> ", "/"), "\\n> ");
        assert_eq!(r("\\x41", "/"), "\\x41");
        assert_eq!(r("a\\", "/"), "a\\"); // a trailing one
        assert_eq!(r("\\ w", "/"), "\\ w");
        assert_eq!(r("\\\\\\w", "/x"), "\\/x"); // an escaped backslash, then \w
    }

    #[test]
    fn dollar_expansion_is_not_done() {
        assert_eq!(r("$HOME> ", "/"), "$HOME> ");
        assert_eq!(r("${X}", "/"), "${X}");
    }

    #[test]
    fn control_characters_are_dropped_and_the_rest_stays() {
        assert_eq!(r("a\tb\x1b[2Jc> ", "/"), "ab[2Jc> ");
        assert_eq!(r("a\nb> ", "/"), "ab> ");
        assert_eq!(r("\x07\x08", "/"), "> "); // nothing left: the default
        assert_eq!(r("é日> ", "/"), "é日> ");
        assert_eq!(r("\u{85}x", "/"), "x"); // a C1 control too
    }

    #[test]
    fn a_directory_with_non_ascii_letters_is_kept_whole() {
        assert_eq!(r("\\w> ", "/données"), "/données> ");
        assert_eq!(r("\\W> ", "/root/日本語"), "日本語> ");
    }

    #[test]
    fn a_long_prompt_keeps_its_end() {
        let long = "x".repeat(500) + "> ";
        let got = r(&long, "/");
        assert_eq!(got.chars().count(), MAX_PROMPT);
        assert!(got.ends_with("x> "));
        let deep = "/".to_string() + &"d/".repeat(200);
        let got = r("\\w> ", &deep);
        assert_eq!(got.chars().count(), MAX_PROMPT);
        assert!(got.ends_with("/> "));
    }

    #[test]
    fn the_length_limit_counts_characters_not_bytes() {
        let wide = "日".repeat(MAX_PROMPT);
        assert_eq!(r(&wide, "/").chars().count(), MAX_PROMPT);
        assert_eq!(r(&(wide + "x"), "/").chars().count(), MAX_PROMPT);
    }
}
