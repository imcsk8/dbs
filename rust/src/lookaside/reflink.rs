//! BTRFS Copy-on-Write (CoW) reflink operations and storage optimization utilities.
//!
//! Provides support for the Linux `FICLONE` ioctl system call to create zero-cost,
//! instant file clones within BTRFS (and XFS) filesystems, with automated fallbacks
//! to hard links and standard streaming copies when cross-filesystem operations occur.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

/// Resulting mode utilized when cloning or copying a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflinkMode {
    /// BTRFS/XFS zero-cost Copy-on-Write clone created via `FICLONE` ioctl.
    Reflink,
    /// Direct hard link pointing to the identical inode.
    Hardlink,
    /// Byte-by-byte full data copy.
    Copy,
}

impl std::fmt::Display for ReflinkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReflinkMode::Reflink => write!(f, "BTRFS CoW reflink (instant, 0 disk cost)"),
            ReflinkMode::Hardlink => write!(f, "hardlink (shared inode)"),
            ReflinkMode::Copy => write!(f, "standard copy"),
        }
    }
}

/// Clones `src` to `dst` using the Linux `FICLONE` ioctl on copy-on-write filesystems.
///
/// # Arguments
/// * `src` - Path to the existing source file.
/// * `dst` - Destination path for the cloned file.
#[cfg(target_os = "linux")]
pub fn reflink_file(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;

    let src_file = match File::open(src) {
        Ok(f) => f,
        Err(e) => return Err(e),
    };

    let dst_file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(dst)
    {
        Ok(f) => f,
        Err(e) => return Err(e),
    };

    // FICLONE ioctl command: _IOW(0x94, 9, int) = 0x40049409
    const FICLONE: libc::c_ulong = 0x40049409;

    let ret = unsafe { libc::ioctl(dst_file.as_raw_fd(), FICLONE, src_file.as_raw_fd()) };
    if ret != 0 {
        let err = io::Error::last_os_error();
        // Remove empty destination file created during open
        let _ = fs::remove_file(dst);
        return Err(err);
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn reflink_file(_src: &Path, _dst: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "FICLONE reflink is only supported on Linux",
    ))
}

/// Automatically clones or copies `src` to `dst`, attempting BTRFS reflink first,
/// falling back to hard link if possible, and ultimately standard file copy.
pub fn reflink_or_copy(src: &Path, dst: &Path) -> io::Result<ReflinkMode> {
    if let Some(parent) = dst.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    // Attempt BTRFS/XFS FICLONE reflink first
    match reflink_file(src, dst) {
        Ok(()) => return Ok(ReflinkMode::Reflink),
        Err(_) => {
            // Reflink failed (e.g. cross-device EXDEV or unsupported filesystem)
        }
    }

    // Try hardlink if destination does not exist yet
    if !dst.exists() {
        match fs::hard_link(src, dst) {
            Ok(()) => return Ok(ReflinkMode::Hardlink),
            Err(_) => {
                // Hardlink failed (cross-device or permission)
            }
        }
    }

    // Fallback: standard file copy
    match fs::copy(src, dst) {
        Ok(_) => Ok(ReflinkMode::Copy),
        Err(e) => Err(e),
    }
}

/// Detects whether the filesystem hosting the given path is BTRFS.
#[cfg(target_os = "linux")]
pub fn is_btrfs(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // Use parent if path does not exist yet
    let target = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| Path::new(".").to_path_buf())
    };

    let c_path = match CString::new(target.as_os_str().as_bytes()) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
    if res == 0 {
        // BTRFS_SUPER_MAGIC = 0x9123683E
        const BTRFS_MAGIC: libc::__fsword_t = 0x9123683E;
        stat.f_type == BTRFS_MAGIC
    } else {
        false
    }
}

#[cfg(not(target_os = "linux"))]
pub fn is_btrfs(_path: &Path) -> bool {
    false
}
