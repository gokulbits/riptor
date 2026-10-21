<div align="center">

<img src="app-icon.svg" alt="Riptor logo" width="112" height="112">

# Riptor

**A fast, security-focused BitTorrent client for Windows and Linux.**

Built-in search, an app-only VPN with a kill switch, encrypted backups, and a storage layer designed for fast downloading and seeding.

[![CI](https://github.com/gokulbits/riptor/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/gokulbits/riptor/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/gokulbits/riptor?include_prereleases&sort=semver)](https://github.com/gokulbits/riptor/releases)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20Linux-informational)
![Built with](https://img.shields.io/badge/built%20with-Rust%20%2B%20Tauri%202-orange)
[![License](https://img.shields.io/github/license/gokulbits/riptor)](LICENSE)

[Download](#installation) · [Features](#features) · [Usage](#usage) · [Security](#security) · [Building](#building-from-source) · [Roadmap](#roadmap)

</div>

> [!NOTE]
> Riptor is in **early development**. Downloading, seeding, search, statistics and backups work and are covered by tests. The app-only VPN has not yet been tested against a real VPN provider, and the installers have not been tested on clean machines. Expect rough edges, and please [report what you find](https://github.com/gokulbits/riptor/issues).

## Contents

- [Why Riptor](#why-riptor)
- [Features](#features)
- [Installation](#installation)
- [Usage](#usage)
- [Security](#security)
- [Building from source](#building-from-source)
- [Project structure](#project-structure)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [Acknowledgements](#acknowledgements)
- [License](#license)

## Why Riptor

- **Safe by default.** Torrent metadata, peers, trackers, search results, plugins, backups and VPN profiles are all treated as hostile input. A torrent can't write outside its save folder, downloads are never opened automatically, and nothing is added without you seeing it first, except by RSS rules you set up yourself.
- **A VPN that only covers Riptor.** Connect an OpenVPN profile from your provider and only Riptor's traffic goes through it. Your browser and other apps keep your normal connection. The kill switch holds every torrent while the tunnel is down, including at startup.
- **Everything included.** The Windows setup installs everything the app needs, and the Linux packages pull in their dependencies through the package manager. No extra downloads or manual setup.
- **Lightweight.** A native Rust engine ([librqbit](https://github.com/ikatson/rqbit)) and a small Tauri shell, not a bundled browser.

## Features

### Downloading and seeding
- Add `.torrent` files, magnet links, info hashes and http(s) torrent URLs. Drag and drop files or links, paste with <kbd>Ctrl</kbd>+<kbd>V</kbd>, or use **Add**.
- The add dialog lets you pick files, the save folder, a category and tags. It shows free disk space and warns when a torrent contains programs or scripts.
- Share limits: maximum ratio, seeding time and inactive time, set globally or per torrent. When a limit is reached, Riptor stops the torrent, removes it, or removes it with its files.
- Queueing: limits on active downloads, uploads and torrents, with "don't count slow torrents", force start and a manual queue order.
- Global and alternative speed limits, with a weekly schedule for the alternative ones. Per-torrent limits can change while it runs.
- Categories with default folders, and tags. Move a torrent, downloaded or still downloading, to another folder, optionally whenever its category changes.
- Force recheck, copy magnet link, export the `.torrent` file, and watched folders (each with its own category and save folder) that add `.torrent` files saved there.
- File priorities (High, Normal, Low) on top of choosing which files to download. Open finished files (never programs) or show them in their folder. Audio and video can be played while they download, once their start is on disk.
- Rename files and folders inside a torrent; the files move on disk.
- Edit a torrent's trackers and re-announce, add peers by address, ban peers, and load an IP blocklist from a URL or a local file.
- A piece map showing which parts are downloaded.
- Downloads pause by themselves when their drive runs low on space (1 GiB by default) and resume when there's room.
- When downloads finish, Riptor can put the computer to sleep or shut it down (with a minute to cancel).
- Import torrents from another client's folder of `.torrent` and `.fastresume` files; existing data is checked, not downloaded again.
- A folder for incomplete downloads: torrents download there and move to their save folder when complete.
- Trackers that are added to every new public torrent, and an option to add new torrents stopped.
- RSS and Atom feeds, with download rules (words or regular expressions, per feed, with a category and folder) that add matching torrents automatically. Feeds are fetched through the VPN when it's on, and torrents with programs are always added stopped.
- Web seeds (BEP 19): torrents that list HTTP servers, such as most Internet Archive items, download from them too.
- Create torrents from your files, with trackers, web seeds and a private flag, and start seeding them in place.

### Storage engine
- **Write-back piece buffer:** hash checks read pieces from memory, and verified pieces reach the disk in a few large writes.
- **Seeding read cache:** a shared cache with read-ahead, so a peer reading a piece block by block costs one disk read, not one per block.
- **Handle pool:** files are opened lazily, with a cap that keeps large torrents within OS limits.
- **Preallocation:** sparse (the default) or full.

### Search
- Built-in Internet Archive search, Torznab indexers, and nova3-compatible Python search plugins.
- A plugin catalog fetched from GitHub. Pick several plugins, review their source and SHA-256 together, then install.
- Engines are queried in parallel, and results stream in as they arrive.
- Results without seed and peer counts get them from their trackers. This is optional.
- Right-click a result to see its details inside Riptor (files, size, trackers, info hash), download it to a folder in one step, open its web page, or copy its link.

### App-only VPN
- Import OpenVPN (`.ovpn`) profiles from your VPN provider and connect from the **VPN** tab.
- Only Riptor's sockets are bound to the tunnel. The tunnel never becomes your computer's default route.
- Searches, seed and peer lookups, plugin requests and `.torrent` links you add go through the tunnel too. Indexers on your own machine or LAN are reached directly.
- Kill switch: while it's on and the VPN is down, there's no DHT, no listening port, no trackers, no peers and no searches.
- A leak check compares the address trackers and peers see with your normal one, and checks name lookups go through the tunnel.
- UPnP and local peer discovery are switched off while the VPN is in use. Optional NAT-PMP port forwarding is available if your provider supports it.
- The status bar shows whether the VPN is connected and the public IP peers see. The IP is masked until you click the eye icon.

### Everything else
- An encrypted, tamper-evident backup and restore of torrents, settings, categories and statistics, with an optional password.
- A statistics page with all-time and session totals, a 30-day chart, peaks, wasted data and disk-cache figures.
- Customisable torrent list: show, hide, reorder and resize columns (including downloaded, uploaded, completed, seeding time, tags, tracker and save path). The layout is saved with your settings and backups. The sidebar filters by status, category, tag and tracker.
- A system tray icon with live speeds. Closing the window keeps Riptor running in the tray (you can turn this off), and Riptor can start in the tray when you sign in.
- The status bar shows free space where downloads go; click either speed to pick a global limit.
- Window size and position, sort order and panel layout are remembered.
- 9 themes, "follow system", and custom accent colours.
- An About page and a log viewer. Logs stay in memory, with tracker passkeys and URL parameters masked.
- In-app updates on Windows: Riptor checks GitHub for a new version and installs it after verifying its signature. On Linux, updates come from your package manager.
- Headless mode on Linux: `riptord` runs Riptor without a window and `riptor-cli` controls it over a private local socket (no network port).
- "Open with" and default-app registration for `.torrent` files and `magnet:` links.

## Installation

Download the latest release from the **[Releases page](https://github.com/gokulbits/riptor/releases)**.

| Platform | File | Install |
| --- | --- | --- |
| Windows 10/11 (64-bit) | `Riptor_<version>_x64-setup.exe` | Run the setup |
| Debian, Ubuntu | `Riptor_<version>_amd64.deb` | `sudo apt install ./Riptor_<version>_amd64.deb` |
| Fedora, openSUSE | `Riptor-<version>-1.x86_64.rpm` | `sudo dnf install ./Riptor-<version>-1.x86_64.rpm` (or `sudo zypper install`) |

**What the installers do**

- **Windows:** installs Riptor for all users in Program Files. It also installs OpenVPN Community silently (only the command-line client and its network drivers), unless OpenVPN is already installed. It carries its own copy of Python for search plugins, kept inside Riptor's folder and not added to `PATH`. Uninstalling Riptor leaves OpenVPN in place, since other programs may use it.
- **Linux:** the package manager pulls in `openvpn`, `pkexec`, `python3` and `bubblewrap`. The package also installs a polkit policy so Riptor can ask for administrator rights when you connect the VPN.

**Verify your download.** Every release includes `SHA256SUMS.txt`:

```bash
sha256sum -c SHA256SUMS.txt --ignore-missing
```

> [!IMPORTANT]
> Release builds are not code-signed yet, so Windows SmartScreen may warn the first time you run the setup. Check the SHA-256 first, then choose **More info › Run anyway**.

## Usage

### Adding torrents
Use **Add** for a `.torrent` file, or **Magnet** for magnet links and URLs. You can also drop files or links onto the window, paste magnet links, info hashes or `.torrent` links anywhere, or open one from your browser. Every torrent goes through the add dialog, where you choose what to download and where to put it.

Right-click a torrent to pause, resume or force-start it, change its category or tags, set share limits, move it in the queue, open its folder, or copy its magnet link. Double-clicking a torrent opens its folder.

### Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>O</kbd> | Add a torrent file |
| <kbd>Ctrl</kbd>+<kbd>U</kbd> | Add magnet links or URLs |
| <kbd>Ctrl</kbd>+<kbd>V</kbd> | Add the magnet links, info hashes or `.torrent` links on the clipboard |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Filter the torrent list |
| <kbd>Ctrl</kbd>+<kbd>K</kbd> | Open search |
| <kbd>↑</kbd> <kbd>↓</kbd> <kbd>Home</kbd> <kbd>End</kbd> | Move through the list (<kbd>Shift</kbd> extends the selection) |
| <kbd>Space</kbd> | Stop the selected torrents, or resume them if they're all stopped |
| <kbd>Enter</kbd> | Open the focused torrent's folder |
| <kbd>Ctrl</kbd>+<kbd>A</kbd> | Select all torrents |
| <kbd>Delete</kbd> | Remove the selected torrents |

### Customising the torrent list
Right-click any column header to show or hide columns, or to reset the layout. Drag a header to move a column, and drag a header's edge to resize it.

### RSS
Open the **RSS** tab, add a feed's URL, and use the filter button to set up download rules. A rule's words must all appear in an article's title (use `|` between alternatives), or switch it to a regular expression; the preview shows which current articles match. Rules apply to new articles as they arrive.

### Search
Open the **Search** tab and choose **Search engines** to:
- add a Torznab indexer (its feed URL and API key);
- install search plugins from the catalog or from a URL.

Plugins are third-party code, so Riptor shows each one's source and SHA-256 before installing it.

### Using the VPN
1. Download an OpenVPN configuration (`.ovpn`) from your VPN provider's website. Choose a server that allows P2P traffic.
2. In the **VPN** tab, choose **Import .ovpn / .conf** and select the file.
3. If the profile needs a login, enter the OpenVPN username and password. Many providers issue these separately from your account login. They are stored in your system's credential store.
4. Select **Connect** and approve the administrator prompt. Creating the tunnel adapter needs administrator rights.
5. Turn on the **kill switch** to make sure torrents never run outside the VPN.

### Headless mode (Linux)
The Linux packages include `riptord`, which runs Riptor without a window, and `riptor-cli` to control it. Its data lives in `~/.local/share/riptor-headless`, separate from the desktop app.

```bash
systemctl --user enable --now riptord
```
```bash
riptor-cli add 'magnet:?xt=urn:btih:…' --dir ~/Downloads
```
```bash
riptor-cli list
```

The VPN needs the desktop app, so headless mode has no VPN.

### Backups
In **Settings › Backup & restore**, create a `.riptorbackup` file of your torrents, settings, categories and statistics, then restore it on any Riptor installation. Add a password for real protection: without one, the file is only tamper-evident.

## Security

Security is a core design requirement. In short:

- **Hostile metadata:** every file path in a torrent is rebuilt and proven to stay inside the save folder. This includes Windows drive-prefix tricks, reserved device names, alternate data streams and `..`.
- **Protected locations:** save folders can never be system folders, startup folders, `~/.ssh` or Riptor's own data.
- **Downloads:** never opened automatically. On Windows, finished files get a Mark-of-the-Web, so SmartScreen and Office treat them as coming from the internet.
- **Search plugins:** they install only after you confirm, and run isolated (`python -I`, cleared environment, time and output limits, and bubblewrap on Linux).
- **VPN profiles:** scripts, plugins and file references are removed or refused. The small elevated helper accepts one fixed request and re-checks every profile before OpenVPN reads it.
- **Web seeds:** only http(s) URLs on public addresses (checked after DNS and on every redirect), exact lengths, and every piece is hash-checked like peer data. A server that keeps failing or sends bad data is dropped.
- **Public IP check:** looked up from `api.ipify.org` (falling back to `ipv4.icanhazip.com`), through the tunnel when the VPN is on. Nothing is sent while the kill switch holds traffic.
- **Secrets:** API keys and VPN credentials live in the OS credential store, never in files or backups.
- **Backups:** XChaCha20-Poly1305 authenticated encryption with an optional Argon2id password. Restores re-validate everything.
- **App hardening:** no web UI or HTTP API server, a strict content security policy, Tauri's isolation pattern with an IPC allowlist, and least-privilege permissions.

**Known limitations:**
- Peer traffic isn't encrypted (the engine has no MSE/PE support).
- On the VPN, search plugins reach the web through a local proxy bound to the tunnel. Plugins that use the standard helpers or Python's `urllib` go through it; one that opens its own sockets would bypass it.
- Web seeds are skipped while a SOCKS5 proxy or anonymous mode is in use, since they would bypass the proxy.

See **[SECURITY.md](SECURITY.md)** for the full threat model and how to report a vulnerability privately.

## Building from source

### Prerequisites
- [Rust](https://rustup.rs/) (stable) and [Node.js](https://nodejs.org/) 22 or newer.
- **Windows:** Visual Studio 2022 Build Tools with the "Desktop development with C++" workload. WebView2 is preinstalled on Windows 11.
- **Linux (Debian/Ubuntu):**
  ```bash
  sudo apt-get install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev
  ```

### Run in development mode

```bash
npm ci
```
```bash
npx tauri dev
```

The VPN needs its helper next to the Riptor executable. Build it once:

```bash
cargo build -p riptor-vpn-helper
```

Search plugins in a development build use Python 3.9+ from your `PATH`. On Windows, the **Search engines** dialog can also set up a private copy.

### Test and lint

```bash
cargo test --workspace
```
```bash
npm test
```
```bash
cargo clippy --workspace --all-targets -- -D warnings
```
```bash
cargo fmt --all --check
```
```bash
npx tsc -b --noEmit
```

### Build the installers

1. Fetch the bundled third-party installers. Both scripts check pinned SHA-256 hashes; they're only needed on Windows.
   ```bash
   node scripts/fetch-openvpn.mjs
   ```
   ```bash
   node scripts/fetch-python.mjs
   ```
2. Build the VPN helper:
   ```bash
   cargo build --release -p riptor-vpn-helper
   ```
3. Copy `target/release/riptor-vpn-helper(.exe)` to `src-tauri/binaries/`, named `Riptor VPN Service-<target-triple>.exe` on Windows (the admin prompt shows that name) or `riptor-vpn-helper-<target-triple>` on Linux. Find your target triple with `rustc -vV`. On Linux, also build `cargo build --release -p riptor-daemon` and copy `riptord` and `riptor-cli` there as `riptord-<target-triple>` and `riptor-cli-<target-triple>`.
4. On Windows, the setup is signed for the in-app updater: set `TAURI_SIGNING_PRIVATE_KEY` to the updater key (its content or path) before building. CI reads it from the repository secret of the same name.
5. Build. On Windows:
   ```bash
   npx tauri build --config src-tauri/tauri.bundle.conf.json --config src-tauri/tauri.bundle.windows.conf.json
   ```
   On Linux, leave out the second `--config`.

> [!TIP]
> If the build fails with "NASM command not found" (from `aws-lc-sys`), set `AWS_LC_SYS_PREBUILT_NASM=1` or install NASM.

### Continuous integration and releases

CI runs formatting, clippy, tests and security checks (`cargo-deny`, `npm audit`) on Windows and Linux for every push and pull request. Installers are built only for version tags and manual runs.

To cut a release:
1. Bump `version` in `Cargo.toml`, `package.json` and `src-tauri/tauri.conf.json`.
2. Push a tag:
   ```bash
   git tag -a v0.1.2 -m "Riptor 0.1.2" && git push origin v0.1.2
   ```
3. CI creates a **draft** release with the installers, `SHA256SUMS.txt` and `latest.json` (what the in-app updater reads), after checking that the files match the tag and the checksums from the build jobs.
4. Review the draft and publish it. Installed copies offer the update once it's published.

Tags containing a hyphen (for example `v0.2.0-beta.1`) become pre-releases.

## Project structure

```
crates/
  riptor-core/        Engine: librqbit session, settings, statistics, backups, save-folder policy
  riptor-storage/     Cached, hardened storage layer and benchmarks
  riptor-seeding/     Share limits and queueing (pure logic)
  riptor-search/      Search providers, Torznab, plugin runner and Python shim, tracker scrapes
  riptor-vpn/         VPN profile sanitizers, OpenVPN management protocol, NAT-PMP
  riptor-vpn-helper/  The small elevated program that starts OpenVPN
  riptor-rss/         RSS and Atom feeds and download rules
  riptor-daemon/      Headless mode for Linux: riptord and riptor-cli
src-tauri/            Desktop app: IPC commands, tray, file associations, VPN manager, installers
src/                  React UI
isolation/            Tauri isolation hook (IPC allowlist)
scripts/              Fetch and verify the bundled third-party installers
vendor/               Vendored librqbit with Riptor patches (see vendor/PATCHES.md)
```

## Roadmap

The main remaining items.

### VPN
Done: profile import and sanitizing, the elevated helper, the OpenVPN management client, Windows adapter binding, the kill switch and watchdog, NAT-PMP, the VPN page, the status indicator with a public-IP check, tracker and DHT lookups through the tunnel, WireGuard on Linux, a private Unix socket for OpenVPN management on Linux, and a security review.
- [ ] Finish the end-to-end test with a real VPN provider. Verified on Windows: connecting with a provider OpenVPN profile works, Riptor appears with the VPN IP while other apps keep the normal IP, the default route is untouched, and OpenVPN stops with Riptor. Still to check: the in-app leak check against a real provider, and that nothing leaks when the tunnel drops.
- [ ] Test WireGuard and the Unix management socket on real Linux (so far only type-checked and built in CI).

### Packaging and release
Done: the Windows setup with OpenVPN and Python bundled, `.deb` and `.rpm` packages (with headless mode), checksums, draft releases from CI, in-app updates on Windows, an About page and a log viewer.
- [ ] Set the `TAURI_SIGNING_PRIVATE_KEY` repository secret before the next tag, and try an update from one release to the next.
- [ ] Try headless mode (`riptord`, `riptor-cli`) on real Linux.
- [ ] Test the installers on clean Windows and Linux machines.
- [ ] Code signing (optional).
- [ ] Keep the Windows setup under 20 MB (19.3 MB for 0.1.2).

### Engine and storage
Done: web seeds (BEP 19), runtime per-torrent speed limits, force recheck, moving and renaming a torrent's files, a free-space guard, editable trackers, peer bans, a piece map, play-while-downloading, importing from other clients, and removing leftover session entries.
- [ ] Download and seed a large public torrent end to end on a real network.
- [ ] Storage benchmark: the download path now measures the same as plain file I/O (medians within 1%), but results are noisy; re-check on a fast NVMe drive.

### Seeding and queueing
Done: a weekly schedule for the alternative speed limits, a watched folder, moving files when a torrent's category changes, creating torrents, RSS feeds with download rules, an incomplete-downloads folder, file priorities, extra trackers for new torrents, and exporting `.torrent` files.
- [ ] Try RSS against real feeds from sites and indexers.

### Search
- [ ] Test the bubblewrap plugin sandbox on Linux.
- [ ] Test popular community plugins. Set `RIPTOR_TEST_PYTHON` to run the plugin end-to-end test without a system Python.
- [ ] Update the pinned embedded Python with each Python security release (3.14.8 is current).

### Security and quality
Done: hostile-input tests for torrents, bencode, backups, Torznab and Internet Archive replies, plugin results, tracker scrapes and VPN profiles. They found and fixed a stack overflow on deeply nested bencode, integer overflow in bencode numbers, and a crash on non-ASCII dates in search results.
- [ ] Check for dependency updates regularly and upgrade by hand. Advisories already fail CI.
- [ ] Do a security review before each release.
- [ ] Privately report the Windows path-escape issue and the bencode crashes to upstream librqbit (see `vendor/PATCHES.md`).
- [ ] Test on real Linux hardware. So far Riptor has only been tested on Linux in CI.

## Contributing

Bug reports, ideas and pull requests are welcome.

- **Bugs and feature requests:** open an [issue](https://github.com/gokulbits/riptor/issues). Include your OS, the Riptor version and the steps to reproduce.
- **Security issues:** don't open a public issue. Follow [SECURITY.md](SECURITY.md).
- **Pull requests:** before submitting, make sure these pass:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `npm test`
  - `npx tsc -b --noEmit`

  Any change that handles external input should come with a test that shows the attack failing.

## Acknowledgements

- [librqbit](https://github.com/ikatson/rqbit), the BitTorrent engine Riptor is built on.
- [Tauri](https://tauri.app/), [React](https://react.dev/) and [Tailwind CSS](https://tailwindcss.com/) for the desktop shell and UI.
- [OpenVPN Community](https://openvpn.net/community/) and [Python](https://www.python.org/), bundled with the Windows setup under their own licenses. See [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## License

Riptor is released under the [MIT License](LICENSE). Bundled third-party software keeps its own license (see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)).
