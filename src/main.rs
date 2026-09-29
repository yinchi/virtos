#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

// `keyboard::queue` and `exec::elfparse` hold data in `alloc` collections (`VecDeque`, `Vec`);
// `alloc` isn't in the prelude the way `std` is, so it needs its own `extern crate` to be
// reachable -- ungated (unlike the hardware-only modules below) since `exec::elfparse` is pure and
// needs it under the host test target too.
extern crate alloc;

// Everything below is bare-metal-only (touches hardware, or exists only to serve code that
// does); gated so `cargo test --target <host triple>` (`just test-host`) can build and run
// `console`/`keyboard`'s unit tests as ordinary host tests, without any of this -- see
// Cargo.toml's matching `[target.'cfg(target_os = "none")'.dependencies]`.
#[cfg(target_os = "none")]
mod arch;
#[cfg(target_os = "none")]
mod drivers;
#[cfg(target_os = "none")]
mod shell;
#[cfg(target_os = "none")]
mod syscall;

// Pure logic, no hardware dependency: builds and runs under both targets.
mod console;
mod keyboard;

// Mixed: each has a pure part (`fs::sector`, `exec::elfparse`) and a hardware-only part
// (`fs::blkio`, `exec::elf`/`exec::process`) -- see their own `mod.rs` for the split.
mod exec;
mod fs;

// `pub(crate)` (not private) so `syscall::sync_el0_handler`'s fault path can grab a UART via
// `bare_metal::get_uart` too, the same way `panic` and `unexpected_exception` below do.
#[cfg(target_os = "none")]
pub(crate) mod bare_metal {
    use crate::arch::gic::gic;
    use crate::arch::init::{UART_BASE, init_devices};
    use crate::arch::rtc::DEFAULT_TIMEZONE;
    use crate::arch::rtc::rtc;
    use crate::console::{self, Console, DEFAULT_BG as BG};
    use crate::drivers::virtio::{blk, gpu};
    use crate::fs;
    use crate::shell;
    use arm_pl011_uart::{
        DataBits, LineConfig, PL011Registers, Parity, StopBits, Uart, UniqueMmioPointer,
    };
    use core::{fmt::Write, ptr::NonNull};
    use hadris_fat::FatVolumeReadExt;
    use hadris_fat::sync::DirectoryEntry;
    use linked_list_allocator::LockedHeap;

    /// Size of the kernel heap. `aarch64-paging`'s `IdMap` allocates its page-table nodes from it, so
    /// it must be initialized before `mmu::enable` runs.
    const HEAP_SIZE: usize = 1 << 20; // 1 MiB
    /// The heap's backing storage: a plain `.bss` static, so it's already covered by `mmu::enable`'s
    /// `__data_start..__data_end` mapping -- nothing extra needs mapping for it.
    static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

    #[global_allocator]
    static ALLOCATOR: LockedHeap = LockedHeap::empty();

