//! Platform-specific allocation hints. This is the only module in the crate
//! allowed to use `unsafe` (one Win32 ioctl).

use std::fs::File;
use std::io;

/// Mark a file sparse so unwritten regions take no disk space (NTFS needs an
/// explicit flag; Linux/ext4 files are sparse by default).
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn mark_sparse(f: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let mut returned = 0u32;
    // SAFETY: the handle is valid for the lifetime of `f`; FSCTL_SET_SPARSE
    // takes no input/output buffers (null with zero length is documented as
    // "set sparse"), and `returned` outlives the synchronous call.
    let ok = unsafe {
        DeviceIoControl(
            f.as_raw_handle() as _,
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn mark_sparse(_f: &File) -> io::Result<()> {
    Ok(())
}

/// Reserve real disk blocks for the whole file.
#[cfg(target_os = "linux")]
pub fn allocate(f: &File, len: u64) -> io::Result<()> {
    use rustix::fs::{fallocate, FallocateFlags};
    match fallocate(f, FallocateFlags::empty(), 0, len) {
        Ok(()) => Ok(()),
        // Filesystems without fallocate (some FUSE/NFS): set_len already ran.
        Err(e) if e == rustix::io::Errno::OPNOTSUPP => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// NTFS allocates clusters for a non-sparse file on `set_len`.
#[cfg(not(target_os = "linux"))]
pub fn allocate(_f: &File, _len: u64) -> io::Result<()> {
    Ok(())
}

/// Tell the kernel we are about to read this range (seeding read-ahead).
#[cfg(target_os = "linux")]
pub fn will_need(f: &File, offset: u64, len: u64) {
    use std::num::NonZeroU64;
    let _ = rustix::fs::fadvise(
        f,
        offset,
        NonZeroU64::new(len),
        rustix::fs::Advice::WillNeed,
    );
}

#[cfg(not(target_os = "linux"))]
pub fn will_need(_f: &File, _offset: u64, _len: u64) {}
