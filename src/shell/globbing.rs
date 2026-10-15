//! Filename expansion: turns a glob pattern -- the unquoted parts of a word, quoted characters
//! escaped (`expand::Field::pattern`) -- into the sorted list of paths that match it. The
//! directory reading is a parameter (`list`), so the walk is pure and host-tested.
//!
//! The rules are the shell's: a pattern is matched one `/`-separated component at a time, each
//! against the entries of the directories the earlier components matched, so a wildcard never
//! matches a `/`; a name starting with `.` matches only a component that writes the dot
//! (`textlib::glob::matches_name`); a component without wildcards is just a name that has to be
//! there; a trailing `/` keeps only directories and is kept on the result; results are sorted
//! bytewise, which is `ls`'s order. Matching is case-sensitive, against the names as stored.

use alloc::string::String;
use alloc::vec::Vec;

use textlib::glob::{is_magic, matches_name, unescape};

/// One directory entry: its name, and whether it is a directory.
pub type Entry = (String, bool);

/// The paths matching `pattern`, as typed (relative stays relative: `src/*.rs` gives
/// `src/main.rs`), sorted; empty when nothing matches, which the caller reads as "leave the word
/// as it was". `list` returns the entries of the directory it is given -- `""` for the working
/// directory, otherwise the path built so far, with its trailing `/` -- or `None` if there is no
/// such directory.
pub fn expand(pattern: &str, list: &mut impl FnMut(&str) -> Option<Vec<Entry>>) -> Vec<String> {
    let absolute = pattern.starts_with('/');
    let components = split_components(pattern);
    let (components, want_dir) = match components.split_last() {
        Some((&"", rest)) => (rest.to_vec(), true),
        _ => (components, false),
    };
    if components.is_empty() {
        return Vec::new();
    }

    let mut prefixes: Vec<String> = alloc::vec![if absolute { "/".into() } else { String::new() }];
    let last_index = components.len() - 1;
    for (i, component) in components.iter().enumerate() {
        let final_component = i == last_index;
        let mut next = Vec::new();
        for prefix in &prefixes {
            let Some(entries) = list(prefix) else {
                continue;
            };
            let mut names: Vec<(&str, bool)> = Vec::new();
            if is_magic(component) {
                names.extend(
                    entries
                        .iter()
                        .filter(|(name, _)| matches_name(component, name))
                        .map(|(name, dir)| (name.as_str(), *dir)),
                );
            } else {
                let literal = unescape(component);
                names.extend(
                    entries
                        .iter()
                        .filter(|(name, _)| *name == literal)
                        .map(|(name, dir)| (name.as_str(), *dir)),
                );
            }
            for (name, is_dir) in names {
                // Everything but the last component has to be a directory to go further; the
                // last has to be one only when the pattern ended in `/`.
                if (!final_component || want_dir) && !is_dir {
                    continue;
                }
                let mut path = prefix.clone();
                path.push_str(name);
                if !final_component || want_dir {
                    path.push('/');
                }
                next.push(path);
            }
        }
        prefixes = next;
        if prefixes.is_empty() {
            break;
        }
    }
    prefixes.sort();
    prefixes
}

