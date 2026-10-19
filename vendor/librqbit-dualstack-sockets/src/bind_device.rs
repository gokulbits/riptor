#[cfg(test)]
pub(crate) mod tests;

use crate::Error;
use std::{ffi::CString, num::NonZeroU32, str::FromStr};

#[derive(Debug, Clone)]
pub struct BindDevice {
    #[allow(unused)]
    index: NonZeroU32,
    #[allow(unused)]
    name: CString,
}

impl BindDevice {
    #[cfg(not(windows))]
    pub fn new_from_name(name: &str) -> crate::Result<Self> {
        let name = CString::new(name).map_err(|_| Error::BindDeviceInvalid)?;

        let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
        let index = NonZeroU32::new(index)
            .ok_or_else(|| Error::BindDeviceInvalidError(std::io::Error::last_os_error()))?;
        Ok(Self { index, name })
    }

    // RIPTOR PATCH: Windows support. `name` is an adapter's friendly name (as
    // shown in Network Connections / `Get-NetAdapter`, e.g. "ProtonVPN"), or
    // "#<interface index>".
    #[cfg(windows)]
    pub fn new_from_name(name: &str) -> crate::Result<Self> {
        use network_interface::{NetworkInterface, NetworkInterfaceConfig};
        let index = match name.strip_prefix('#') {
            Some(n) => n.parse::<u32>().ok(),
            None => NetworkInterface::show()
                .map_err(|e| Error::BindDeviceInvalidError(std::io::Error::other(e.to_string())))?
                .into_iter()
                .find(|i| i.name.eq_ignore_ascii_case(name))
                .map(|i| i.index),
        };
        let index = index.and_then(NonZeroU32::new).ok_or(Error::BindDeviceInvalid)?;
        let name = CString::new(name).map_err(|_| Error::BindDeviceInvalid)?;
        Ok(Self { index, name })
    }

    pub fn index(&self) -> NonZeroU32 {
        self.index
    }

    pub fn name(&self) -> &str {
        // We constructed from a string so this can't fail
        unsafe { std::str::from_utf8_unchecked(self.name.to_bytes()) }
    }

    #[cfg(target_os = "macos")]
    pub fn bind_sref(&self, sref: &socket2::Socket, is_v6: bool) -> crate::Result<()> {
        if is_v6 {
            sref.bind_device_by_index_v6(Some(self.index))
                .map_err(Error::BindDeviceSetDeviceError)
        } else {
            sref.bind_device_by_index_v4(Some(self.index))
                .map_err(Error::BindDeviceSetDeviceError)
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    pub fn bind_sref(&self, sref: &socket2::Socket, _is_v6: bool) -> crate::Result<()> {
        let name = self.name.as_bytes_with_nul();
        sref.bind_device(Some(name))
            .map_err(Error::BindDeviceSetDeviceError)
    }

    // RIPTOR PATCH: pin outgoing unicast traffic to the adapter with
    // IP_UNICAST_IF / IPV6_UNICAST_IF. If the adapter disappears (VPN down),
    // sends fail instead of leaving through another interface.
    #[cfg(windows)]
    pub fn bind_sref(&self, sref: &socket2::Socket, is_v6: bool) -> crate::Result<()> {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{
            IP_UNICAST_IF, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF, SOCKET, setsockopt,
        };
        let raw = sref.as_raw_socket() as SOCKET;
        let set = |level: i32, opt: i32, value: u32| -> std::io::Result<()> {
            // SAFETY: `raw` is a valid socket owned by `sref` for this call;
            // the option value is a 4-byte integer, as the API requires.
            let rc = unsafe {
                setsockopt(raw, level, opt, (&value as *const u32).cast(), size_of::<u32>() as i32)
            };
            if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
        };
        let index = self.index.get();
        if is_v6 {
            // IPV6_UNICAST_IF takes the index in host byte order.
            set(IPPROTO_IPV6, IPV6_UNICAST_IF as i32, index).map_err(Error::BindDeviceSetDeviceError)?;
            // Dual-stack sockets also carry IPv4; pin that too where supported.
            let _ = set(IPPROTO_IP, IP_UNICAST_IF as i32, index.to_be());
            Ok(())
        } else {
            // IP_UNICAST_IF takes the index in network byte order.
            set(IPPROTO_IP, IP_UNICAST_IF as i32, index.to_be()).map_err(Error::BindDeviceSetDeviceError)
        }
    }
}

impl FromStr for BindDevice {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new_from_name(s)
    }
}
