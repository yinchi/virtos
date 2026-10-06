//! Copying files and directory trees, and removing them -- what `cp` does, what `mv` does between
//! two volumes (where the kernel's `rename` says `EXDEV`), and what `rm -r` does. The engine
//! reports in `cp`/`mv`/`rm`'s own voice (`Copier::prog`), and its failures are `Err(Reported)`
//! after the report, so a caller only decides what to do next.
//!
//! A directory is copied by creating the destination directory and copying its entries into it,
//! in name order; an existing destination directory is merged into. Every file's size is checked
//! once it is closed: a copy that came up short (a full disk) is an error, not a quiet
//! truncation. With `preserve`, a copied file keeps its executable and read-only bits and every
//! copy keeps its modify time -- a directory's after its contents are in, since putting entries
//! in may touch it -- through `utimens`.

extern crate alloc;

use alloc::format;
use alloc::string::String;

use textlib::stamp::fat_to_unix;
use userlib::{
    ATTR_EXEC, ATTR_READ_ONLY, O_RDONLY, O_WRONLY, Stat, TimeSet, chmod, close, mkdir, open, read,
    stat, unlink, utimens,
};

use crate::{cli, join, read_dir, write_all_fd};

/// A copy or removal that failed, and has already said why: the caller only decides what to do
/// next.
#[derive(Debug)]
pub struct Reported;

/// What a copy does, and who is talking.
pub struct Copier {
    /// The program's name, for its messages.
    pub prog: &'static str,
    /// Keep the executable and read-only bits, and the modify time.
    pub preserve: bool,
    /// Leave a file that already exists at the destination alone, without a word (`cp -n`).
    pub no_clobber: bool,
    /// Say each copy made: `'src' -> 'dst'` (`cp -v`).
    pub verbose: bool,
}

const BUF_SIZE: usize = 512;

impl Copier {
    fn announce(&self, src: &str, dst: &str) {
        if self.verbose {
            cli::say(&format!("'{src}' -> '{dst}'"));
        }
    }

    fn fail(&self, what: &str) -> Reported {
        cli::report(self.prog, what);
        Reported
    }

    /// `fail` for a syscall that returned the negated errno `errno`: `<what>: <what it means>`.
    fn fail_errno(&self, what: &str, errno: isize) -> Reported {
        cli::report_errno(self.prog, what, errno);
        Reported
    }

    /// Gives `dst` what `preserve` asks of a copy of `src`: for a file, its executable and
    /// read-only bits; for both, its modify time.
    fn preserve_from(&self, info: &Stat, dst: &str) -> Result<(), Reported> {
        if !self.preserve {
            return Ok(());
        }
        let mut result = Ok(());
        if !info.is_dir {
            const BITS: u8 = ATTR_EXEC | ATTR_READ_ONLY;
            let want = info.attrs & BITS;
            let r = chmod(dst, want, BITS & !want);
            if r < 0 {
                result = Err(self.fail_errno(&format!("{dst}: failed to preserve permissions"), r));
            }
        }
        if let Some(unix) = fat_to_unix(info.modified_date, info.modified_time)
            && utimens(dst, TimeSet::Omit, TimeSet::At(unix)) < 0
        {
            result = Err(self.fail(&format!("{dst}: failed to preserve the modify time")));
        }
        result
    }

    /// Copies the plain file `src` to `dst` (created, or truncated to empty if it exists). The
    /// first chunk is read before `dst` is opened -- and so truncated: a source that can't be read
    /// at all (a directory, say) must not destroy an existing destination first.
    pub fn file(&self, src: &str, dst: &str) -> Result<(), Reported> {
        if self.no_clobber && stat(dst).is_ok() {
            return Ok(());
        }
        let input = open(src, O_RDONLY);
        if input < 0 {
            return Err(self.fail_errno(src, input));
        }
        let input = input as usize;
        let mut buf = [0u8; BUF_SIZE];
        let first = read(input, &mut buf);
        if first < 0 {
            close(input);
            return Err(self.fail_errno(src, first));
        }
        let output = open(dst, O_WRONLY);
        if output < 0 {
            close(input);
            return Err(self.fail_errno(dst, output));
        }
        let output = output as usize;

        let mut result = Ok(());
        let mut n = first;
        while n > 0 {
            if let Err(e) = write_all_fd(output, &buf[..n as usize]) {
                result = Err(self.fail_errno(dst, e));
                break;
            }
            n = read(input, &mut buf);
            if n < 0 {
                result = Err(self.fail_errno(src, n));
                break;
            }
        }
        close(input);
        // Closing `output` is what commits its final size to disk -- a failure here is a failed
        // copy.
        let closed = close(output);
        if closed < 0 && result.is_ok() {
            result = Err(self.fail_errno(dst, closed));
        }
        result?;

        let (Ok(from), Ok(to)) = (stat(src), stat(dst)) else {
            return Err(self.fail(&format!("{dst}: cannot check the copy")));
        };
        if from.size != to.size {
            return Err(self.fail(&format!("{dst}: the copy is not the size of the original")));
        }
        self.preserve_from(&from, dst)?;
        self.announce(src, dst);
        Ok(())
    }

