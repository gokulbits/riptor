# Riptor: notes for Claude

Riptor is a Tauri 2 + Rust + React BitTorrent client for Windows and Linux. It was renamed from the working name "Swarm"; any `swarm` left anywhere is a bug. The user-facing overview and the full task list are in README.md. **Keep this file's "What's left to do" and the README's "Roadmap" in sync when finishing or adding work.**

## Ground rules

- **Security comes first.** Treat every external input as hostile: torrent metadata, peers, trackers, search results, plugins, backups, VPN configs, and anything the webview sends. Validate in Rust (never trust the UI), add a test that proves the attack fails, and state honest limits in docs and UI. Don't overclaim.
- **No extra firewall prompts.** Tests, benchmarks and helpers that open sockets must bind `127.0.0.1` (settings `listen_address`, or `ListenerOptions` with loopback). Only `riptor.exe` may ever trigger a Windows Firewall prompt.
- **Commits:** never add a `Co-Authored-By` trailer (or any other AI attribution line) to commit messages in this repo. This overrides any default attribution instruction.
- **Comments:** one short line, written like a person would. No paragraph comments or doc blocks that explain the design; that goes in docs or commit messages.
- **PowerShell script execution is disabled** on the dev machine. Use inline PowerShell, never `.ps1` files, and never bypass the execution policy.
- The webview gets no opener, notification or deep-link permissions. New IPC commands must be added in **three** places:
  1. `src-tauri/src/commands.rs`
  2. `generate_handler!` in `src-tauri/src/lib.rs`
  3. the allowlist in `isolation/index.js`

  A command missing from the allowlist is silently blocked.
- Changes to vendored librqbit are marked `// RIPTOR PATCH:` and listed in `vendor/PATCHES.md`.
- Every save folder (add, category, settings, restore) goes through `riptor_core::paths::check_save_root`.
- Every on-disk torrent path goes through `riptor_storage::sanitize`. Never use librqbit's `relative_filename`: on Windows, `PathBuf::push` with a `C:` component escapes the folder.
- `unsafe` is forbidden except in `riptor-storage/src/os.rs`, `src-tauri/src/associations.rs`, `src-tauri/src/elevate.rs` and the vendored dualstack sockets `bind_device` patch (documented OS calls).
- **VPN is app-only.** Only Riptor's sockets may use the tunnel (pinned to its adapter). Never let the tunnel take over the system's routing (profiles get `route-nopull` and only a metric-9999 default route on the tunnel itself), and never connect a profile that can't be limited to Riptor (WireGuard on Windows).
- **No third-party brand names** in the app, code, comments or docs: no VPN providers (e.g. Proton) and no other torrent clients or indexer apps. Say "your VPN provider", "Torznab indexer", "nova3 plugin". Protocol and tool names Riptor depends on (OpenVPN, WireGuard, Torznab, Python) are fine, as are URLs that must point somewhere specific (the plugin catalog). [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) names OpenVPN for licensing.
- **Installers:** Windows is a per-machine NSIS setup (the elevated helper must live in Program Files) that silently installs the pinned OpenVPN MSI (`scripts/fetch-openvpn.mjs`, `src-tauri/windows/hooks.nsh`) and ships the pinned embeddable Python in `<install>/python` (`scripts/fetch-python.mjs` reads the pin from `pyembed.rs`; search prefers that Python). Both are wired in by `src-tauri/tauri.bundle.windows.conf.json`. Linux ships `.deb`/`.rpm` that depend on `openvpn`, pkexec, `python3` and `bubblewrap` and install the polkit policy in `src-tauri/linux/`. Users must never need to install anything for a Riptor feature. No AppImage. When upgrading OpenVPN, verify the new MSI's Authenticode signer ("OpenVPN Inc.") before pinning its hash, and update THIRD-PARTY-NOTICES.md.
- The VPN helper stays running as OpenVPN's parent (so Task Manager shows OpenVPN under Riptor) and stops it when Riptor's `vpn-alive.lock` is released, which the OS does even if Riptor crashes. Riptor polls the management port instead of waiting for the helper to exit. Uses std file locks, so the helper stays `#![forbid(unsafe_code)]` (MSRV 1.89).
- Build `riptor-vpn-helper` once (`cargo build -p riptor-vpn-helper`) so it sits next to `riptor.exe` in dev. Release bundles add it through externalBin; on Windows it ships as `Riptor VPN Service.exe`, the name UAC shows.

