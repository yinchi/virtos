.section ".text.boot"
.global _start

/* Every EL0 binary built against userlib shares this entry point: the kernel's loader `eret`s
straight into it, with `argc`/`argv` already in `x0`/`x1` (per the standard AArch64 calling
convention) and `SP_EL0` already set to the top of whatever `exec::process::run_program` wrote
there. No stack reservation is needed here -- the kernel owns the whole user window's layout and
decides where the stack lives independently of this binary's own sections.

`bl main` forwards x0/x1 unchanged, since nothing above it touches those registers -- whether
`main` actually reads them as argc/argv depends only on which of `userlib::entry!`/
`entry_with_args!` (see lib.rs) generated this crate's `main`. */
_start:
	bl	main

	/* `main` is declared `-> !` and must never return; reaching here means it violated that
	contract (a bug -- unsafe code or a UB path, since well-typed safe Rust can't fall off the end
	of a `-> !` function). Rather than fall into whatever memory follows silently, fall back to
	`exit` with a sentinel code so the kernel still regains control and the failure is at least
	observable. Tail call (`b`, not `bl`): `exit` itself never returns, so there is no return
	address worth preserving. */
	mov	w0, #255
	b	exit
