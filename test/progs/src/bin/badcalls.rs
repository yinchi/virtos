//! Makes one bad system call after another and prints `<name> <result>` for each: the result is
//! the negated errno the kernel answered with (`abi::errno`), which `test/cases/syscalls.py`
//! compares with what each call should get. Needs the working directory to be `/` (a relative
//! path is never used) and the HOME disk mounted at `/root` and a third volume, labelled SPARE, not mounted.
#![no_std]
#![no_main]

extern crate alloc;

use abi::{
    SYS_BLKINFO, SYS_CHMOD, SYS_CLOSE, SYS_GETCWD, SYS_GETDENTS, SYS_MKDIRAT, SYS_MOUNT,
    SYS_NEWFSTATAT, SYS_OPEN, SYS_READ, SYS_REBOOT, SYS_RENAMEAT, SYS_UMOUNT2, SYS_UNLINKAT,
    SYS_UTIMENSAT, SYS_WRITE,
};
use testprogs::{raw, report};
use userlib::ExitCode;

userlib::entry!(run);

const O_RDONLY: usize = 0;
const O_WRONLY: usize = 1;

/// An address below the program's memory (and not null).
const LOW: usize = 0x1000;
/// An address inside the kernel's memory.
const KERNEL: usize = 0x4000_0000;
/// An address so high that `ptr + len` wraps around.
const WRAP: usize = usize::MAX - 1;

fn path(call: usize, p: &str) -> isize {
    raw(call, p.as_ptr() as usize, p.len(), 0, 0)
}

fn open(p: &str, flags: usize) -> isize {
    raw(SYS_OPEN, p.as_ptr() as usize, p.len(), flags, 0)
}

