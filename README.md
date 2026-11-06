<div align="center">

<img src="app-icon.svg" alt="Riptor logo" width="96" height="96">

# Riptor

A BitTorrent client for Windows and Linux, with built-in search and an app-only VPN.

[![CI](https://github.com/gokulbits/riptor/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/gokulbits/riptor/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/gokulbits/riptor?include_prereleases&sort=semver)](https://github.com/gokulbits/riptor/releases)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20Linux-informational)
[![License](https://img.shields.io/github/license/gokulbits/riptor)](LICENSE)

</div>

Riptor is a desktop torrent client built on a Rust engine ([librqbit](https://github.com/ikatson/rqbit)) with a Tauri shell, so there's no bundled browser. It does the usual things a torrent client does, plus three that most don't:

- **Search is built in**: Internet Archive, Torznab indexers, and nova3-compatible Python plugins, queried in parallel.
- **The VPN only covers Riptor**: Import an OpenVPN profile from your provider and only Riptor's sockets use the tunnel. Your browser keeps its normal connection, and your computer's default route is never touched. A kill switch holds every torrent while the tunnel is down.
- **External input is treated as hostile**: Torrent metadata, peers, trackers, search results, plugins, backups and VPN profiles are all validated in Rust, with tests that prove the attacks fail.

> [!NOTE]
> Riptor is in early development. Downloading, seeding, search, statistics and backups work and are covered by tests. The app-only VPN has not been tested against a real VPN provider, and the installers have not been tried on clean machines. Please [report what you find](https://github.com/gokulbits/riptor/issues).

## Install

Grab the latest build from the [releases page](https://github.com/gokulbits/riptor/releases).

| Platform | File | Install |
| --- | --- | --- |
| Windows 10/11 (64-bit) | `Riptor_<version>_x64-setup.exe` | Run the setup |
| Debian, Ubuntu | `Riptor_<version>_amd64.deb` | `sudo apt install ./Riptor_<version>_amd64.deb` |
| Fedora, openSUSE | `Riptor-<version>-1.x86_64.rpm` | `sudo dnf install ./Riptor-<version>-1.x86_64.rpm` |

You never need to install anything else for a Riptor feature. The Windows setup installs for all users and silently installs the OpenVPN command-line client (unless it's already there); it also carries its own copy of Python for search plugins, kept inside Riptor's folder and off your `PATH`. Uninstalling Riptor leaves OpenVPN alone, since other programs may use it. The Linux packages pull in `openvpn`, `pkexec`, `python3` and `bubblewrap`, and install a polkit policy so Riptor can ask for administrator rights when you connect the VPN.

Every release ships `SHA256SUMS.txt`:

```bash
sha256sum -c SHA256SUMS.txt --ignore-missing
```

Release builds aren't code-signed, so Windows SmartScreen will warn the first time you run the setup. Check the hash, then choose **More info** and **Run anyway**.

## Features

**Downloading and seeding.** `.torrent` files, magnet links, info hashes and torrent URLs, by drag and drop, paste or dialog. Per-torrent and global speed limits with a weekly schedule for the alternative ones, queueing, share limits (ratio, seeding time, inactive time), categories with default folders, tags, watched folders, file priorities, force recheck, renaming files inside a torrent, moving finished downloads, an incomplete-downloads folder, a piece map, and torrent creation. Audio and video play while they download.

**Peers and trackers.** Editable trackers with re-announce, manually added peers, peer bans, an IP blocklist from a URL or file, and peer encryption (MSE/PE) that's on by default: it hides BitTorrent from throttling, but it isn't privacy. Peer countries are available once you turn them on. Seeding slots go first to torrents whose swarms have few other seeders, and a torrent tells you when you may be the last one.

**Automation.** RSS and Atom feeds with download rules (words or regular expressions, per feed), a monthly data cap, pausing on metered connections, a free-space guard that holds downloads when a drive runs low, and sleep or shutdown when everything finishes.

**Doesn't download what you already have.** When a new torrent contains files another torrent already has, Riptor hard-links or copies them instead, proven against the new torrent's own piece hashes. Web seeds (BEP 19) are used where a torrent lists them.

**Trust and safety.** The add dialog flags the tricks fake releases use to get malware run: hidden characters in names, `movie.mp4.exe` double extensions, "codec" installers, archives with their password in a text file, a "1080p" release with a tiny video. Anything dangerous-looking is added stopped, and Riptor won't open a file that's really a program. It verifies `SHA256SUMS`, `.sfv` and `.md5` files that ship with a torrent, re-reads what you seed in the background to catch a failing disk, strips your tracker passkey out of copied magnet links and exported `.torrent` files, and previews images and text (including `.nfo`) in-app.

**Storage engine.** Pieces are buffered in memory and flushed in a few large writes; hash checks read from that buffer. Seeding uses a shared read-ahead cache, so a peer reading a piece block by block costs one disk read instead of one per block. File handles are pooled under a cap that keeps large torrents inside OS limits. Preallocation is sparse by default.

**The rest.** Encrypted, tamper-evident backup and restore with an optional password; a statistics page with a 30-day chart; a customisable torrent list with sidebar filters; a tray icon with live speeds; nine themes and custom accents; in-app updates on Windows; headless mode on Linux; remote control from phone apps that speak qBittorrent's Web API; and "open with" registration for `.torrent` files and magnet links.

## Usage

Use **Add** for a `.torrent` file or **Magnet** for links and URLs. You can also drop files on the window, paste a magnet link or info hash anywhere, or open one from your browser. Everything goes through the add dialog, where you pick the files, the folder, a category and tags. Right-click a torrent for pause, force start, share limits, queue position and the rest; double-click to open its folder.

### Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>O</kbd> | Add a torrent file |
| <kbd>Ctrl</kbd>+<kbd>U</kbd> | Add magnet links or URLs |
| <kbd>Ctrl</kbd>+<kbd>V</kbd> | Add whatever's on the clipboard |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Filter the list |
| <kbd>Ctrl</kbd>+<kbd>K</kbd> | Open search |
| <kbd>Space</kbd> | Stop the selection, or resume it if all stopped |
| <kbd>Enter</kbd> | Open the focused torrent's folder |
| <kbd>Delete</kbd> | Remove the selection |

### Search

Open the **Search** tab and choose **Search engines** to add a Torznab indexer (feed URL and API key) or install plugins from the catalog or a URL. Plugins are third-party code, so Riptor shows each one's source and SHA-256 before installing, and runs them isolated. Right-click a result for its files, size and trackers, to download it in one step, or to open its page.

### VPN

1. Download an OpenVPN profile (`.ovpn`) from your provider, from a server that allows P2P.
2. In the **VPN** tab, choose **Import .ovpn / .conf**.
3. Enter the OpenVPN username and password if the profile needs them. Most providers issue these separately from your account login; they go in your system's credential store.
4. **Connect**, and approve the administrator prompt: creating the tunnel adapter needs it.
5. Turn on the **kill switch** so torrents never run outside the tunnel.

Searches, tracker scrapes, plugin requests, RSS and `.torrent` URLs go through the tunnel too; indexers on your own machine or LAN stay direct. With the kill switch on and the VPN down there's no DHT, no listening port, no trackers, no peers and no searches. A leak check compares the address trackers and peers see with your normal one, and peers that report seeing your real address will hold torrents until you've looked. The **connections ledger** lists every host Riptor contacted this session and whether it went through the tunnel; it's kept in memory only. UPnP and local peer discovery are off while the VPN is in use; NAT-PMP forwarding is there if your provider supports it.

### Moving from qBittorrent

Riptor takes over your torrents without downloading anything again.

1. In qBittorrent, pause the torrents that are still downloading, or close it. Seeding ones can keep going.
2. In Riptor, open the three-dot menu and choose **Import from qBittorrent**. It finds them itself.
3. **Import.** Each torrent keeps its folder, category, tags, downloaded and uploaded totals, seeding time and share limits, and qBittorrent's all-time totals are added to Riptor's statistics. Files are checked where they are.
4. Once you're happy, remove the torrents in qBittorrent and keep the files. Don't run the same torrent in both apps.

If it can't find them, with a portable install for example, click **Choose folder** and pick qBittorrent's `BT_backup`:

- Windows: `%LOCALAPPDATA%\qBittorrent\BT_backup`
- Linux: `~/.local/share/qBittorrent/BT_backup`, or `~/.var/app/org.qbittorrent.qBittorrent/data/qBittorrent/BT_backup` for the Flatpak

Importing twice is safe: torrents already in Riptor are left alone and totals aren't counted again.

### Remote control

Riptor can speak qBittorrent's Web API, so apps like qBitController manage it over your network. It's off by default.

1. In **Settings > Remote Control**, turn on **Allow remote control**, set a password, save.
2. In the app, add a server at `https://<this computer's IP>:8080` with your username (`admin` unless you changed it) and that password.
3. Turn on "trust self-signed certificates": Riptor makes its own unless you give it one.

Only devices on your local network are accepted unless you allow more, and never anything arriving through the VPN tunnel. Search and RSS aren't available remotely, and a few qBittorrent-only switches answer "not supported".

### Headless mode (Linux)

The Linux packages include `riptord`, which runs without a window, and `riptor-cli`, which controls it over a private local socket, with no network port. Its data lives in `~/.local/share/riptor-headless`, separate from the desktop app. The VPN needs the desktop app, so headless mode has none.

```bash
systemctl --user enable --now riptord
```

```bash
riptor-cli add 'magnet:?xt=urn:btih:...' --dir ~/Downloads
```

### Backups

**Settings > Backup & restore** writes a `.riptorbackup` file holding your torrents, settings, categories and statistics, restorable on any Riptor install. Add a password for real protection: without one the file is only tamper-evident.

## Security

The full threat model and how to report a vulnerability privately are in [SECURITY.md](SECURITY.md). The short version:

- Every file path in a torrent is rebuilt and proven to stay inside the save folder, including Windows drive prefixes, reserved device names, alternate data streams and `..`.
- Save folders can never be system folders, startup folders, `~/.ssh` or Riptor's own data.
- Downloads are never opened automatically. On Windows, finished files get a Mark-of-the-Web so SmartScreen and Office treat them as internet files.
- Search plugins install only after you confirm, and run with `python -I`, a cleared environment, time and output limits, and bubblewrap on Linux.
- VPN profiles have scripts, plugins and file references removed or refused. The small elevated helper accepts one fixed request and re-checks every profile before OpenVPN reads it.
- Web seeds must be http(s) on public addresses (checked after DNS and every redirect), with exact lengths and hash checks on every piece. Servers that fail or lie are dropped.
- API keys and VPN credentials live in the OS credential store, never in files or backups.
- Backups use XChaCha20-Poly1305 with an optional Argon2id password, and restores re-validate everything.
- Transfer statistics are sealed with a key from the credential store, so they can't be edited by hand.
- No server listens unless you turn on remote control. Strict CSP, Tauri's isolation pattern with an IPC allowlist, least-privilege permissions.

Honest limits:

- Peer encryption (MSE/PE) is obfuscation, not privacy: RC4 with an unauthenticated key exchange, so anyone in the middle can still read it. Tested between Riptor sessions, not yet against other clients.
- Fake-release checks are heuristics. They catch common tricks, not malware.
- On the VPN, plugins reach the web through a local proxy bound to the tunnel. A plugin that opens its own sockets would bypass it.
- Web seeds are skipped while a SOCKS5 proxy or anonymous mode is on, since they'd bypass it.
- The update check uses its own connection and never goes through the VPN. The ledger shows it.
- What peers claim to see can be made up, so the swarm check needs several unrelated sources.
- Metered state and network names come from Windows or NetworkManager (`busctl`). Without them a network counts as untrusted.

## Building from source

You'll need [Rust](https://rustup.rs/) (stable) and [Node.js](https://nodejs.org/) 22+. On Windows, Visual Studio 2022 Build Tools with "Desktop development with C++"; WebView2 ships with Windows 11. On Debian/Ubuntu:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev
```

Then:

```bash
npm ci && cargo build -p riptor-vpn-helper && npx tauri dev
```

The VPN helper has to sit next to the Riptor executable, which is what that middle step is for. Search plugins in a dev build use Python 3.9+ from your `PATH`; on Windows the **Search engines** dialog can set up a private copy.

Checks, all of which must be clean:

```bash
cargo test --workspace && npm test && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check && npx tsc -b --noEmit && npm run lint
```

### Installers

1. Fetch the bundled third-party installers (Windows only; both scripts verify pinned SHA-256 hashes):
   ```bash
   node scripts/fetch-openvpn.mjs && node scripts/fetch-python.mjs
   ```
2. Build the helper: `cargo build --release -p riptor-vpn-helper`.
3. Copy `target/release/riptor-vpn-helper(.exe)` into `src-tauri/binaries/` as `Riptor VPN Service-<target-triple>.exe` on Windows (that name is what the admin prompt shows) or `riptor-vpn-helper-<target-triple>` on Linux. `rustc -vV` prints your target triple. On Linux also build `riptor-daemon` and copy `riptord` and `riptor-cli` there with the same suffix.
4. On Windows, set `TAURI_SIGNING_PRIVATE_KEY` to the updater key (content or path) so the setup is signed for the in-app updater.
5. Build:
   ```bash
   npx tauri build --config src-tauri/tauri.bundle.conf.json --config src-tauri/tauri.bundle.windows.conf.json
   ```
   Leave out the second `--config` on Linux.

If it fails with "NASM command not found" (from `aws-lc-sys`), set `AWS_LC_SYS_PREBUILT_NASM=1` or install NASM.

### Releases

CI runs formatting, clippy, tests and security checks (`cargo-deny`, `npm audit`) on Windows and Linux for every push and pull request. Installers build only for version tags and manual runs.

1. Bump `version` in `Cargo.toml`, `package.json` and `src-tauri/tauri.conf.json`.
2. Tag it: `git tag -a v0.1.9 -m "Riptor 0.1.9" && git push origin v0.1.9`.
3. CI makes a **draft** release with the installers, `SHA256SUMS.txt` and the `latest.json` the updater reads, after checking the files match the tag.
4. Review and publish. Installed copies offer the update once it's public.

A tag with a hyphen in it (`v0.2.0-beta.1`) becomes a pre-release.

## Contributing

Bug reports, ideas and pull requests are welcome.

- **Bugs and features:** open an [issue](https://github.com/gokulbits/riptor/issues) with your OS, the Riptor version and steps to reproduce.
- **Security:** don't open a public issue. Follow [SECURITY.md](SECURITY.md).
- **Pull requests:** make the checks above pass. Anything that handles external input needs a test showing the attack fail.

## Acknowledgements

- [librqbit](https://github.com/ikatson/rqbit), the BitTorrent engine Riptor is built on.
- [Tauri](https://tauri.app/), [React](https://react.dev/) and [Tailwind CSS](https://tailwindcss.com/).
- [OpenVPN Community](https://openvpn.net/community/) and [Python](https://www.python.org/), bundled with the Windows setup under their own licenses. See [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
- [IP geolocation by DB-IP](https://db-ip.com) (CC BY 4.0) for peer countries.
- [Claude Code](https://claude.com/claude-code), which helped build Riptor.

## License

Riptor is MIT licensed, see [LICENSE](LICENSE). Bundled third-party software keeps its own license ([THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)).
