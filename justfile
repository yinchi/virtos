default:
    just --list

# Run the pure-logic modules' unit tests on the host, not the bare-metal target -- see
# Cargo.toml's `[target.'cfg(target_os = "none")'.dependencies]` comment for how the split works.
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

# Build the disk-image tool every future `*.img` is built through -- see plans/PORTING.md for why
# this replaces mtools (it can set this project's own executable-bit convention; mtools can't).
build-mkimage:
    cd tools/mkimage && cargo build --release --target {{HOST_TRIPLE}}

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
