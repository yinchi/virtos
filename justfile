default:
    just --list

# See Cargo.toml's `[target.'cfg(target_os = "none")'.dependencies]` comment for how the split
# between bare-metal and host-testable code works.
# Run the pure-logic modules' unit tests on the host, not the bare-metal target.
test-host:
    cargo test --target aarch64-unknown-linux-gnu

HOME_IMG := "home.img"

# Serial number for the system image (all zeros)
SYSTEM_ID := "00000000"

# The host triple, so `tools/mkimage` (a plain `std` build-time tool, not part of the kernel's
# no_std/bare-metal build) doesn't inherit `.cargo/config.toml`'s bare-metal default target --
# same idiom `test-host` uses, computed rather than hardcoded so it works on any machine.
HOST_TRIPLE := `rustc -vV | sed -n 's/^host: //p'`
MKIMAGE := "tools/mkimage/target/" + HOST_TRIPLE + "/release/mkimage"

# See plans/PORTING.md for why this replaces mtools (it can set this project's own
# executable-bit convention; mtools can't).
# Build the disk-image tool every future `*.img` is built through.
build-mkimage:
    cd tools/mkimage && cargo build --release --target {{HOST_TRIPLE}}

# Every crate in this repo besides the kernel (the root `Cargo.toml`) -- found rather than
# hardcoded, so a new `tools/*` or `user/*` crate is picked up with no justfile/prek.toml edit.
# Never Cargo workspace members of each other, or of the kernel (see plans/PORTING.md's "Repo
# organization"), so none of this is reachable from a plain `cargo fmt`/`clippy --all` at the root.
OTHER_CRATES := `find . -name Cargo.toml -not -path "./Cargo.toml" -not -path "*/target/*" -exec dirname {} \;`

# `--check` reports without rewriting; `fmt-fix` rewrites instead. `cargo fmt` doesn't care about
# target, so unlike `clippy` below there's no per-crate target override needed here.
# Format every crate in this repo.
fmt:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --all -- --check
    for crate in {{OTHER_CRATES}}; do
        (cd "$crate" && cargo fmt --all -- --check)
    done

# Same, but rewrites instead of just checking -- what `prek.toml`'s commit hook runs.
fmt-fix:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --all
    for crate in {{OTHER_CRATES}}; do
        (cd "$crate" && cargo fmt --all)
    done

# Unlike `fmt`, target matters here: a crate under `tools/` is host-side (see `build-mkimage`'s
# note) and needs its host triple explicitly; anything else (e.g. a future `user/*` crate) *wants*
# the kernel's bare-metal default, so gets no override. None of these can take `--all-targets`
# either: `#[cfg(test)]` code doesn't compile for the bare-metal target at all (no `test` crate
# there), which is exactly why host-side tests run via `test-host`'s own target instead.
# Clippy every crate in this repo.
clippy:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo clippy
    for crate in {{OTHER_CRATES}}; do
        target=""
        case "$crate" in
            ./tools/*) target="--target {{HOST_TRIPLE}}" ;;
        esac
        (cd "$crate" && cargo clippy $target)
    done

# Same, but fixes instead of just checking -- what `prek.toml`'s commit hook runs.
clippy-fix:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo clippy --fix --allow-dirty
    for crate in {{OTHER_CRATES}}; do
        target=""
        case "$crate" in
            ./tools/*) target="--target {{HOST_TRIPLE}}" ;;
        esac
        (cd "$crate" && cargo clippy $target --fix --allow-dirty)
    done

# Format + lint every crate in this repo.
lint: fmt clippy

# Build the OS as a release binary
build:
    cargo build --release && cp target/aarch64-unknown-none-softfloat/release/virtos virtos.elf

# Run the OS in QEMU
run: build system seed-home
    qemu-system-aarch64 -machine virt -cpu max -nodefaults \
    -serial mon:stdio \
    -device virtio-gpu-device \
    -device virtio-keyboard-device \
    -drive file={{HOME_IMG}},if=none,format=raw,id=home \
    -device virtio-blk-device,drive=home \
    -drive file=system.img,if=none,format=raw,id=system \
    -device virtio-blk-device,drive=system \
    -kernel virtos.elf

# Seed the home disk from the host system
seed-home: build-mkimage
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{justfile_directory()}}  # The directory containing this justfile
    if [ -e {{HOME_IMG}} ]; then
        echo "{{HOME_IMG}} already exists, skipping seed."
        exit 0
    fi
    # FAT serial number for the home disk
    id=$(head -c 4 /dev/urandom | od -An -tx1 | tr -d ' \n')
    {{MKIMAGE}} {{HOME_IMG}} 64M HOME "$id" disks/home_seed
    echo "Created {{HOME_IMG}} with serial $id."

# Rebuild the home disk
reseed-home:
    mv {{HOME_IMG}} {{HOME_IMG}}.bak
    just seed-home

# TODO: system recipe to depend on building user programs and copying them into disks/system/usr/bin

# Build the system image
system: build-mkimage
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{justfile_directory()}}  # The directory containing this justfile
    mkdir -p disks/system/root  # Mount point for the HOME disk (/root)
    mkdir -p disks/system/usr/bin  # TODO: copy user programs into this directory
    {{MKIMAGE}} system.img 64M SYSTEM {{SYSTEM_ID}} disks/system
    echo "Created system.img with serial {{SYSTEM_ID}}."
