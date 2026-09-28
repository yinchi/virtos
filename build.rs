//! Assembles every .s/.S file under src/ (they live in src/arch/) and links the result into
//! `libboot_asm.a`.

use std::path::Path;

/// Adds every `.s`/`.S` file under `dir`, recursively, to `build`; returns whether it found any.
fn add_asm(dir: &Path, build: &mut cc::Build) -> bool {
    // Flag indicating whether any assembly files were found
    let mut any = false;

    // Iterate over the entries in the directory
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("failed to read {dir:?}: {e}")) {
        let path = entry.expect("failed to read dir entry").path();
        if path.is_dir() {
            // Recurse into subdirectory
            any |= add_asm(&path, build);
        } else if matches!(path.extension().and_then(|e| e.to_str()), Some("s" | "S")) {
            // Found an assembly file: add it to the build
            println!("cargo:rerun-if-changed={}", path.display());
            build.file(&path);
            any = true;
        }
    }
    any
}

fn main() {
    // Skip entirely when building for a host test target (`cargo test --target <host triple>`,
    // see Cargo.toml's `[target.'cfg(target_os = "none")'.dependencies]` comment): this assembly
    // defines the bare-metal entry point (`_start`) and friends, which would collide with the
    // host OS's own C runtime if linked into a normal executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }

    println!("cargo:rerun-if-changed=link.ld");
    println!("cargo:rerun-if-changed=src");
    let mut build = cc::Build::new();

    // Recursively add all assembly files under `src` to the build
    // All assembly files found will be compiled into `boot_asm`
    if add_asm(Path::new("src"), &mut build) {
        build.compile("boot_asm");
    }
}
