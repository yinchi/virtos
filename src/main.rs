#![no_std]
#![no_main]

mod arch;

use crate::arch::gic::gic;
use crate::arch::init::{UART_BASE, init_devices};
use crate::arch::rtc::DEFAULT_TIMEZONE;
use crate::arch::rtc::rtc;
use arm_pl011_uart::{
    DataBits, LineConfig, PL011Registers, Parity, StopBits, Uart, UniqueMmioPointer,
};
use core::{fmt::Write, ptr::NonNull};
use linked_list_allocator::LockedHeap;

/// Size of the kernel heap. `aarch64-paging`'s `IdMap` allocates its page-table nodes from it, so
/// it must be initialized before `mmu::enable` runs.
const HEAP_SIZE: usize = 1 << 20; // 1 MiB
/// The heap's backing storage: a plain `.bss` static, so it's already covered by `mmu::enable`'s
/// `__data_start..__data_end` mapping -- nothing extra needs mapping for it.
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

fn get_uart<'a>() -> Uart<'a> {
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

    // Placeholder until we have a shell
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
