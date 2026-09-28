#![no_std]
#![no_main]

mod arch;

use crate::arch::gic::gic;
use crate::arch::init::{UART_BASE, init_devices};
use crate::arch::rtc::rtc;
use arm_pl011_uart::{
    DataBits, LineConfig, PL011Registers, Parity, StopBits, Uart, UniqueMmioPointer,
};
use core::{fmt::Write, ptr::NonNull};
use linked_list_allocator::LockedHeap;

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
    let mut uart = get_uart();
    writeln!(uart, "\x1b[2JHello Uart!").expect("Could not write to UART");

    // boot.s ensures that the DTB pointer is in the correct register before jumping to the kernel
    // (x0, which goes to argument `dtb_ptr`)
    init_devices(dtb_ptr, &mut uart);

    writeln!(uart, "{:?}", gic().lock()).expect("Could not write to UART");

    writeln!(
        uart,
        "Current RTC Timestamp: {:?}",
        rtc().get_unix_timestamp()
    )
    .expect("Could not write to UART");

    // Placeholder until we have a shell
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}

#[unsafe(no_mangle)]
extern "C" fn unexpected_exception(_: usize) -> ! {
    panic!("Unexpected exception occurred!");
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
