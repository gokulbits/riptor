# Riptor

A lightweight, security-focused BitTorrent client for **Windows and Linux**, built with Tauri 2 (Rust) and React.

> **Status: early development.** The core client, search, statistics and backups work. The app-only VPN is built but not yet tested against a real VPN server, and installers are not done yet. See [What's left to do](#whats-left-to-do).

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
- A plugin catalog, fetched from GitHub on demand: qBittorrent's official plugins and (opt-in) the community list. Pick several, review them together, install.
- No Python? On Windows, Riptor can set up python.org's embeddable Python itself (downloaded on demand, SHA-256 pinned, kept in Riptor's data folder).
- Engines are queried in parallel and results stream in as they arrive, with filters for seeds and size.
- Results without seed/peer counts (the Internet Archive sends none) get them from their trackers: HTTP and UDP scrapes, bounded and optional.

**App-only VPN** (`crates/riptor-vpn`, `crates/riptor-vpn-helper`)
- Import OpenVPN `.ovpn` profiles (Proton's included) and connect from the VPN tab. It needs [OpenVPN Community](https://openvpn.net/community-downloads/) installed.
- **Only Riptor uses the tunnel.** Riptor's sockets are pinned to the tunnel adapter. The tunnel never becomes the computer's default route, so your browser and other apps keep your normal connection.
- **Kill switch:** while it's on and the VPN is down, there's no DHT, listener, trackers or peers, and torrents are held. This includes startup. A 1-second watchdog holds torrents if the tunnel adapter disappears.
- UPnP and LSD are off while the VPN is in use. There's optional Proton port forwarding over NAT-PMP, renewed every 45 s.
- WireGuard `.conf` profiles can be imported, but they only run app-only on Linux. Windows has no supported way to limit a WireGuard tunnel to one app, so Riptor refuses rather than tunnel the whole computer.

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
- **Search plugins:** they are code, so install requires confirming the source and SHA-256, also when picked from the catalog. They run isolated (`python -I`, cleared environment, time and output limits; bubblewrap on Linux). No plugin code ships with Riptor.
- **Tracker lookups:** search results choose which trackers are asked for seed/peer counts, so only public addresses are contacted (checked after DNS and on redirects), replies are size-capped and strictly parsed, and lookups can be turned off. They are skipped while the VPN or kill switch is active, because they don't go through the tunnel. Search requests themselves (Internet Archive, Torznab, plugins) always use the normal connection.
- **Secrets:** Torznab API keys, OpenVPN usernames and passwords, and WireGuard keys live in the OS credential store.
- **VPN profiles are untrusted:**
  - Script, plugin, management and logging directives are removed on import (`up`, `down`, `plugin`, `script-security`, `PostUp`, …).
  - Imported files are picked in a native dialog opened by Rust; the UI never names a path.
  - The elevated helper accepts one fixed request. It re-sanitizes the config itself, copies it to an admin-only folder, and runs OpenVPN from its installed location with the management interface on 127.0.0.1 behind a random password.
  - Known limit: tracker host names are resolved with the system DNS server, outside the tunnel.
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
  riptor-vpn/       VPN profiles, config sanitizers, OpenVPN management parser, NAT-PMP
  riptor-vpn-helper/ the small elevated program that starts OpenVPN
src-tauri/          Tauri app: IPC commands, tray, file associations, VPN manager
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

Optional: qBittorrent search plugins need Python 3.9+ on the PATH. On Windows, the Search engines dialog can set up its own copy instead.

If a build fails with "NASM command not found" (aws-lc-sys), set `AWS_LC_SYS_PREBUILT_NASM=1` or install NASM.

To use the VPN in a development build, build the helper once. It has to sit next to `riptor.exe`; installers bundle it automatically.
```bash
cargo build -p riptor-vpn-helper
```

**CI** runs the formatter, clippy, tests and security checks (cargo-deny, npm audit) on every push and pull request. Installers are only built for version tags and manual runs, because full-LTO release builds take a long time:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

Or use **Actions › CI › Run workflow** on GitHub. The installers are attached to the run as artifacts.

## What's left to do

### Next: finish the VPN (milestone 4)
Done: profile import and sanitizing, the elevated helper, the OpenVPN management client, Windows adapter binding (a librqbit-dualstack-sockets patch), the kill switch and watchdog, NAT-PMP, and the VPN page.
- [ ] **End-to-end test with a real server:** OpenVPN Community plus a Proton OpenVPN profile. Check that the peer IP seen by a tracker is the VPN's, that other apps keep their normal IP, and that killing the tunnel holds torrents within 1 s with nothing leaking on the physical adapter.
- [ ] Resolve tracker host names through the tunnel (they currently use the system DNS server).
- [ ] Public-IP check on the VPN page.
- [ ] WireGuard on Linux: the helper's `wg-quick` path, plus a test on real Linux. WireGuard stays refused on Windows (it can't be limited to one app).
- [ ] Linux: management interface over a 0600 Unix socket instead of 127.0.0.1 TCP.
- [ ] Security review of the VPN milestone.

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
- [ ] Test the Linux bubblewrap sandbox for plugins.
- [ ] Try the popular community plugins against Riptor's shim (the official ones work with Python 3.14 on Windows). Set `RIPTOR_TEST_PYTHON` to run the end-to-end plugin test without a system Python.
- [ ] Bump the embedded Python pin (`riptor-search/src/pyembed.rs`) with each Python security release.
- [ ] Route tracker lookups through the VPN once the kill switch exists (they go out directly today).

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
