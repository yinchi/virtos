//! Builds a FAT16 disk image from a source directory tree -- the host-side replacement for
//! `scripts/folder_to_img.sh`'s mtools-based approach, built specifically because mtools cannot
//! set this project's own executable-bit convention (see `plans/PORTING.md`): a real FAT attribute
//! byte has no such concept, so it can only be set through a tool (this one, or the kernel itself)
//! that knows what the bit means. Setting it here, once, at build time, replaces the kernel doing
//! it fresh every boot the way the reference lab's does.
//!
//! Usage: `mkimage <image> <size> <label> <serial> <source_dir>` -- mirrors
//! `folder_to_img.sh`'s argument order exactly, so it's a drop-in replacement in the justfile.

use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::raw::DirEntryAttrFlags;
use hadris_fat::sync::{DirectoryEntry, FatDir, FatVolume, FatVolumeWriteExt};
use hadris_fat::time::{FatDateTime, StaticTimeProvider};

/// This project's own convention for "this file may be launched as a program" -- one of the FAT
/// attribute byte's two bits the spec leaves reserved/undefined (0x40). Not yet enforced by the
/// kernel (no launcher exists yet), but setting it here now means every future image already has
/// it right, rather than the kernel needing to set it fresh at every boot the way the reference
/// lab's does. TODO: move to a shared `abi`-style crate once one exists (see
/// `plans/USERSPACE_PLAN.md`), so the kernel and this tool aren't two independent copies of the
/// same magic number.
const ATTR_EXEC: u8 = 0x40;

/// The directory (relative to the image root) whose files get `ATTR_EXEC` -- silently skipped if
/// the image being built doesn't have one (e.g. the home disk, which holds no programs).
const EXECUTABLE_DIR: &str = "usr/bin";

/// A fixed build timestamp instead of the host clock, for byte-for-byte reproducible images --
/// same reasoning as `folder_to_img.sh`'s `SOURCE_DATE_EPOCH` (2026-01-01T00:00:00Z).
static BUILD_TIME: std::sync::LazyLock<StaticTimeProvider> =
    std::sync::LazyLock::new(|| StaticTimeProvider(FatDateTime::new(2026, 1, 1, 0, 0, 0)));

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let [_, image, size, label, serial, source_dir] = args.as_slice() else {
        eprintln!("usage: mkimage <image> <size> <label> <serial> <source_dir>");
        return ExitCode::FAILURE;
    };

    if let Err(e) = run(image, size, label, serial, source_dir) {
        eprintln!("mkimage: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run(
    image: &str,
    size: &str,
    label: &str,
    serial: &str,
    source_dir: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let size_bytes = parse_size(size)?;
    let volume_id = u32::from_str_radix(serial, 16)?;

    let _ = std::fs::remove_file(image);
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(image)?;
    file.set_len(size_bytes)?;

    let options = FatFormatOptions::new(size_bytes)
        .volume_label(label)
        .fat_type(FatTypeSelection::Fat16)
        .volume_id(volume_id);
    // Formatting itself touches no per-file timestamp; the fixed clock matters for what gets
    // created below.
    FatVolumeFormatter::format(file, options)?;

    let file = File::options().read(true).write(true).open(image)?;
    let vol = FatVolume::builder(file)
        .time_provider(&*BUILD_TIME)
        .open()?;

    populate(&vol, Path::new(source_dir))?;
    mark_executable(&vol)?;

    Ok(())
}

/// Parses a size like `folder_to_img.sh`'s (and `truncate`'s) argument: a plain byte count, or one
/// suffixed `K`/`M`/`G` (binary, 1024-based -- matching `truncate`'s own suffix meaning).
fn parse_size(s: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let (digits, mult) = match s.chars().last() {
        Some('K' | 'k') => (&s[..s.len() - 1], 1024),
        Some('M' | 'm') => (&s[..s.len() - 1], 1024 * 1024),
        Some('G' | 'g') => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        _ => (s, 1),
    };
    Ok(digits.parse::<u64>()? * mult)
}

/// Copies `source_dir`'s whole tree into the just-formatted volume: directories before files, both
/// in sorted order, matching `folder_to_img.sh`'s own walk (a parent's path always sorts before
/// its children, so directory creation never runs ahead of its parent existing).
fn populate(vol: &FatVolume<File>, source_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    walk(source_dir, source_dir, &mut dirs, &mut files)?;
    dirs.sort();
    files.sort();

    for rel in &dirs {
        create_dir(vol, rel)?;
    }
    for rel in &files {
        let parent = rel.parent().unwrap_or_else(|| Path::new(""));
        let dir = open_dir(vol, parent)?;
        let name = rel
            .file_name()
            .unwrap()
            .to_str()
            .ok_or("non-UTF-8 file name")?;
        let entry = vol.create_file(&dir, name)?;
        let bytes = std::fs::read(source_dir.join(rel))?;
        let mut writer = vol.write_file(&entry)?;
        writer.write(&bytes)?;
        writer.finish()?;
    }
    Ok(())
}

/// Walks `dir` recursively, collecting every entry's path relative to `root` into `dirs`/`files`.
fn walk(
    root: &Path,
    dir: &Path,
    dirs: &mut Vec<PathBuf>,
    files: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap().to_path_buf();
        if path.is_dir() {
            dirs.push(rel.clone());
            walk(root, &path, dirs, files)?;
        } else {
            files.push(rel);
        }
    }
    Ok(())
}

