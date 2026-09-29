//! Plans where a new program's argument and environment strings and their pointer arrays go on
//! its initial stack. Pure memory layout calculation -- doesn't perform any actual writes to the
//! stack (`process.rs`'s `push_strings` does that). Carries `envp` layout even though nothing yet
//! produces real environment variables, since the mechanism is symmetric with `argv` and every
//! caller can simply pass an empty environment until one does (see `plans/USERSPACE_PLAN.md`).
//!
//! Working *downward* from where the stack pointer currently is (the direction the stack grows):
//!
//! 1. Compute the space needed for all the strings -- the arguments, then the environment --
//!    including their NUL terminators.
//! 2. Allocate space for the pointer arrays, one block, 16-byte aligned: `argv[]` and a `NULL`,
//!    then `envp[]` and a `NULL`, so `envp` sits right after `argv`'s terminator as in the Linux
//!    layout.
//! 3. Return the planned addresses for the strings and the arrays.
//!
//! If the planned addresses would fall below `floor` or wrap around, `None` is returned.

use alloc::vec::Vec;

/// The addresses to write the argument and environment strings and their pointer arrays at.
#[derive(Debug, PartialEq, Eq)]
pub struct ArgsPlan {
    /// The address of each argument string, in argument order (the first is the highest).
    pub args: Vec<usize>,
    /// The address of each environment string, in order, all below the arguments.
    pub env: Vec<usize>,
    /// The address of `argv[]`: each element points to one of the argument strings, except the
    /// last, a `NULL` terminator. Also the initial stack pointer.
    pub argv: usize,
    /// The address of `envp[]`, laid out the same way, immediately after `argv[]`'s `NULL`.
    pub envp: usize,
}

/// Plans memory layout for argument and environment strings and their pointer arrays.
///
/// * `sp` - The current stack pointer, from which to start allocating downward.
/// * `floor` - The lowest permissible address for the planned layout.
/// * `arg_lens`, `env_lens` - The length of each argument string / each environment string.
///
/// `Some(ArgsPlan)` if the layout fits above `floor` without wrapping, `None` otherwise.
pub fn plan(sp: usize, floor: usize, arg_lens: &[usize], env_lens: &[usize]) -> Option<ArgsPlan> {
    let mut cursor = sp;
    let mut place = |lens: &[usize]| -> Option<Vec<usize>> {
        let mut addrs = Vec::with_capacity(lens.len());
        for &len in lens {
            cursor = cursor.checked_sub(len.checked_add(1)?)?; // +1 for the NUL terminator
            addrs.push(cursor);
        }
        Some(addrs)
    };
    let args = place(arg_lens)?;
    let env = place(env_lens)?;

    let word = core::mem::size_of::<usize>();
    let argv_bytes = arg_lens.len().checked_add(1)?.checked_mul(word)?;
    let envp_bytes = env_lens.len().checked_add(1)?.checked_mul(word)?;
    let argv = cursor.checked_sub(argv_bytes.checked_add(envp_bytes)?)? & !0xf;
    (argv >= floor).then_some(ArgsPlan {
        args,
        env,
        argv,
        envp: argv + argv_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOP: usize = 0x4420_0000;
    const FLOOR: usize = TOP - 0x1_0000;

    #[test]
    fn nothing_at_all_is_two_null_slots() {
        let p = plan(TOP, FLOOR, &[], &[]).unwrap();
        assert!(p.args.is_empty() && p.env.is_empty());
        assert_eq!(p.argv, (TOP - 16) & !0xf);
        assert_eq!(p.envp, p.argv + 8);
    }

    #[test]
    fn strings_descend_in_order_and_do_not_overlap() {
        let (a, e) = ([4, 0, 7, 1], [3, 9]);
        let p = plan(TOP, FLOOR, &a, &e).unwrap();
        assert_eq!(p.args[0], TOP - 5);
        for (i, len) in a.iter().enumerate().skip(1) {
            assert_eq!(p.args[i], p.args[i - 1] - (len + 1));
        }
        // The environment strings continue below the arguments.
        assert_eq!(p.env[0], p.args[3] - 4);
        assert_eq!(p.env[1], p.env[0] - 10);
        // Both pointer arrays, with their terminators, sit below every string.
        assert!(p.argv + 8 * (a.len() + 1 + e.len() + 1) <= p.env[1]);
    }

    #[test]
    fn envp_follows_argvs_null_slot() {
        let p = plan(TOP, FLOOR, &[1, 1, 1], &[2, 2]).unwrap();
        assert_eq!(p.envp, p.argv + 8 * 4);
    }

    #[test]
    fn the_stack_pointer_is_sixteen_byte_aligned_whatever_the_lengths() {
        for a in 0..40 {
            for n in 0..6 {
                for m in 0..4 {
                    let p =
                        plan(TOP - a, FLOOR, &alloc::vec![a; n], &alloc::vec![a + 1; m]).unwrap();
                    assert_eq!(p.argv % 16, 0, "a={a} n={n} m={m}");
                }
            }
        }
    }

    #[test]
    fn refuses_what_does_not_fit_or_would_wrap() {
        assert!(plan(TOP, FLOOR, &[0x8000, 0x8000], &[]).is_none());
        assert!(plan(TOP, FLOOR, &[0x1_0000 - 16], &[]).is_none());
        assert!(plan(TOP, FLOOR, &[usize::MAX], &[]).is_none());
        assert!(plan(TOP, FLOOR, &[usize::MAX - 1], &[]).is_none());
        assert!(plan(TOP, FLOOR, &[], &[usize::MAX]).is_none());
        assert!(plan(4, 0, &[8], &[]).is_none());
        assert!(plan(0x40, 0, &[0x100], &[]).is_none());
        // Exactly fitting is fine: nothing between the arrays and the floor.
        let p = plan(TOP, FLOOR, &[], &[]).unwrap();
        assert!(plan(TOP, p.argv, &[], &[]).is_some());
        assert!(plan(TOP, p.argv + 16, &[], &[]).is_none());
    }

    #[test]
    fn the_limit_counts_arguments_and_environment_together() {
        // Each half fits alone; together they do not.
        let half = 0x9000;
        assert!(plan(TOP, FLOOR, &[half], &[]).is_some());
        assert!(plan(TOP, FLOOR, &[], &[half]).is_some());
        assert!(plan(TOP, FLOOR, &[half], &[half]).is_none());
    }

    #[test]
    fn thirty_arguments_and_thirty_variables() {
        let lens = alloc::vec![2; 30];
        let p = plan(TOP, FLOOR, &lens, &lens).unwrap();
        assert_eq!((p.args.len(), p.env.len()), (30, 30));
        assert_eq!(p.argv % 16, 0);
    }
}
