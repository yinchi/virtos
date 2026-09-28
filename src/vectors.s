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

.macro kernel_exit
    /* Exact reverse of kernel_entry: restore ELR_EL1/SPSR_EL1 and x30 first (they're only ever
     * read back into x21/x22/x30, so it's safe to overwrite x21/x22 again below with their real
     * saved values), then every general-purpose register, then drop the frame and return. */

    ldr x30, [sp, #16 * 16]
    ldp x21, x22, [sp, #16 * 15]
    msr elr_el1, x21
    msr spsr_el1, x22

    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16 * 1]
    ldp x4, x5, [sp, #16 * 2]
    ldp x6, x7, [sp, #16 * 3]
    ldp x8, x9, [sp, #16 * 4]
    ldp x10, x11, [sp, #16 * 5]
    ldp x12, x13, [sp, #16 * 6]
    ldp x14, x15, [sp, #16 * 7]
    ldp x16, x17, [sp, #16 * 8]
    ldp x18, x19, [sp, #16 * 9]
    ldp x20, x21, [sp, #16 * 10]
    ldp x22, x23, [sp, #16 * 11]
    ldp x24, x25, [sp, #16 * 12]
    ldp x26, x27, [sp, #16 * 13]
    ldp x28, x29, [sp, #16 * 14]

    add sp, sp, #272
    eret
.endm

/* The exception vector table itself: 16 entries, aligned to 0x800 (2^11) as a whole, each entry
 * aligned to 0x80 (2^7) by `ventry`. Row = where the exception came from, column = exception
 * type -- see the AArch64 exception model. Only `irq_el1h` (a device interrupt while the kernel
 * itself runs, the only case reachable so far: nothing runs at EL0 yet) has a real handler;
 * every other entry still traps to `unexpected_exception` so a bug is visible immediately
 * instead of silently doing the wrong thing. `x0` carries the vector's index into that panic. */
.align 11
.global vectors
vectors:
    ventry sync_el1t
    ventry irq_el1t
    ventry fiq_el1t
    ventry error_el1t

    ventry sync_el1h
    ventry irq_el1h
    ventry fiq_el1h
    ventry error_el1h

    ventry sync_el0_64
    ventry irq_el0_64
    ventry fiq_el0_64
    ventry error_el0_64

    ventry sync_el0_32
    ventry irq_el0_32
    ventry fiq_el0_32
    ventry error_el0_32

sync_el1t:
    kernel_entry
    mov x0, #0
    bl unexpected_exception
irq_el1t:
    kernel_entry
    mov x0, #1
    bl unexpected_exception
fiq_el1t:
    kernel_entry
    mov x0, #2
    bl unexpected_exception
error_el1t:
    kernel_entry
    mov x0, #3
    bl unexpected_exception

sync_el1h:
    kernel_entry
    mov x0, #4
    bl unexpected_exception
irq_el1h:
    kernel_entry
    bl irq_handler
    kernel_exit
fiq_el1h:
    kernel_entry
    mov x0, #6
    bl unexpected_exception
error_el1h:
    kernel_entry
    mov x0, #7
    bl unexpected_exception

sync_el0_64:
    kernel_entry
    mov x0, #8
    bl unexpected_exception
irq_el0_64:
    kernel_entry
    mov x0, #9
    bl unexpected_exception
fiq_el0_64:
    kernel_entry
    mov x0, #10
    bl unexpected_exception
error_el0_64:
    kernel_entry
    mov x0, #11
    bl unexpected_exception

sync_el0_32:
    kernel_entry
    mov x0, #12
    bl unexpected_exception
irq_el0_32:
    kernel_entry
    mov x0, #13
    bl unexpected_exception
fiq_el0_32:
    kernel_entry
    mov x0, #14
    bl unexpected_exception
error_el0_32:
    kernel_entry
    mov x0, #15
    bl unexpected_exception