fn create_dir(vol: &FatVolume<File>, rel: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let parent = rel.parent().unwrap_or_else(|| Path::new(""));
    let dir = open_dir(vol, parent)?;
    let name = rel
        .file_name()
        .unwrap()
        .to_str()
        .ok_or("non-UTF-8 dir name")?;
    vol.create_dir(&dir, name)?;
    Ok(())
}

/// Opens the directory at `rel` (relative to the volume root; empty for the root itself),
/// component by component -- every path this tool creates has already had its ancestors created
/// first (`populate`'s sorted order), so each lookup here is expected to succeed.
fn open_dir<'a>(
    vol: &'a FatVolume<File>,
    rel: &Path,
) -> Result<FatDir<'a, File>, Box<dyn std::error::Error>> {
    let mut dir = vol.root_dir();
    for component in rel.components() {
        let name = component
            .as_os_str()
            .to_str()
            .ok_or("non-UTF-8 path component")?;
        let entry = find_entry(&dir, name)?.ok_or_else(|| format!("{name}: not found"))?;
        dir = dir.open_entry(&entry)?;
    }
    Ok(dir)
}

fn find_entry(
    dir: &FatDir<'_, File>,
    name: &str,
) -> Result<Option<hadris_fat::sync::FileEntry>, Box<dyn std::error::Error>> {
    for entry in dir.entries() {
        let DirectoryEntry::Entry(entry) = entry?;
        if entry.name().eq_ignore_ascii_case(name) {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

/// Sets `ATTR_EXEC` on every file directly under `EXECUTABLE_DIR`, if that directory exists in
/// this image -- silently does nothing otherwise (e.g. the home disk, which has no programs).
fn mark_executable(vol: &FatVolume<File>) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(dir) = open_dir(vol, Path::new(EXECUTABLE_DIR)) else {
        return Ok(());
    };
    let entries: Vec<_> = dir
        .entries()
        .filter_map(|r| match r {
            Ok(DirectoryEntry::Entry(entry)) if entry.is_file() => Some(entry),
            _ => None,
        })
        .collect();
    for entry in entries {
        let new_attrs = DirEntryAttrFlags::from_bits_retain(entry.attributes().bits() | ATTR_EXEC);
        vol.set_attributes(&entry, new_attrs)?;
    }
    Ok(())
}