    /// Copies the directory `src` to `dst` and everything under it: `dst` is created if it
    /// doesn't exist, or merged into if it's already a directory -- a hard error if it names
    /// anything else. Every entry is attempted even after a failure.
    pub fn tree(&self, src: &str, dst: &str) -> Result<(), Reported> {
        let info = stat(src).map_err(|e| self.fail_errno(src, e))?;
        match stat(dst) {
            Ok(existing) if existing.is_dir => {}
            Ok(_) => {
                return Err(self.fail(&format!(
                    "cannot overwrite non-directory '{dst}' with directory '{src}'"
                )));
            }
            Err(_) => {
                let r = mkdir(dst);
                if r < 0 {
                    return Err(self.fail_errno(&format!("cannot create directory '{dst}'"), r));
                }
            }
        }
        self.announce(src, dst);
        let mut entries = read_dir(src).map_err(|e| self.fail_errno(src, e))?;
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        let mut result = Ok(());
        for ent in entries {
            let from = join(src, &ent.name);
            let to = join(dst, &ent.name);
            let copied = if ent.attrs & userlib::ATTR_DIRECTORY != 0 {
                self.tree(&from, &to)
            } else {
                self.file(&from, &to)
            };
            if copied.is_err() {
                result = Err(Reported);
            }
        }
        // The directory's own time last: putting entries in it may have touched it.
        let kept = self.preserve_from(&info, dst);
        result.and(kept)
    }
}

/// Says a removal was made, for `rm -v` (`removed 'f'`, `removed directory 'd'`).
pub fn removed(path: &str, is_dir: bool, verbose: bool) {
    if verbose {
        cli::say(&format!(
            "removed {}'{path}'",
            if is_dir { "directory " } else { "" }
        ));
    }
}

/// Removes the file or directory `path`, and everything under a directory: every entry is
/// attempted even after a failure, a directory's own removal last. Reports each failure as
/// `<prog>: cannot remove '<path>': <why>`.
pub fn remove_tree(prog: &str, path: &str, verbose: bool) -> Result<(), Reported> {
    let mut result = Ok(());
    let is_dir = matches!(stat(path), Ok(info) if info.is_dir);
    if is_dir {
        match read_dir(path) {
            Ok(entries) => {
                for ent in entries {
                    let child = join(path, &ent.name);
                    let removed = if ent.attrs & userlib::ATTR_DIRECTORY != 0 {
                        remove_tree(prog, &child, verbose)
                    } else {
                        let r = unlink(&child);
                        if r < 0 {
                            cli::report_errno(prog, &format!("cannot remove '{child}'"), r);
                            Err(Reported)
                        } else {
                            removed(&child, false, verbose);
                            Ok(())
                        }
                    };
                    if removed.is_err() {
                        result = Err(Reported);
                    }
                }
            }
            Err(e) => {
                cli::report_errno(prog, &format!("cannot remove '{path}'"), e);
                return Err(Reported);
            }
        }
    }
    // Only once everything inside is gone does the entry itself go.
    if result.is_ok() {
        let r = unlink(path);
        if r < 0 {
            cli::report_errno(prog, &format!("cannot remove '{path}'"), r);
            result = Err(Reported);
        } else {
            removed(path, is_dir, verbose);
        }
    }
    result
}

/// `dir/.<name>` -- or, for a `target` with no directory part, just `.<name>` -- the first of
/// `<name>`, `<name>.1`, `<name>.2`, ... that does not exist yet. `None` if a thousand are taken
/// (a directory full of leftovers). A hidden temporary in the destination's own directory, so the
/// final `rename` into place stays on one volume.
pub fn partial_path(target: &str, name: &str) -> Option<String> {
    let dir = match target.rfind('/') {
        Some(0) => "/",
        Some(at) => &target[..at],
        None => "",
    };
    let named = |leaf: &str| {
        if dir.is_empty() {
            String::from(leaf)
        } else {
            join(dir, leaf)
        }
    };
    let first = named(name);
    if stat(&first).is_err() {
        return Some(first);
    }
    (1..1000)
        .map(|n| named(&format!("{name}.{n}")))
        .find(|path| stat(path).is_err())
}
