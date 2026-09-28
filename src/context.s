/* Defines context switching routines.
 *
 * Currently, only one user program runs at a time, so context switching is simplified.
 */

.section .bss

.align 3 /* Align to 8-byte boundary */

/* Reserve space for the context structure */
.global KERNEL_CTX
KERNEL_CTX:
    .skip 14 * 8 /* Reserve space for 14 8-byte registers */


.section .text

.global enter_el0
enter_el0:
    /* Save the current context and switch to EL0 */

    mov x9, sp /* Save the current stack pointer to x9 */
    adr x10, 1f /* label `1` for the return address */

    ldr x11, =KERNEL_CTX /* Load the address of the kernel context into x11 */
    stp x9, x10, [x11] /* Store the stack pointer and return address into the kernel context */

    /* Kernel context includes x19 to x30 (x30 is the return address) */
    stp x19, x20, [x11, #16]
    stp x21, x22, [x11, #32]
    stp x23, x24, [x11, #48]
    stp x25, x26, [x11, #64]
    stp x27, x28, [x11, #80]
    stp x29, x30, [x11, #96]

    eret

1:
    ret /* Return to the address stored in x30 (restored by `resume_kernel`) */

.global resume_kernel
resume_kernel:
    /* Restore the kernel context and return to the saved address */

    ldr x11, =KERNEL_CTX /* Load the address of the kernel context into x11 */
    ldp x9, x10, [x11] /* Load the saved stack pointer and return address */
    mov sp, x9 /* Restore the stack pointer */

    ldp x19, x20, [x11, #16]
    ldp x21, x22, [x11, #32]
    ldp x23, x24, [x11, #48]
    ldp x25, x26, [x11, #64]
    ldp x27, x28, [x11, #80]
    ldp x29, x30, [x11, #96]

    br x10 /* Branch to the return address stored in x10, i.e. label `1` in `enter_el0` */
