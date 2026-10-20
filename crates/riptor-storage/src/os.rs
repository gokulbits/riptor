//! Allocation hints. The only `unsafe` in this crate.

use std::fs::File;
use std::io;

/// NTFS needs the flag; Linux files are sparse already.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn mark_sparse(f: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let mut returned = 0u32;
    // SAFETY: valid handle; FSCTL_SET_SPARSE takes no buffers.
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

#[cfg(target_os = "linux")]
pub fn allocate(f: &File, len: u64) -> io::Result<()> {
    use rustix::fs::{fallocate, FallocateFlags};
    match fallocate(f, FallocateFlags::empty(), 0, len) {
        Ok(()) => Ok(()),
        // No fallocate (some FUSE/NFS); set_len already ran.
        Err(e) if e == rustix::io::Errno::OPNOTSUPP => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn allocate(_f: &File, _len: u64) -> io::Result<()> {
    Ok(())
}

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

/// Bytes the current user can still write on the volume holding `path`.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn free_space(path: &std::path::Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut avail = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the totals we
    // don't need may be null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut avail,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(avail)
}

#[cfg(target_os = "linux")]
pub fn free_space(path: &std::path::Path) -> io::Result<u64> {
    let s = rustix::fs::statvfs(path)?;
    Ok(s.f_bavail.saturating_mul(s.f_frsize))
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn free_space(_path: &std::path::Path) -> io::Result<u64> {
    Err(io::ErrorKind::Unsupported.into())
}
