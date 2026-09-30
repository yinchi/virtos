/* Load address must match the kernel's own `arch::mmu::USER_BASE`. `.text`/`.rodata` are read+exec
 * only; `.data`/`.bss` (now that `userlib`'s global allocator state needs some) get their own
 * page-aligned, read-write region after them -- the kernel's `mmu::enable` sets SCTLR_EL1.WXN, so
 * a single output section spanning both would come out non-executable in practice (the linker
 * unions its input sections' flags into one segment) even where only the `.text` part needs exec,
 * and `exec::elfparse` refuses two segments with different permissions sharing a page regardless,
 * hence the explicit `ALIGN`. */
ENTRY(_start)

SECTIONS
{
    . = 0x44000000;

    .text : { *(.text*) *(.rodata*) }
    . = ALIGN(0x1000);

    .data : { *(.data*) }
    .bss : { *(.bss*) }

    /* `.comment`: non-allocated metadata the linker would otherwise place on its own -- harmless
     * either way (not ALLOC, so never part of a PT_LOAD segment), dropped just to keep the binary
     * minimal. `.eh_frame*`: unwind-table data `-C force-unwind-tables=no` (`.cargo/config.toml`)
     * doesn't reliably suppress -- when present, the linker gives it its own leading read-only
     * `PT_LOAD` segment, which can share a page with the `.text` segment right after it and get
     * refused by `exec::elfparse`'s same-page/different-permissions check. Not needed anyway: no
     * unwinding happens in this `panic = "abort"` binary. `.shstrtab` (and other linker-internal
     * sections `*(*)` would also match) can't be discarded; name what's actually safe to drop
     * instead of a blanket wildcard. */
    /DISCARD/ : { *(.comment) *(.eh_frame*) }
}