/// Splits `pattern` at its unescaped `/`s. `a//b` has an empty component between, which matches
/// nothing in the walk above and is dropped; a trailing `/` leaves an empty *last* component,
/// which `expand` reads as "directories only".
fn split_components(pattern: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = Vec::new();
    let mut start = 0;
    let mut chars = pattern.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '/' => {
                parts.push(&pattern[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&pattern[start..]);
    // Drop the empty piece before the first `/` of an absolute pattern and any doubled slash.
    let last = parts.len() - 1;
    parts
        .into_iter()
        .enumerate()
        .filter(|(i, p)| !p.is_empty() || *i == last)
        .map(|(_, p)| p)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    /// A little tree: `/` has `a.c`, `b.c`, `.hidden`, `notes.txt`, `src/` and `Doc/`; `src/` has
    /// `main.rs`, `lib.rs`, `mod/`; `src/mod/` has `x.rs`; `Doc/` has `a.c`.
    fn tree(path: &str) -> Option<Vec<Entry>> {
        let e = |name: &str, dir: bool| (name.to_string(), dir);
        match path {
            "" | "/" => Some(alloc::vec![
                e("a.c", false),
                e("b.c", false),
                e(".hidden", false),
                e("notes.txt", false),
                e("src", true),
                e("Doc", true),
            ]),
            "src/" | "/src/" => Some(alloc::vec![
                e("main.rs", false),
                e("lib.rs", false),
                e("mod", true),
            ]),
            "src/mod/" | "/src/mod/" => Some(alloc::vec![e("x.rs", false)]),
            "Doc/" | "/Doc/" => Some(alloc::vec![e("a.c", false)]),
            _ => None,
        }
    }

    fn go(pattern: &str) -> Vec<String> {
        expand(pattern, &mut tree)
    }

    #[test]
    fn a_star_in_the_working_directory_is_sorted_and_skips_dot_names() {
        assert_eq!(go("*.c"), ["a.c", "b.c"]);
        assert_eq!(go("*"), ["Doc", "a.c", "b.c", "notes.txt", "src"]); // bytewise: `D` < `a`
    }

    #[test]
    fn a_dot_must_be_written() {
        assert_eq!(go(".*"), [".hidden"]);
        assert_eq!(go("[.]hidden"), Vec::<String>::new());
        assert_eq!(go("?hidden"), Vec::<String>::new());
    }

    #[test]
    fn no_match_is_empty() {
        assert_eq!(go("*.zzz"), Vec::<String>::new());
        assert_eq!(go("nosuchdir/*"), Vec::<String>::new());
    }

    #[test]
    fn a_wildcard_never_matches_a_slash() {
        assert_eq!(go("src/*"), ["src/lib.rs", "src/main.rs", "src/mod"]);
        assert_eq!(go("*/*.c"), ["Doc/a.c"]);
        assert_eq!(go("*/x.rs"), Vec::<String>::new()); // x.rs is two levels down
        assert_eq!(go("*/*/x.rs"), ["src/mod/x.rs"]);
    }

    #[test]
    fn a_component_without_wildcards_must_just_exist() {
        assert_eq!(go("src/*.rs"), ["src/lib.rs", "src/main.rs"]);
        assert_eq!(go("src/mod/*.rs"), ["src/mod/x.rs"]);
        assert_eq!(go("Doc/a.c"), ["Doc/a.c"]); // reached through a literal, no wildcard anywhere
    }

    #[test]
    fn matching_is_case_sensitive() {
        assert_eq!(go("doc/*"), Vec::<String>::new());
        assert_eq!(go("D*/*"), ["Doc/a.c"]);
        assert_eq!(go("*.C"), Vec::<String>::new());
    }

    #[test]
    fn a_trailing_slash_keeps_only_directories_and_the_slash() {
        assert_eq!(go("*/"), ["Doc/", "src/"]);
        assert_eq!(go("src/*/"), ["src/mod/"]);
        assert_eq!(go("*.c/"), Vec::<String>::new());
    }

    #[test]
    fn an_absolute_pattern_stays_absolute() {
        assert_eq!(go("/*.c"), ["/a.c", "/b.c"]);
        assert_eq!(go("/src/*.rs"), ["/src/lib.rs", "/src/main.rs"]);
        assert_eq!(go("/*/"), ["/Doc/", "/src/"]);
    }

    #[test]
    fn a_doubled_slash_is_one() {
        assert_eq!(go("src//*.rs"), ["src/lib.rs", "src/main.rs"]);
    }

    #[test]
    fn a_wildcard_in_the_middle_of_a_name() {
        assert_eq!(go("[ab].c"), ["a.c", "b.c"]);
        assert_eq!(go("?.c"), ["a.c", "b.c"]);
        assert_eq!(go("n*s.txt"), ["notes.txt"]);
        assert_eq!(go("n*x.txt"), Vec::<String>::new());
        assert_eq!(go("n*s.t[x]t"), ["notes.txt"]);
    }

    #[test]
    fn an_escaped_wildcard_is_a_name() {
        // `\*` is a literal star in a component that has another wildcard; with none it is a
        // literal name that is simply not there.
        assert_eq!(go("\\*.c"), Vec::<String>::new());
        assert_eq!(go("a\\.c"), ["a.c"]);
    }

    #[test]
    fn a_missing_directory_is_no_match_not_an_error() {
        assert_eq!(go("src/zz/*"), Vec::<String>::new());
    }
}