    pub fn get_uart<'a>() -> Uart<'a> {
        let uart_regs = unsafe { &mut *(UART_BASE as *mut PL011Registers) };
        let uart_pointer = unsafe { UniqueMmioPointer::new(NonNull::new(uart_regs).unwrap()) };
        let mut uart = Uart::new(uart_pointer);
        let line_config = LineConfig {
            data_bits: DataBits::Bits8,
            stop_bits: StopBits::One,
            parity: Parity::None,
        };
        uart.enable(line_config, 115200, 16_000_000)
            .expect("Could not enable UART");
        uart
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn kernel_main(dtb_ptr: usize) -> ! {
        // SAFETY: the only call to `init`, and it happens before anything else can possibly allocate.
        unsafe { ALLOCATOR.lock().init(&raw mut HEAP as *mut u8, HEAP_SIZE) };

        let mut uart = get_uart();
        writeln!(uart, "\x1b[2JHello Uart!").expect("Could not write to UART");

        // boot.s ensures that the DTB pointer is in the correct register before jumping to the kernel
        // (x0, which goes to argument `dtb_ptr`)
        let hardening = init_devices(dtb_ptr, &mut uart);
        writeln!(uart, "MMU enabled (PAN: {})", hardening.pan).expect("Could not write to UART");

        // Every interrupt source `init_devices` might enable (currently just the keyboard, via
        // `discover`) is already configured at this point -- safe to start taking IRQs.
        crate::arch::irq::unmask();

        writeln!(uart, "{:?}", gic()).expect("Could not write to UART");

        writeln!(
            uart,
            "Current Time: {:?}",
            rtc().get_time().with_timezone(&DEFAULT_TIMEZONE)
        )
        .expect("Could not write to UART");

        // The global console (`console::set`/`get`): `syscall::fd::write`'s fd 1/2 needs to reach
        // it, and the interactive test below reuses the same instance -- set up once, as soon as
        // a GPU device exists, rather than each caller constructing its own.
        if let Some(gpu_dev) = gpu::get() {
            let fb = gpu_dev.framebuffer();
            console::set(Console::new(console::Framebuffer {
                ptr: fb.ptr,
                width: fb.width,
                height: fb.height,
                stride: fb.stride,
            }));
            console::get().expect("just set").clear(BG);
            gpu_dev.flush();
        }

        // Temporary: confirm mounting a FAT volume through `BlkIo` actually works -- list the root
        // directory and read back a file's content, not just raw sector bytes off the device.
        // Both devices, since blk0 (system.img) currently holds only empty directories and blk1
        // (home.img) is what actually exercises the file-content read path. Replace with the next
        // thing to check once this is confirmed working.
        writeln!(uart, "virtio-blk devices found: {}", blk::count())
            .expect("Could not write to UART");
        for dev in 0..blk::count() {
            fs::volume::open(dev);
            let vol = fs::volume::get(dev).expect("just opened");
            let root = vol.root_dir();
            writeln!(uart, "blk{dev} root directory:").expect("Could not write to UART");
            for entry in root.entries() {
                let DirectoryEntry::Entry(entry) = entry.expect("directory entry read failed");
                writeln!(uart, "  {} ({} bytes)", entry.name(), entry.len())
                    .expect("Could not write to UART");
                if entry.is_file() {
                    let mut reader = vol
                        .read_file(&entry)
                        .expect("failed to open file for reading");
                    let mut buf = [0u8; 256];
                    let n = reader.read(&mut buf).expect("file read failed");
                    writeln!(
                        uart,
                        "    {:?}",
                        core::str::from_utf8(&buf[..n]).unwrap_or("<non-UTF-8>")
                    )
                    .expect("Could not write to UART");
                }
            }
        }

        // `run_forever` never returns: a real program is launched by typing its name here, rather
        // than `kernel_main` hardcoding which ones run at boot -- see `plans/USERSPACE_PLAN.md`'s
        // step 6. Supersedes both the old hardcoded `hello`/`scratch` boot-run and the fixed-15s
        // typing smoke test that came before it; `hello`/`scratch` are still verified, just by
        // typing their names at the prompt instead of the kernel doing it unconditionally.
        shell::run_forever(&mut uart);
    }

    /// One name per entry in `vectors.s`'s exception vector table, in the same order -- `vectors.s`
    /// passes the index of whichever entry it branched through as `vector`.
    const VECTOR_NAMES: [&str; 16] = [
        "sync_el1t",
        "irq_el1t",
        "fiq_el1t",
        "error_el1t",
        "sync_el1h",
        "irq_el1h",
        "fiq_el1h",
        "error_el1h",
        "sync_el0_64",
        "irq_el0_64",
        "fiq_el0_64",
        "error_el0_64",
        "sync_el0_32",
        "irq_el0_32",
        "fiq_el0_32",
        "error_el0_32",
    ];

    #[unsafe(no_mangle)]
    extern "C" fn unexpected_exception(vector: usize) -> ! {
        let esr = arm_sysregs::el1::accessors::read_esr_el1();
        let far = arm_sysregs::el1::accessors::read_far_el1();
        let elr = arm_sysregs::el1::accessors::read_elr_el1();
        panic!(
            "Unexpected exception: {} (ESR_EL1 {esr:?}, FAR_EL1 {far:#x}, ELR_EL1 {elr:#x})",
            VECTOR_NAMES.get(vector).unwrap_or(&"<unknown vector>")
        );
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        let mut uart = get_uart();
        writeln!(uart, "\x1b[31mPanic occurred!\x1b[0m").unwrap_or_default();
        writeln!(uart, "\x1b[31m{:?}\x1b[0m", info.message()).unwrap_or_default();
        loop {
            unsafe { core::arch::asm!("wfe") }
        }
    }
}
