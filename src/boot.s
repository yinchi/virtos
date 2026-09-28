.section ".text.boot"
.global _start

_start:
    /* Set sp to use the EL1 stack */
    msr SPSel, #1
    isb

    /* Initialize the stack pointer for EL1 */
    ldr x0, =__stack_top
    mov sp, x0

    /* QEMU only passes the DTB address in x0 when booting a raw/Image kernel via the Linux boot
     * protocol; for an ELF kernel (bare-metal boot) x0 is left at 0. Since our kernel occupies
     * RAM start (0x40000000), QEMU's auto-generated DTB falls back to flash at 0x0 instead
     * (confirmed empirically via a physical memory dump). */
    mov x0, #0

    /* Jump to the kernel entry point */
    bl kernel_main

/* If kernel_main returns, hang the CPU */
hang:
    wfe
    b hang