fn run() -> ExitCode {
    // ---------------------------------------------------------------- no such call
    report("unknown-syscall", raw(9999, 0, 0, 0, 0));
    report("syscall-zero", raw(0, 0, 0, 0, 0));

    // ------------------------------------------------------- pointers outside the program
    report("write-low-pointer", raw(SYS_WRITE, 1, LOW, 4, 0));
    report("write-kernel-pointer", raw(SYS_WRITE, 1, KERNEL, 4, 0));
    report("write-wrapping-range", raw(SYS_WRITE, 1, WRAP, 16, 0));
    report("write-null-empty", raw(SYS_WRITE, 1, 0, 0, 0));
    report("read-low-pointer", raw(SYS_READ, 0, LOW, 4, 0));
    report("open-low-path", raw(SYS_OPEN, LOW, 4, O_RDONLY, 0));
    report("mkdir-kernel-path", raw(SYS_MKDIRAT, KERNEL, 4, 0, 0));
    report("unlink-low-path", raw(SYS_UNLINKAT, LOW, 4, 0, 0));
    report(
        "rename-low-new",
        raw(SYS_RENAMEAT, "/etc".as_ptr() as usize, 4, LOW, 4),
    );
    report("getcwd-low-buffer", raw(SYS_GETCWD, LOW, 16, 0, 0));
    report(
        "stat-low-output",
        raw(SYS_NEWFSTATAT, "/etc".as_ptr() as usize, 4, LOW, 0),
    );
    report(
        "mount-low-source",
        raw(SYS_MOUNT, LOW, 4, "/tmp".as_ptr() as usize, 4),
    );
    report("blkinfo-low-output", raw(SYS_BLKINFO, 0, LOW, 0, 0));
    report(
        "utimensat-low-times",
        raw(SYS_UTIMENSAT, "/etc".as_ptr() as usize, 4, LOW, 0),
    );

    // ------------------------------------------------------------------- bad descriptors
    let mut buf = [0u8; 512];
    let b = buf.as_mut_ptr() as usize;
    report("write-fd-not-open", raw(SYS_WRITE, 9, b, 4, 0));
    report("write-fd-out-of-range", raw(SYS_WRITE, 99, b, 4, 0));
    report("read-fd-not-open", raw(SYS_READ, 9, b, 4, 0));
    report("close-fd-not-open", raw(SYS_CLOSE, 9, 0, 0, 0));
    report("close-fd-out-of-range", raw(SYS_CLOSE, 1 << 40, 0, 0, 0));
    report("write-to-stdin", raw(SYS_WRITE, 0, b, 4, 0));
    report("read-from-stdout", raw(SYS_READ, 1, b, 4, 0));
    report("getdents-on-stdout", raw(SYS_GETDENTS, 1, b, buf.len(), 0));

    // -------------------------------------------------------------------------- open
    report("open-bad-flags", open("/etc/environment", 7));
    report(
        "open-not-utf8",
        raw(SYS_OPEN, [0xffu8, 0xfe].as_ptr() as usize, 2, O_RDONLY, 0),
    );
    report("open-missing", open("/nosuch", O_RDONLY));
    report("open-through-a-file", open("/etc/environment/x", O_RDONLY));
    report("open-write-missing-directory", open("/nodir/f", O_WRONLY));
    report("open-write-a-directory", open("/etc", O_WRONLY));

    let file = open("/etc/environment", O_RDONLY);
    let dir = open("/etc", O_RDONLY);
    report(
        "write-to-a-read-only-file",
        raw(SYS_WRITE, file as usize, b, 4, 0),
    );
    report(
        "getdents-on-a-file",
        raw(SYS_GETDENTS, file as usize, b, buf.len(), 0),
    );
    report("read-a-directory", raw(SYS_READ, dir as usize, b, 4, 0));
    report("write-a-directory", raw(SYS_WRITE, dir as usize, b, 4, 0));
    report(
        "getdents-buffer-too-small",
        raw(SYS_GETDENTS, dir as usize, b, 8, 0),
    );
    report("close-a-file", raw(SYS_CLOSE, file as usize, 0, 0, 0));
    report("close-it-twice", raw(SYS_CLOSE, file as usize, 0, 0, 0));
    report("close-a-directory", raw(SYS_CLOSE, dir as usize, 0, 0, 0));

    // Every slot above the standard three, then one too many.
    let mut opened = 0;
    let mut last = 0;
    for _ in 0..32 {
        last = open("/etc/environment", O_RDONLY);
        if last < 0 {
            break;
        }
        opened += 1;
    }
    report("open-until-full-count", opened);
    report("open-when-full", last);
    for fd in 3..32 {
        raw(SYS_CLOSE, fd, 0, 0, 0);
    }

    // ------------------------------------------------------------------ paths
    report("mkdir-existing", path(SYS_MKDIRAT, "/etc"));
    report("mkdir-root", path(SYS_MKDIRAT, "/"));
    report("mkdir-mount-point", path(SYS_MKDIRAT, "/root"));
    report("mkdir-missing-parent", path(SYS_MKDIRAT, "/nodir/x"));
    report("unlink-missing", path(SYS_UNLINKAT, "/nosuch"));
    report("unlink-root", path(SYS_UNLINKAT, "/"));
    report("unlink-mount-point", path(SYS_UNLINKAT, "/root"));
    report("unlink-non-empty-directory", path(SYS_UNLINKAT, "/etc"));
    let rename = |old: &str, new: &str| {
        raw(
            SYS_RENAMEAT,
            old.as_ptr() as usize,
            old.len(),
            new.as_ptr() as usize,
            new.len(),
        )
    };
    report("rename-missing", rename("/nosuch", "/x"));
    report("rename-into-itself", rename("/etc", "/etc/x"));
    report(
        "rename-across-volumes",
        rename("/etc/environment", "/root/x"),
    );
    report("rename-mount-point", rename("/root", "/x"));
    report(
        "rename-onto-mount-point",
        rename("/etc/environment", "/root"),
    );
    report("getcwd-too-small", raw(SYS_GETCWD, b, 0, 0, 0));
    report(
        "stat-missing",
        raw(SYS_NEWFSTATAT, "/nosuch".as_ptr() as usize, 7, b, 0),
    );
    report(
        "stat-through-a-file",
        raw(
            SYS_NEWFSTATAT,
            "/etc/environment/x".as_ptr() as usize,
            18,
            b,
            0,
        ),
    );

    // ------------------------------------------------------------ chmod, utimensat
    let chmod = |p: &str, set: usize, clear: usize| {
        raw(SYS_CHMOD, p.as_ptr() as usize, p.len(), set, clear)
    };
    report("chmod-directory-bit", chmod("/etc/environment", 0x10, 0));
    report("chmod-bit-clear-too", chmod("/etc/environment", 0, 0x08));
    report(
        "chmod-more-than-a-byte",
        chmod("/etc/environment", 0x100, 0),
    );
    report("chmod-the-root", chmod("/", 0x40, 0));
    report("chmod-missing", chmod("/nosuch", 0x40, 0));

    // Two `timespec`s (access, modify): seconds then nanoseconds each.
    let mut times = [0i64; 4];
    let t = times.as_ptr() as usize;
    let utimes = |p: &str, t: usize| raw(SYS_UTIMENSAT, p.as_ptr() as usize, p.len(), t, 0);
    report("utimensat-missing", utimes("/nosuch", 0));
    report("utimensat-the-root", utimes("/", 0));
    times = [1_700_000_000, 0, 1_700_000_000, 2_000_000_000];
    report(
        "utimensat-bad-nanoseconds",
        utimes("/etc/environment", times.as_ptr() as usize),
    );
    times = [1_700_000_000, 0, -1, 0];
    report(
        "utimensat-before-1980",
        utimes("/etc/environment", times.as_ptr() as usize),
    );
    times = [1_700_000_000, 0, 5_000_000_000, 0];
    report(
        "utimensat-after-2107",
        utimes("/etc/environment", times.as_ptr() as usize),
    );
    let _ = t;

    // -------------------------------------------------------------- mount, umount
    let mount = |src: &str, target: &str| {
        raw(
            SYS_MOUNT,
            src.as_ptr() as usize,
            src.len(),
            target.as_ptr() as usize,
            target.len(),
        )
    };
    report("mount-not-a-source", mount("garbage", "/tmp"));
    report("mount-no-such-volume", mount("LABEL=NOPE", "/tmp"));
    report("mount-already-mounted", mount("LABEL=SYSTEM", "/tmp"));
    report("mount-target-missing", mount("LABEL=SPARE", "/nodir"));
    report(
        "mount-target-a-file",
        mount("LABEL=SPARE", "/etc/environment"),
    );
    report("umount-not-a-mount-point", path(SYS_UMOUNT2, "/etc"));
    report("umount-the-root", path(SYS_UMOUNT2, "/"));
    report("umount-nothing", path(SYS_UMOUNT2, "/nosuch"));

    // ---------------------------------------------------------- blkinfo, reboot
    report("blkinfo-past-the-last", raw(SYS_BLKINFO, 99, b, 0, 0));
    report("reboot-unknown-command", raw(SYS_REBOOT, 0x1234, 0, 0, 0));
    report("reboot-wide-command", raw(SYS_REBOOT, 1 << 32, 0, 0, 0));
    ExitCode(0)
}
