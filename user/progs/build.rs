//! Nothing to build -- just tells cargo to relink when `link.ld` changes, since it's referenced
//! only via a linker flag (inherited from the repo root's `.cargo/config.toml`), which cargo
//! doesn't otherwise track as a rebuild trigger. Same reasoning as the kernel's own `build.rs`.

fn main() {
    println!("cargo:rerun-if-changed=link.ld");
}
