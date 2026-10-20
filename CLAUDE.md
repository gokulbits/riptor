# Riptor: notes for Claude

Riptor is a Tauri 2 + Rust + React BitTorrent client for Windows and Linux. It was renamed from the working name "Swarm"; any `swarm` left anywhere is a bug. The user-facing overview and the full task list are in README.md. **Keep the "What's left to do" lists in both files in sync when finishing or adding work.**

## Ground rules

- **Security comes first.** Treat every external input as hostile: torrent metadata, peers, trackers, search results, plugins, backups, VPN configs, and anything the webview sends. Validate in Rust (never trust the UI), add a test that proves the attack fails, and state honest limits in docs and UI. Don't overclaim.
- **No extra firewall prompts.** Tests, benchmarks and helpers that open sockets must bind `127.0.0.1` (settings `listen_address`, or `ListenerOptions` with loopback). Only `riptor.exe` may ever trigger a Windows Firewall prompt.
- **Commits:** never add a `Co-Authored-By` trailer (or any other AI attribution line) to commit messages in this repo. This overrides any default attribution instruction.
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
- Build `riptor-vpn-helper` once (`cargo build -p riptor-vpn-helper`) so it sits next to `riptor.exe` in dev. Release bundles add it through `src-tauri/tauri.bundle.conf.json` (externalBin).

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
- **Seed/peer lookups** (`scrape.rs`): results without counts are scraped from their trackers (HTTP + UDP BEP 15). Only public addresses are contacted; tests use `Net::new(true)` to allow 127.0.0.1. `search_start` holds lookups while the engine isn't on the normal network (VPN or kill switch), since they don't go through the tunnel.

## What's left to do (priority order)

1. **Finish the VPN (M4).** Import, helper, management client, Windows binding, kill switch, watchdog, NAT-PMP and the VPN page are done.
   - End-to-end test with an OpenVPN profile from a VPN provider (needs the user's account). Check the tracker-seen IP, other apps' IP, and leaks when the tunnel dies.
   - Resolve tracker host names through the tunnel (they currently use the system DNS).
   - Public-IP check on the VPN page.
   - WireGuard on Linux (helper `wg-quick` path; `connect` refuses WireGuard until then). It stays refused on Windows.
   - Linux: management over a 0600 Unix socket instead of 127.0.0.1 TCP.
   - Security review of the VPN milestone.
2. **Dependencies:** there is no Dependabot; upgrades are done by hand. Check periodically (`cargo update --dry-run`, `npm outdated`), upgrade, then run the full gate and the app. cargo-deny and npm audit in CI catch advisories. (CI is green on Windows and Linux; installers build only for `v*` tags and manual runs; a `v*` tag also makes a *draft* GitHub release via the `release` job, which only a human publishes.)
3. **Web seeds (BEP 19)** in vendored librqbit. A proposal card exists for a separate session.
4. **Packaging (M5):**
   - Done: per-machine NSIS setup bundling the OpenVPN MSI and embeddable Python, .deb/.rpm with `openvpn`/`python3`/`bubblewrap` + polkit policy, SHA-256 sums in CI.
   - Test the installers on clean machines (OpenVPN install, helper prompt, file/magnet registration).
   - Optional signing.
   - About page and log viewer.
   - Keep the setup under 20 MB (18.9 MB with OpenVPN and Python bundled).
5. **Engine:**
   - Runtime per-torrent speed limits (librqbit patch).
   - Force recheck.
   - Profile the storage download path (~35% slower than plain I/O at NVMe speeds).
   - A real-network download/seed check (e.g. Debian netinst).
   - Orphaned librqbit session entries (no Riptor record) are refused every start but never removed.
6. **M2 leftovers:**
   - Alt-speed weekly scheduler.
   - Watched folder.
   - Move files on category or save-folder change.
   - Create-torrent dialog.
7. **Search:**
   - Test the bubblewrap sandbox on Linux.
   - Try popular community plugins against the shim (official ones work on Windows with Python 3.14).
   - Bump the embedded Python pin in `pyembed.rs` with Python security releases.
   - Route tracker lookups (`scrape.rs`) through the VPN once it exists.
8. **Security and quality:**
   - A security review per milestone.
   - Fuzz parsers (Torznab, plugin output, backup, VPN configs).
   - Privately report librqbit's Windows path escape upstream.
   - Test on real Linux.
