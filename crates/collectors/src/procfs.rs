//! Shared `/proc` helpers.

use crate::rootfs::FsError;
use crate::rootfs::RootFs;
use crate::util::{clean, read_warning};

/// Most `/proc` entries listed (pids plus the few non-numeric names).
const MAX_PROC_ENTRIES: usize = 200_000;

/// Numeric process ids found in `/proc`, ascending. Problems are appended to `warnings`.
pub(crate) fn list_pids(root: &RootFs, warnings: &mut Vec<String>) -> Vec<u32> {
    let listing = match root.read_dir("/proc", MAX_PROC_ENTRIES) {
        Ok(listing) => listing,
        Err(err) => {
            warnings.push(read_warning("/proc", &err));
            return Vec::new();
        }
    };
    if listing.truncated {
        warnings.push(format!(
            "/proc has more than {MAX_PROC_ENTRIES} entries; the rest were not read"
        ));
    }
    let mut pids: Vec<u32> = listing
        .names
        .iter()
        .filter(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|name| name.parse().ok())
        .collect();
    pids.sort_unstable();
    pids
}

/// Most bytes read from `/proc/<pid>/comm` (the kernel allows 16; the rest is hostile input).
const MAX_COMM_BYTES: usize = 4_096;

/// Longest process name kept, in characters (it ends up inside an item key).
pub(crate) const MAX_COMM_CHARS: usize = 128;

/// The name from `/proc/<pid>/comm`: trimmed, control characters replaced, at most
/// [`MAX_COMM_CHARS`] characters. May be empty.
pub(crate) fn read_comm(root: &RootFs, pid: u32) -> Result<String, FsError> {
    let file = root.read_to_string(&format!("/proc/{pid}/comm"), MAX_COMM_BYTES)?;
    Ok(clean(file.text.trim(), MAX_COMM_CHARS))
}