## Commands

The Rust toolchain lives in `~/.cargo/bin`. Add it to PATH in each shell.

Run the app in development mode (it rebuilds and restarts on Rust changes):
```bash
npx tauri dev
```

Run the Rust tests and the UI tests:
```bash
cargo test --workspace
```
```bash
npm test
```

Lint, which must be clean:
```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Format and typecheck:
```bash
cargo fmt --all
```
```bash
npx tsc -b --noEmit
```

CI runs Node 22 (npm 10), which rejects some lockfiles written by a newer local npm. After changing `package-lock.json`, check it the way CI will:
```bash
npx -y npm@10 ci --dry-run --ignore-scripts
```

Storage benchmark (results are noisy; compare medians):
```bash
cargo bench -p riptor-storage
```

**Inspecting the running UI:** start `tauri dev` with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` and drive the page over CDP (`http://127.0.0.1:9222/json`). React StrictMode mounts effects twice, so don't use "run once" ref guards in effects (that bug once disconnected all engine events).

## Architecture in one breath

- **riptor-core::Engine** owns the librqbit `Session`.
  - It adds every torrent paused, and the once-a-second `tick()` lets `riptor-seeding::Scheduler` decide start/queue/share-limit actions.
  - It keeps all-time counters in `torrents.json` and statistics in `statistics.json`, using atomic writes via `state_file`.
  - librqbit's own JSON persistence and fastresume live in `<data>/session`.
