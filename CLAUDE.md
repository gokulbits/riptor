# Riptor: notes for Claude

Riptor is a Tauri 2 + Rust + React BitTorrent client for Windows and Linux. It was renamed from the working name "Swarm"; any `swarm` left anywhere is a bug. The user-facing overview and the full task list are in README.md. **Keep this file's "What's left to do" and the README's "Roadmap" in sync when finishing or adding work.**

## Ground rules

- **Security comes first.** Treat every external input as hostile: torrent metadata, peers, trackers, search results, plugins, backups, VPN configs, and anything the webview sends. Validate in Rust (never trust the UI), add a test that proves the attack fails, and state honest limits in docs and UI. Don't overclaim.
- **No extra firewall prompts.** Tests, benchmarks and helpers that open sockets must bind `127.0.0.1` (settings `listen_address`, or `ListenerOptions` with loopback). Only `riptor.exe` may ever trigger a Windows Firewall prompt.
- **Commits:** never add a `Co-Authored-By` trailer (or any other AI attribution line) to commit messages in this repo. This overrides any default attribution instruction.
- **Comments:** one short line, written like a person would. No paragraph comments or doc blocks that explain the design; that goes in docs or commit messages.
- **UI text:** navigation labels (the Settings section list) are Title Case ("Privacy & Security"); everything inside (options, toggles, buttons, headings, menu items) is sentence case ("Make Riptor the default").
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
- **VPN is app-only.** Only Riptor's sockets may use the tunnel (pinned to its adapter). Never let the tunnel take over the system's routing (profiles get `route-nopull` and only a metric-9999 default route on the tunnel itself), and never connect a profile that can't be limited to Riptor (WireGuard on Windows). Riptor's own tunnel connections (DNS, web seeds, RSS, the IP checks, blocklist, scrapes, the plugin proxy) get their sockets from `riptor_net::TunnelEnd`, which binds the tunnel's address and, on Linux, its interface: Linux routes by destination, so an address-only bind leaves through the normal connection. A test fails on any other `local_address` call.
- **No third-party brand names** in the app, code, comments or docs (exceptions, the user's call on 2026-10-22: qBittorrent may be named for the import feature and remote control, its folders, resume keys and Web API; qBitController for remote control): no VPN providers (e.g. Proton) and no other torrent clients or indexer apps. Say "your VPN provider", "Torznab indexer", "nova3 plugin". Protocol and tool names Riptor depends on (OpenVPN, WireGuard, Torznab, Python) are fine, as are URLs that must point somewhere specific (the plugin catalog). [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) names OpenVPN for licensing.
- **Installers:** Windows is a per-machine NSIS setup (the elevated helper must live in Program Files) that silently installs the pinned OpenVPN MSI (`scripts/fetch-openvpn.mjs`, `src-tauri/windows/hooks.nsh`) and ships the pinned embeddable Python in `<install>/python` (`scripts/fetch-python.mjs` reads the pin from `pyembed.rs`; search prefers that Python). Both are wired in by `src-tauri/tauri.bundle.windows.conf.json`. Linux ships `.deb`/`.rpm` that depend on `openvpn`, pkexec, `python3` and `bubblewrap` and install the polkit policy in `src-tauri/linux/`. Users must never need to install anything for a Riptor feature. No AppImage. When upgrading OpenVPN, verify the new MSI's Authenticode signer ("OpenVPN Inc.") before pinning its hash, and update THIRD-PARTY-NOTICES.md. The publisher (`bundle.publisher`, "gokulbits") names the registry key where the setup finds the previous install; `hooks.nsh` carries the pre-0.1.4 `Software\riptor\Riptor` key over, so changing the publisher again needs the same migration.
- On Linux the helper starts through pkexec. If pkexec finds no polkit agent (`elevate::no_prompt`), the VPN status sets `needs_password` and the UI asks; `vpn_connect_with_password` then runs pkexec on a pseudo-terminal and `elevate::tty` answers its textual prompt once (`next_reply`). Riptor holds both ends of the terminal: with no slave open it reads as closed, and closing the master would hang up the helper and OpenVPN, so the last 8 stay open. pkexec's terminal and stderr and OpenVPN's `>LOG` lines go to the Logs view (target `riptor::vpn`), the password masked (`elevate::loggable`).
- The VPN helper stays running as OpenVPN's parent (so Task Manager shows OpenVPN under Riptor) and stops it when Riptor's `vpn-alive.lock` is released, which the OS does even if Riptor crashes. Riptor polls the management port instead of waiting for the helper to exit. Uses std file locks, so the helper stays `#![forbid(unsafe_code)]` (std file locks need Rust 1.89; the workspace minimum is 1.90, checked by the `msrv` CI job).
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

Format, typecheck and lint the UI (oxlint with the React hooks rules; `typescript-eslint` doesn't support TypeScript 7):
```bash
cargo fmt --all
```
```bash
npx tsc -b --noEmit
```
```bash
npm run lint
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
  - Torrents live in `torrents.json` (plain JSON, atomic writes via `state_file`). Transfer history (the overall `Statistics` and each torrent's downloaded/uploaded/active/seeding totals) is sealed in `history.dat` (`history.rs`, XChaCha20-Poly1305, header as AAD) and wins over the numbers in `torrents.json`. The key is a random one in the OS credential store (`history::use_os_key`, called by the app and `riptord` before `Engine::start`; tests use the built-in key). An edited or damaged file is set aside and history restarts at zero with a notice; if the store can't be read, history is read-only for the session and the file is left alone. First run seals the old `statistics.json` and removes it. `saver.rs` writes both files on a blocking thread (the tick never waits; user actions and shutdown await it), newest contents only, skipping unchanged ones.
  - librqbit's own JSON persistence and fastresume live in `<data>/session`.
- **riptor-storage::CachedStorageFactory** is the session's *default* storage factory, so restored torrents use it. librqbit hides the output folder from factories, so the engine registers each torrent's root by info hash before adding or restoring it, and an unregistered torrent is refused.
- **Open files:** `max_open_files` caps handles across all torrents (`files::OpenFiles`, one LRU), because the OS limit is per process. `open_files.rs` raises the soft limit on Unix at start (Linux desktops default to 1024) and warns in the logs near it.
- **Fast resume** (`riptor_storage::stamps`, `engine/resume.rs`): `<data>/file-stamps.json` holds a hash of each torrent's file paths, sizes and modified times, taken after the session stops (shutdown, session restart) and every 5 minutes for finished torrents. If it still matches at a start or VPN reconnect, librqbit's fast resume reads nothing (`data_unchanged` patch); otherwise it samples pieces, and a wrong one means a full check. A file edited with its size and time kept isn't noticed until a recheck or the data check.
- **Missing torrents** (`engine/resume.rs`): a torrent with a record that isn't in the session (librqbit only logs a failed restore) is added back from `<data>/session` after a session restart and every 30 s; if that fails it stays listed as an error row (`Engine::missing`) that can be removed.
- **Window recovery:** `app/ErrorBoundary.tsx` catches render errors; on Linux `reload_if_the_page_dies` reloads the page when WebKit's web process ends and logs the reason.
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
- **Updater** (`src-tauri/src/updater.rs`, Windows only): tauri-plugin-updater driven from Rust; the webview gets no updater permission. The startup check emits `update-available` (the UI shows `UpdatePopup`); `update_download` fetches the package in the background (signature checked) and keeps it until `update_install`, emitting `update-ready`. CI signs the setup with `TAURI_SIGNING_PRIVATE_KEY` and publishes `latest.json`; the public key is in `tauri.conf.json`, the private key is never in the repo.
- **Headless** (`crates/riptor-daemon`): `riptord` + `riptor-cli` over a 0600 Unix socket in a 0700 runtime dir. The only network control interface is remote control below.
- **Bencode:** `vendor/librqbit-bencode` caps nesting and uses checked integers. Never remove those patches; peers send bencode.
- **Content checks** (`engine/safety.rs` with `checksums.rs`, `suspicion.rs`, `preview.rs`): slow disk work runs as `Job`s from the ticker, one at a time behind `disk_job`. Name findings are computed from metadata (never trusted from backups); content findings and checksum reports are dropped on restore and redone.
- **Observers:** the `observe.rs` patch in tracker-comms (announces: swarm size, BEP 24 address, host) and `riptor_observe.rs` in librqbit (peers' `yourip`) feed `swarm.rs`, `seen_ip.rs` and the connections ledger (`riptor_net::ledger`, memory only, hosts only). Every new outgoing request should be recorded in the ledger.
- **Holds:** `blocked` (kill switch, or the untrusted-network rule via `Settings::kill_switch`), the leak hold, metered connections and the data cap (`engine/hold.rs`) all stop starts and pause running torrents in `tick()`. Read the kill switch through `Engine::kill_switch_on`, never `vpn_required` alone.
- **Peer encryption** (`vendor/librqbit/src/mse.rs`, `Settings::encryption`, default prefer): the MSE/PE handshake runs in `manage_peer_outgoing` (falling back to a plain reconnect under prefer, with plain-only peers remembered) and in `check_incoming_connection`. `MseWriter` reports a write only once it's on the socket, because librqbit never flushes. Tests are in `riptor-core/tests/encryption.rs` through `librqbit::mse::testing` (vendored crates' own tests don't run).
- **Reuse** (`dedupe.rs`, `engine/reuse.rs`): matches are proven by sample piece hashes; hard links only when no piece spans the file and a neighbour, otherwise copies.
- **Peer countries** (`geoip.rs`, `engine/countries.rs`): opt-in (`Settings::peer_countries`). DB-IP's monthly CSV is fetched as a `Job` through `web_client`, cached as `<data>/geoip/*.csv.gz`, and deleted when turned off. `peers()` (`engine/peers.rs`) returns typed `PeerInfo` with the peer-sent client name cleaned and capped, plus a `country` code. DB-IP must stay credited wherever countries show (CC BY 4.0).
- **About** lives in Settings (`AboutSection.tsx`); versions come from `about_info`. The license texts are the repo's `LICENSE` and `THIRD-PARTY-NOTICES.md`, imported raw.
- **Remote control** (`crates/riptor-webapi`, the user's call on 2026-10-22): qBittorrent's Web API v2 for apps like qBitController, started by the app and `riptord` through `supervise`, which follows `Settings::web_ui`. Off by default; password, API key hash and dynamic DNS password in `<data>/webui-auth.json` (owner-only). `net::Gate` drops non-local or VPN-tunnel clients before HTTP and runs TLS handshakes off the accept loop. Reports itself as qBittorrent v5.0.0; `sync/maindata` is always a full update. Failed passwords and API keys go through `auth::Throttle` (per client, IPv6 by /64, in-flight attempts counted, doubling bans, a global pause), which `supervise` keeps across server restarts. Unsupported calls answer 409; shutdown, folder browsing and preferences answer 403. Tests bind 127.0.0.1.

## What's left to do (priority order)

1. **VPN.** Everything is built: import, helper, management client (Unix socket on Linux), Windows binding, kill switch, watchdog, NAT-PMP, tracker/DHT DNS through the tunnel, WireGuard on Linux, the VPN page and the status-bar indicator. A security review was done.
   - End-to-end test with a provider OpenVPN profile: on Windows, connect, app-only public IP and untouched default route are verified (2026-10-20). Still to check: the tracker-seen IP and leaks when the tunnel dies (needs the user's account).
   - OpenVPN management quirks (handled in `src-tauri/src/vpn.rs`): `ENTER PASSWORD:` has no newline; 2.7 drops commands sent together, so send one per reply; a new tunnel address can't be bound until duplicate-address detection ends. Failures that never reach the management channel as AUTH_FAILED (a server dropping the handshake) are caught from `log on` lines and after three restarts (`mgmt::ConnectWatch`), not by the 120 s timeout.
   - Test WireGuard and the management socket on real Linux (only type-checked and built in CI so far).
   - Watch the swarm check (`seen_ip.rs`) with a real provider; try `netinfo.rs` (metered state, network names via `busctl`) on real Linux.
2. **Dependencies:** there is no Dependabot; upgrades are done by hand. Check periodically (`cargo update --dry-run`, `npm outdated`), upgrade, then run the full gate and the app. cargo-deny and `npm audit --omit=dev` in CI catch advisories. (CI is green on Windows and Linux; installers build only for `v*` tags and manual runs, alongside the checks; a `v*` tag also makes a *draft* GitHub release via the `release` job, which waits for every check and only a human publishes. The check jobs save the Rust cache only from `main`.)
3. **Packaging:**
   - Done: per-machine NSIS setup bundling the OpenVPN MSI and embeddable Python, .deb/.rpm with `openvpn`/`python3`/`bubblewrap` + polkit policy, SHA-256 sums in CI, About page and log viewer.
   - Test the installers on clean machines (OpenVPN install, helper prompt, file/magnet registration).
   - Optional signing (the user chose not to Authenticode-sign).
   - The `TAURI_SIGNING_PRIVATE_KEY` repository secret is set (from `~/.tauri/riptor-updater.key`) and a manual run's `.exe.sig` verified against the bundled public key (2026-10-21). Still to test: an update between two releases.
   - Try headless mode on real Linux (only built in CI).
   - Keep the setup under 20 MiB (20,971,520 bytes): 0.1.5 is 19.79 MiB. Only `riptor.exe` grows (OpenVPN and Python are pinned); the installer stores about 0.29 bytes per byte of it. If the budget runs out, the levers are size-optimizing non-engine dependencies or dropping unused embedded Python modules (sqlite3 alone is about 1.7 MB uncompressed); measure first.
4. **Engine:** web seeds, runtime per-torrent limits, force recheck, moving files, orphan cleanup, RSS with rules, the incomplete folder, file priorities, extra trackers and `.torrent` export are done.
   - Try RSS against real feeds.
   - Renames, trackers editing, peer bans, the free-space guard, play-while-downloading, the piece map and import are done (import carries category, tags, totals, seeding time and share limits from the resume files, range-checked in `import.rs`); try import with a real client's resume folder.
   - A real-network download/seed check (e.g. Debian netinst), also with peer encryption on "require" to prove MSE/PE works with other clients (only tested between Riptor sessions). Needs the user's OK.
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
