#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

// Everything below is bare-metal-only (touches hardware, or exists only to serve code that
// does); gated so `cargo test --target <host triple>` (`just test-host`) can build and run
// `console`/`keyboard`'s unit tests as ordinary host tests, without any of this -- see
// Cargo.toml's matching `[target.'cfg(target_os = "none")'.dependencies]`.
#[cfg(target_os = "none")]
mod arch;
#[cfg(target_os = "none")]
mod drivers;

// Pure logic, no hardware dependency: builds and runs under both targets.
mod console;
mod keyboard;

// Mixed: `fs::sector` is pure like the above, `fs::blkio` is hardware-only like `drivers` --
// see `fs/mod.rs`.
mod fs;

#[cfg(target_os = "none")]
mod bare_metal {
    use crate::arch::gic::gic;
    use crate::arch::init::{UART_BASE, init_devices};
    use crate::arch::rtc::DEFAULT_TIMEZONE;
    use crate::arch::rtc::rtc;
    use crate::arch::timer;
    use crate::console::{self, Console, DEFAULT_BG as BG, DEFAULT_FG as FG};
    use crate::drivers::virtio::{blk, gpu, input};
    use crate::fs;
    use crate::keyboard::KeyState;
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

        writeln!(uart, "{:?}", gic()).expect("Could not write to UART");

        writeln!(
            uart,
            "Current Time: {:?}",
            rtc().get_time().with_timezone(&DEFAULT_TIMEZONE)
        )
        .expect("Could not write to UART");

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

        // Temporary: confirm the console + keyboard pipeline works end to end -- real typed
        // characters, translated by `keyboard` and echoed to the framebuffer by `console`, not
        // just raw event codes to the UART or a solid test pattern. Replace with the next thing
        // to check once this is confirmed working.
        if let (Some(gpu_dev), Some(keyboard_dev)) = (gpu::get(), input::get()) {
            let fb = gpu_dev.framebuffer();
            let mut console = Console::new(console::Framebuffer {
                ptr: fb.ptr,
                width: fb.width,
                height: fb.height,
                stride: fb.stride,
            });
            console.clear(BG);
            console.puts("virtos> ", FG, BG);
            gpu_dev.flush();

            writeln!(uart, "console+keyboard: typing for 15s, press some keys...")
                .expect("Could not write to UART");
            let mut keys = KeyState::new();
            let deadline = timer::now() + timer::freq() * 15;
            while timer::now() < deadline {
                if let Some(event) = keyboard_dev.poll()
                    && let Some(ch) = keys.apply(event.code, event.value)
                {
                    console.putc(ch, FG, BG);
                    gpu_dev.flush();
                }
            }
            writeln!(uart, "console+keyboard: done").expect("Could not write to UART");
        } else {
            writeln!(uart, "No virtio-gpu or virtio-keyboard device found")
                .expect("Could not write to UART");
        }

        loop {
            unsafe { core::arch::asm!("wfe") }
        }
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
        panic!(
            "Unexpected exception: {}",
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