- **riptor-storage::CachedStorageFactory** is the session's *default* storage factory, so restored torrents use it. librqbit hides the output folder from factories, so the engine registers each torrent's root by info hash before adding or restoring it, and an unregistered torrent is refused.
- **Pieces:** buffered per piece in memory, then flushed synchronously in `on_piece_completed`, which runs *before* the piece is announced to peers.
- **UI preferences:** anything that should survive a backup (e.g. the torrent list's `columns`) lives in `Settings` and is validated in Rust. localStorage is only for throwaway view state (sort, panel height, tab).
- **Backups** (`riptor-core/src/backup.rs`): `RIPTRBK\0` header, XChaCha20-Poly1305 with the header as AAD, deflate+JSON payload. The built-in key (tamper-evident only) or an Argon2id password.
- **Search plugins:** run through Riptor's own JSON-line shim in `crates/riptor-search/assets/nova`. Engines are loaded by file path and never put on `sys.path`. The catalog (`catalog.rs`) is fetched from GitHub at runtime; never vendor plugin code. Riptor's own Python (`pyembed.rs`, Windows) is python.org's embeddable zip, pinned by SHA-256. `RIPTOR_TEST_PYTHON=<python.exe>` lets the end-to-end plugin test run without a system Python.
- **Seed/peer lookups** (`scrape.rs`): results without counts are scraped from their trackers (HTTP + UDP BEP 15). Only public addresses are contacted (`riptor-net`); tests use `Net::new(true)` to allow 127.0.0.1. `search_start` routes them through the tunnel on the VPN and holds them under the kill switch.
- **Web seeds** (`webseed.rs`): workers per torrent, managed from `tick()`, fetch whole pieces over HTTP and hand them to the librqbit `webseed_*` patch. Tests call `Engine::allow_local_web_seeds()` to use a loopback server.
- **RSS** (`crates/riptor-rss`, `src-tauri/src/rss.rs`): feeds are parsed with quick-xml (no entity expansion, size/depth/item caps) and stored in `<data>/rss.json` with seen-ids so a rule fires once per article. A minute ticker refreshes due feeds through `Engine::web_client` (VPN-aware) and adds picks via `preview` + `add`; anything with programs is added stopped.
- **Incomplete folder / file priority:** `Record::move_on_complete` sends a finished download through `reattach` to its save folder (queued from `tick()`, run by the ticker). File priority reorders librqbit's file list (`set_file_priority_levels` patch), applied once per live state.
- **Renames** (`Record::renames`, `TorrentRoot::renames`): per-file relative paths applied in `layout_for` before the usual sanitizing; renaming runs through `reattach` like a move. Restored backups keep only renames that pass `clean_rel_path`.
- **Free-space guard:** `check_disk_space` in `tick()` holds unfinished torrents (`TorrentState::NoSpace`) below `min_free_mib` and the scheduler skips their starts.
- **Updater** (`src-tauri/src/updater.rs`, Windows only): tauri-plugin-updater driven from Rust; the webview gets no updater permission. CI signs the setup with `TAURI_SIGNING_PRIVATE_KEY` and publishes `latest.json`; the public key is in `tauri.conf.json`, the private key is never in the repo.
- **Headless** (`crates/riptor-daemon`): `riptord` + `riptor-cli` over a 0600 Unix socket in a 0700 runtime dir; no network control interface, ever.
- **Bencode:** `vendor/librqbit-bencode` caps nesting and uses checked integers. Never remove those patches; peers send bencode.

## What's left to do (priority order)

1. **VPN.** Everything is built: import, helper, management client (Unix socket on Linux), Windows binding, kill switch, watchdog, NAT-PMP, tracker/DHT DNS through the tunnel, WireGuard on Linux, the VPN page and the status-bar indicator. A security review was done.
   - End-to-end test with a provider OpenVPN profile: on Windows, connect, app-only public IP and untouched default route are verified (2026-10-20). Still to check: the tracker-seen IP and leaks when the tunnel dies (needs the user's account).
   - OpenVPN management quirks (handled in `src-tauri/src/vpn.rs`): `ENTER PASSWORD:` has no newline; 2.7 drops commands sent together, so send one per reply; a new tunnel address can't be bound until duplicate-address detection ends. Failures that never reach the management channel as AUTH_FAILED (a server dropping the handshake) are caught from `log on` lines and after three restarts (`mgmt::ConnectWatch`), not by the 120 s timeout.
   - Test WireGuard and the management socket on real Linux (only type-checked and built in CI so far).
2. **Dependencies:** there is no Dependabot; upgrades are done by hand. Check periodically (`cargo update --dry-run`, `npm outdated`), upgrade, then run the full gate and the app. cargo-deny and `npm audit --omit=dev` in CI catch advisories. (CI is green on Windows and Linux; installers build only for `v*` tags and manual runs; a `v*` tag also makes a *draft* GitHub release via the `release` job, which only a human publishes.)
3. **Packaging:**
   - Done: per-machine NSIS setup bundling the OpenVPN MSI and embeddable Python, .deb/.rpm with `openvpn`/`python3`/`bubblewrap` + polkit policy, SHA-256 sums in CI, About page and log viewer.
   - Test the installers on clean machines (OpenVPN install, helper prompt, file/magnet registration).
   - Optional signing (the user chose not to Authenticode-sign).
   - The `TAURI_SIGNING_PRIVATE_KEY` repository secret is set (from `~/.tauri/riptor-updater.key`) and a manual run's `.exe.sig` verified against the bundled public key (2026-10-21). Still to test: an update between two releases.
   - Try headless mode on real Linux (only built in CI).
   - Keep the setup under 20 MB (19.3 MB for 0.1.2).
4. **Engine:** web seeds, runtime per-torrent limits, force recheck, moving files, orphan cleanup, RSS with rules, the incomplete folder, file priorities, extra trackers and `.torrent` export are done.
   - Try RSS against real feeds.
   - Renames, trackers editing, peer bans, the free-space guard, play-while-downloading, the piece map and import are done; try import with a real client's resume folder.
   - A real-network download/seed check (e.g. Debian netinst). Needs the user's OK.
   - Storage download path: the benchmark now shows the store level with plain I/O (noisy; re-check on fast NVMe).
5. **Search:**
   - Test the bubblewrap sandbox on Linux.
   - Try popular community plugins against the shim (official ones work on Windows with Python 3.14).
   - Bump the embedded Python pin in `pyembed.rs` with Python security releases (3.14.8 is current).
   - On the VPN, searches, scrapes and URL adds go through the tunnel, and plugins through a loopback proxy (`proxy.rs`) bound to it; LAN/loopback hosts stay direct (`riptor_net::stays_local`). A plugin that ignores proxy settings and opens its own sockets would bypass it.
6. **Security and quality:**
   - A security review before each release.
   - Hostile-input tests exist for torrents, bencode, backups, search replies, plugin results, scrapes and VPN profiles (`tests/hostile_input.rs` in core, search and vpn). Extend them with new parsers.
   - Privately report librqbit's Windows path escape and the bencode crashes (now patched in `vendor/librqbit-bencode`) upstream. Needs the user's OK.
   - Test on real Linux.
