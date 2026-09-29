/* Load address must match the kernel's own `arch::mmu::USER_BASE`. Read+exec only, deliberately no
 * `.data*`/`.bss*` glob here: the kernel's `mmu::enable` sets SCTLR_EL1.WXN, so any output section
 * that's ever both writable and executable comes out non-executable in practice (the linker unions
 * its input sections' flags into one segment) -- use the stack (already mapped read/write by
 * `exec::process::run_program`) for any writable buffer this program needs instead of statics. */
ENTRY(_start)

SECTIONS
{
    . = 0x44000000;

    .text : { *(.text*) *(.rodata*) }

    /* Non-allocated metadata the linker would otherwise place on its own -- harmless either way
     * (not ALLOC, so never part of a PT_LOAD segment), dropped just to keep the binary minimal.
     * `.shstrtab` (and other linker-internal sections `*(*)` would also match) can't be discarded;
     * name what's actually safe to drop instead of a blanket wildcard. */
    /DISCARD/ : { *(.comment) }
}
