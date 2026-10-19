# Third-party software shipped with Riptor

## OpenVPN Community (Windows setup only)

The Windows setup (`Riptor_*_x64-setup.exe`) contains the unmodified,
officially signed OpenVPN Community installer, version 2.7.8-I001, and runs it
so the VPN tab works out of the box. Riptor does not link to or modify
OpenVPN; it starts OpenVPN as a separate program.

- License: GNU General Public License version 2, with OpenVPN's OpenSSL linking
  exception. The full license text:
  https://github.com/OpenVPN/openvpn/blob/v2.7.8/COPYING
- Source code: https://github.com/OpenVPN/openvpn/tree/v2.7.8 and the Windows
  installer sources at https://github.com/OpenVPN/openvpn-build
- Original download: https://swupdate.openvpn.net/community/releases/OpenVPN-2.7.8-I001-amd64.msi
  (SHA-256 `1ea61a3056905b90607d4b0f084c04e5cf3043e07cf192d9cd44b0474f29cc49`)

OpenVPN is installed as its own program (it appears in Windows' installed
apps) and stays installed if Riptor is removed.

On Linux, OpenVPN comes from the distribution's own `openvpn` package, which
the `.deb` and `.rpm` packages depend on.

## Python (Windows setup only)

The Windows setup installs the official python.org "embeddable package",
version 3.14.8 (x64), unmodified, into Riptor's install folder
(`python\`). Riptor uses it only to run search plugins, isolated (`-I`,
cleared environment, time and output limits). It is not added to PATH and does
not affect any other Python on the computer.

- License: Python Software Foundation License Version 2
  (https://docs.python.org/3/license.html); the license text is also inside
  the install folder as `python\LICENSE.txt`.
- Original download: https://www.python.org/ftp/python/3.14.8/python-3.14.8-embed-amd64.zip
  (SHA-256 pinned in `crates/riptor-search/src/pyembed.rs`)

On Linux, the `.deb` and `.rpm` packages depend on the distribution's
`python3` (and `bubblewrap`, which sandboxes plugins).
