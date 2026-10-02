//! Block devices: `blkinfo`, what `lsblk` lists.

use abi::blk::{BLKINFO_SIZE, BlkInfo};

/// Describes block device `index` (0-based, in the order the kernel found them): `Err(ENODEV)`
/// past the last one, so a caller counts the devices by asking until it gets that.
pub fn blkinfo(index: usize) -> Result<BlkInfo, isize> {
    let mut out = [0u8; BLKINFO_SIZE];
    let result = syscall!(abi::SYS_BLKINFO, index, out.as_mut_ptr() as usize);
    if result < 0 {
        return Err(result);
    }
    Ok(BlkInfo::decode(&out))
}
