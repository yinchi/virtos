default:
    just --list

# Run the pure-logic modules' unit tests on the host, not the bare-metal target -- see
# Cargo.toml's `[target.'cfg(target_os = "none")'.dependencies]` comment for how the split works.
test-host:
    cargo test --target aarch64-unknown-linux-gnu

HOME_IMG := "home.img"

# Serial number for the system image (all zeros)
SYSTEM_ID := "00000000"

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
seed-home:
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{justfile_directory()}}  # The directory containing this justfile
    if [ -e {{HOME_IMG}} ]; then
        echo "{{HOME_IMG}} already exists, skipping seed."
        exit 0
    fi
    # FAT serial number for the home disk
    id=$(head -c 4 /dev/urandom | od -An -tx1 | tr -d ' \n')
    TZ=UTC ./scripts/folder_to_img.sh {{HOME_IMG}} 64M HOME "$id" disks/home_seed
    echo "Created {{HOME_IMG}} with serial $id."

# Rebuild the home disk
reseed-home:
    mv {{HOME_IMG}} {{HOME_IMG}}.bak
    just seed-home

# TODO: system recipe to depend on building user programs and copying them into disks/usr/bin

# Build the system image
system:
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{justfile_directory()}}  # The directory containing this justfile
    mkdir -p disks/root  # Mount point for the HOME disk (/root)
    mkdir -p disks/usr/bin  # TODO: copy user programs into this directory
    TZ=UTC ./scripts/folder_to_img.sh system.img 64M SYSTEM {{SYSTEM_ID}} disks/system
    echo "Created system.img with serial {{SYSTEM_ID}}."
