//! How `mount` names a mounted volume when it lists it: in the spelling `mount` itself takes as a source, so a line can
//! be copied back to a command line. There are no device names (`/dev/vdb`) to show -- nothing can be done with one --
//! so a volume is `LABEL=name`, or `UUID=XXXX-XXXX` when it has no label or another volume has the same one (a label
//! that does not pick one volume is not a name for it). Pure, so it is tested on the host.

use alloc::format;
use alloc::string::String;

/// The source spelling for a volume with `label` (empty if it has none) and volume ID `id`; `label_is_shared` says
/// another device has the same label.
pub fn source_of(label: &str, id: u32, label_is_shared: bool) -> String {
    if label.is_empty() || label_is_shared {
        format!("UUID={:04X}-{:04X}", id >> 16, id & 0xffff)
    } else {
        format!("LABEL={label}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unique_label_is_the_name() {
        assert_eq!(source_of("SYSTEM", 0, false), "LABEL=SYSTEM");
    }

    #[test]
    fn a_shared_or_missing_label_falls_back_to_the_id() {
        assert_eq!(source_of("HOME", 0x5e6f_7a8b, true), "UUID=5E6F-7A8B");
        assert_eq!(source_of("", 0x0000_00ab, false), "UUID=0000-00AB");
    }
}
