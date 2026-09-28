ENTRY(_start)

SECTIONS
{
    . = 0x40000000; /* Start of RAM for QEMU's `virt` machine */

    __text_start = .;
    .text : { *(.text.boot) *(.text*) }
    . = ALIGN(0x1000); /* 4096 */
    __text_end = .;

    __rodata_start = .;
    .rodata : { *(.rodata*) }

    /* Force placement of exception handling frames here so they don't get placed arbitrarily */
    .eh_frame_hdr : { *(.eh_frame_hdr) }
    .eh_frame : { *(.eh_frame) }
    . = ALIGN(0x1000); /* 4096 */
    __rodata_end = .;

    __data_start = .;
    .data : { *(.data*) }
    .bss : { *(.bss*) }
    . = ALIGN(0x1000); /* 4096 */
    __data_end = .;

    /* Stack */
    __stack_guard = .;
    . = . + 0x10000; /* Reserve 64KiB for the stack guard */
    __stack_bottom = .;
    . = . + 0x100000; /* Reserve 1MiB for the stack */
    __stack_top = .;
    __kernel_end = .;
}
