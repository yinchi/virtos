/* AArch64 exception vector table: 16 entries, 0x80 bytes apart, 0x800 total. */

.macro ventry label
    .align 7
    b \label
.endm

.macro kernel_entry
    /* Save all general-purpose registers, ELR_EL1, and SPSR_EL1.  Called for all exceptions
     * at EL1. Every exception ends in one of the following ways:
     * - `kernel_exit` to restore all registers and return from the exception (eret)
     * - `resume_kernel` from context.s, for terminating a user process
     * - `unexpected_exception`, which hangs the system
     */

    sub sp, sp, #272 /* Enough to save all registers involved */
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16 * 1]
    stp x4, x5, [sp, #16 * 2]
    stp x6, x7, [sp, #16 * 3]
    stp x8, x9, [sp, #16 * 4]
    stp x10, x11, [sp, #16 * 5]
    stp x12, x13, [sp, #16 * 6]
    stp x14, x15, [sp, #16 * 7]
    stp x16, x17, [sp, #16 * 8]
    stp x18, x19, [sp, #16 * 9]
    stp x20, x21, [sp, #16 * 10]
    stp x22, x23, [sp, #16 * 11]
    stp x24, x25, [sp, #16 * 12]
    stp x26, x27, [sp, #16 * 13]
    stp x28, x29, [sp, #16 * 14]

    /* Copy ELR_EL1, and SPSR_EL1 into general-purpose registers */
    mrs x21, elr_el1
    mrs x22, spsr_el1

    /* Save the remaining registers */
    stp x21, x22, [sp, #16 * 15]
    str x30, [sp, #16 * 16]
.endm
