# Riptor

A lightweight, security-focused BitTorrent client for **Windows and Linux**, built with Tauri 2 (Rust) and React.

> **Status: early development.** The core client, search, statistics and backups work. VPN support and installers are not done yet. See [What's left to do](#whats-left-to-do).

## Features

**Downloading and seeding**
- `.torrent` files, magnet links and http(s) torrent URLs. Drag and drop, paste a magnet link (Ctrl+V), or use Add.
- An add dialog with file selection, save folder, category, tags and a warning about executables. Links opened from a browser or file manager always show this dialog; nothing is added silently.
- Seeding and ratio rules modelled on qBittorrent:
  - global and per-torrent max ratio, max seeding time and max inactive time;
  - when a limit is reached: Stop, Remove, or Remove with files;
  - queueing: limits on active downloads, uploads and total torrents, "don't count slow torrents", force start, queue order.
- Global and alternative speed limits, categories with default folders, tags, and per-torrent detail tabs (General, Files, Trackers, Peers, Speed).

**Custom storage layer** (`crates/riptor-storage`)
- **Write-back piece buffer:** hash checks read pieces from memory, and verified pieces are written in a few large writes.
- **Seeding read cache:** an LRU cache shared by all torrents, with read-ahead.
- **File handles:** a pool of lazily opened handles with an LRU cap.
- **Preallocation:** sparse or full.
- **Path hardening:** every on-disk path is rebuilt from the raw torrent metadata and proven to stay under the save folder.

**Search** (`crates/riptor-search`)
- Internet Archive (built in), Jackett/Prowlarr via Torznab, and qBittorrent-compatible `.py` search plugins.
- Engines are queried in parallel and results stream in as they arrive, with filters for seeds and size.

**Everything else**
- A statistics page with all-time and session transfer, ratio, 30-day chart, peaks, waste, disk-cache and I/O figures.
- Encrypted, tamper-evident backup and restore of torrents, settings, categories and statistics (`.riptorbackup`), with an optional password.
- 9 themes plus "follow system", and accent colours.
- System tray, desktop notifications, and "Open with" / default-app registration for `.torrent` and `magnet:`.

## Security

Security is a core requirement. Highlights:

- **Hostile torrent metadata:** blocked from writing outside the save folder. This includes Windows drive-prefix tricks, reserved names, alternate data streams and `..`.
- **Protected locations:** save folders can never be system folders, Startup/autostart folders, `~/.ssh`, or Riptor's own data.
- **Downloaded files:** never opened automatically. On Windows, finished downloads get a Mark-of-the-Web.
- **Search plugins:** they are code, so install requires confirming the source and SHA-256. They run isolated (`python -I`, cleared environment, time and output limits; bubblewrap on Linux).
- **Secrets:** Torznab API keys live in the OS credential store.
- **Backups:**
  - Authenticated encryption (XChaCha20-Poly1305): any modification is detected.
  - Without a password, the key is built into the app. That stops editing and detects corruption, but someone who extracts the key could forge a backup. Restores therefore re-validate everything, and bring torrents back stopped by default.
  - Use a password for real protection.
- **App hardening:** no Web UI or HTTP API server, strict CSP, Tauri's isolation pattern with an IPC allowlist, and least-privilege capabilities.

See [SECURITY.md](SECURITY.md) for the threat model and how to report vulnerabilities.

**Known limitation:** the engine has no BitTorrent protocol encryption (MSE/PE), and no web seeds (BEP 19) yet.

## Project layout

```
crates/
  riptor-core/      engine: librqbit session, stats, settings, backups, add/remove, policy
  riptor-storage/   cached, hardened storage layer + benchmarks
  riptor-seeding/   share limits and queueing (pure logic)
  riptor-search/    search providers, Torznab, plugin runner + Python shim (assets/nova)
src-tauri/          Tauri app: IPC commands, tray, file associations
src/                React UI (features/torrents, search, settings, stats)
isolation/          Tauri isolation hook (IPC allowlist)
vendor/librqbit/    vendored librqbit 9.0.1 with Riptor patches (vendor/PATCHES.md)
```

## Building

Requirements:
- Rust (stable) and Node.js 22+.
- **Windows:** Visual Studio 2022 Build Tools with the C++ workload. WebView2 is preinstalled on Windows 11.
- **Linux:**
  ```bash
  sudo apt-get install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev
  ```

Install dependencies:
```bash
npm ci
```

Run the app in development mode:
```bash
npx tauri dev
```

Run all tests:
```bash
cargo test --workspace && npm test
```

Lint:
```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Build installers (NSIS on Windows, AppImage and .deb on Linux):
```bash
npx tauri build
```

Optional: qBittorrent search plugins need Python 3.9+ on the PATH.

**CI** runs the formatter, clippy, tests and security checks (cargo-deny, npm audit) on every push and pull request. Installers are only built for version tags and manual runs, because full-LTO release builds take a long time:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

Or use **Actions › CI › Run workflow** on GitHub. The installers are attached to the run as artifacts.

## What's left to do

### Next: VPN (milestone 4)
- [ ] Import OpenVPN `.ovpn` and WireGuard `.conf` profiles, including Proton's. Reject script and plugin directives (`up`, `down`, `plugin`, `script-security`, `PostUp`, …).
- [ ] Privileged helper (`riptor-vpn-helper`): UAC on Windows, pkexec and polkit on Linux. It takes a fixed set of requests and re-validates and copies configs before use.
- [ ] OpenVPN management-interface client (credentials from the OS keyring, never written to disk). WireGuard tunnel control and status.
- [ ] Kill switch:
  - bind torrent traffic to the VPN adapter;
  - on Windows this needs a librqbit patch, because `bind_device_name` is Linux/macOS only;
  - a watchdog that pauses everything within 1 s if the tunnel drops.
- [ ] While the VPN is required, disable UPnP and LSD.
- [ ] Proton NAT-PMP port forwarding (gateway `10.2.0.1`, renewed every 45 s).
- [ ] VPN page in the UI: profiles, connect and disconnect, status, public-IP check, "only run torrents on VPN".

### Packaging and release (milestone 5)
- [ ] Build and test the NSIS installer, AppImage and `.deb`. Check that installed builds register `.torrent` and `magnet:` correctly.
- [ ] Linux polkit policy and MIME/desktop integration in the packages.
- [ ] Release artifacts with SHA-256 sums. Optional Authenticode signing.
- [ ] About page and log viewer.
- [ ] Check the installer size against the target of under 15 MB.

### Engine and storage
- [ ] **Web seeds (BEP 19)** in vendored librqbit. Most Internet Archive torrents depend on them.
- [ ] Per-torrent speed limits that can change at runtime (needs a librqbit patch).
- [ ] "Force recheck" for a torrent.
- [ ] Profile the storage download path. It currently costs about 35% at very high throughput on a fast NVMe SSD (writes go through the OS page cache), even though the seeding cache measures faster.
- [ ] Finish the real-network check: download and seed a public torrent such as Debian netinst end to end.

### Seeding and queueing extras (milestone 2 leftovers)
- [ ] Weekly scheduler for alternative speed limits.
- [ ] Watched folder that adds `.torrent` files automatically.
- [ ] Move a torrent's files when its category or save folder changes.
- [ ] Create-torrent dialog (uses librqbit `create_torrent`).

### Search
- [ ] Test qBittorrent plugins with a real Python interpreter on Windows. The development machine has no Python, so the end-to-end test only runs in CI.
- [ ] Test the Linux bubblewrap sandbox for plugins.

### UI polish
- [ ] Replace the `window.prompt` used for editing tags with a proper dialog.
- [ ] Remember the detail-panel height and the selected tab between runs.

### Security and quality
- [ ] Check for dependency updates regularly (`cargo update --dry-run`, `npm outdated`; security advisories already fail CI through cargo-deny and npm audit). Upgrade by hand, then build, test and run the app before committing. Dependabot is intentionally not used.
- [ ] Do a full security review pass at the end of each milestone, and fuzz the parsers (Torznab, plugin output, backup, VPN configs).
- [ ] Privately report the Windows path-escape issue in upstream librqbit (see `vendor/PATCHES.md`).
- [ ] Test on Linux (not yet run outside CI).

## License

MIT
