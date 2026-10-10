default:
    just --list

# Everything: the host unit tests, then the end-to-end QEMU tests (see test/README.md).
test: test-host test-qemu

# See Cargo.toml's `[target.'cfg(target_os = "none")'.dependencies]` comment for how the split
# between bare-metal and host-testable code works.
# Run the pure-logic modules' unit tests on the host, not the bare-metal target.
test-host:
    cargo test --target aarch64-unknown-linux-gnu
    cd user/abi && cargo test --target aarch64-unknown-linux-gnu
    cd user/textlib && cargo test --target aarch64-unknown-linux-gnu
    cd user/editlib && cargo test --target aarch64-unknown-linux-gnu

# Boots the kernel headless in QEMU, one group (one QEMU) per module of test/cases/, in parallel,
# after rebuilding the kernel and the system image (see test/README.md).
# Run the end-to-end QEMU tests: all, or the named modules (`just test-qemu shell launch`).
test-qemu *MODULES: build system build-mkimage build-test-progs
    python3 test/run_tests.py --kernel virtos.elf --system system.img --mkimage {{MKIMAGE}} --test-progs test/progs/target/aarch64-unknown-none-softfloat/release {{MODULES}}

# The EL0 programs only the tests use (`test/progs`): bad system calls and deliberate faults. A
# module that wants them lists them in `TEST_PROGS`; they are put on its private copy of the
# system image, never on the shipped one.
# Build the test-only programs.
build-test-progs:
    cd test/progs && cargo build --release

# The shortest end-to-end check: boot, an echo, a pipeline, a redirect.
test-smoke: build system build-mkimage
    python3 test/run_tests.py --kernel virtos.elf --system system.img --mkimage {{MKIMAGE}} smoke

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
    # `while read` rather than `for crate in` OTHER_CRATES directly: it contains newlines if there's
    # more than one crate; IFS parses the quoted variable and iterates over the crates correctly.
    while IFS= read -r crate; do
        [ -n "$crate" ] || continue
        (cd "$crate" && cargo fmt --all -- --check)
    done <<< "{{OTHER_CRATES}}"

# Same, but rewrites instead of just checking -- what `prek.toml`'s commit hook runs.
fmt-fix:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --all
    while IFS= read -r crate; do
        [ -n "$crate" ] || continue
        (cd "$crate" && cargo fmt --all)
    done <<< "{{OTHER_CRATES}}"

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
    while IFS= read -r crate; do
        [ -n "$crate" ] || continue
        target=""
        case "$crate" in
            ./tools/*) target="--target {{HOST_TRIPLE}}" ;;
        esac
        (cd "$crate" && cargo clippy $target)
    done <<< "{{OTHER_CRATES}}"

# Same, but fixes instead of just checking -- what `prek.toml`'s commit hook runs.
clippy-fix:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo clippy --fix --allow-dirty
    while IFS= read -r crate; do
        [ -n "$crate" ] || continue
        target=""
        case "$crate" in
            ./tools/*) target="--target {{HOST_TRIPLE}}" ;;
        esac
        (cd "$crate" && cargo clippy $target --fix --allow-dirty)
    done <<< "{{OTHER_CRATES}}"

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

# Starts by wiping disks/system/usr/bin/ rather than just adding to whatever's already there:
# otherwise a program whose src/bin/*.rs got deleted leaves its old binary behind forever, stale
# but still shipped in every future image, since nothing else would ever remove it. Each crate
# directory is built exactly once (a plain `cargo build --release`, no `--bin`, builds every binary
# target the crate has in one invocation -- both `src/main.rs` and any number of `src/bin/*.rs`),
# then every binary it produced is stripped in turn -- so a crate with several programs (e.g.
# `user/progs`) isn't rebuilt once per program. A crate with neither layout (`abi`/`userlib`: plain
# libraries, `src/lib.rs`, no `[[bin]]`) is skipped -- there's nothing to strip. `strip -o` rather
# than stripping in place: the unstripped binary stays in that crate's own `target/` for a debugger
# to use, only the copy actually shipped in the image is stripped. No justfile edit needed for a
# new crate or a new program within one.
# Builds every user/* crate's EL0 programs and strips each into disks/system/usr/bin/<name>.
build-user:
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf disks/system/usr/bin
    mkdir -p disks/system/usr/bin
    for crate in user/*/; do
        crate="${crate%/}"
        bins=()
        if [ -d "$crate/src/bin" ]; then
            for f in "$crate"/src/bin/*.rs; do
                [ -e "$f" ] || continue
                bins+=("$(basename "$f" .rs)")
            done
        fi
        if [ -f "$crate/src/main.rs" ]; then
            bins+=("$(basename "$crate")")
        fi
        [ ${#bins[@]} -gt 0 ] || continue
        (cd "$crate" && cargo build --release)
        for name in "${bins[@]}"; do
            aarch64-linux-gnu-strip -o disks/system/usr/bin/"$name" \
                "$crate/target/aarch64-unknown-none-softfloat/release/$name"
        done
    done

# Build the system image
system: build-mkimage build-user
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{justfile_directory()}}  # The directory containing this justfile
    mkdir -p disks/system/root  # Mount point for the HOME disk (/root)
    {{MKIMAGE}} system.img 64M SYSTEM {{SYSTEM_ID}} disks/system
    echo "Created system.img with serial {{SYSTEM_ID}}."
