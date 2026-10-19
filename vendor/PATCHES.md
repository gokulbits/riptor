# Vendored dependencies

## librqbit 9.0.1

Copied unmodified from crates.io (`librqbit-9.0.1`), plus the patches below.
Every change is marked in the source with a `RIPTOR PATCH` comment. Re-apply them
when upgrading, and try to upstream each one.

| File | Change | Why | Upstream |
| --- | --- | --- | --- |
| `src/session_persistence/json.rs` | Lift the "only FilesystemStorageFactory can be persisted" check. | Riptor installs `CachedStorageFactory` as the session's *default* factory, so restored torrents use it correctly. Without persistence there is no fast-resume, and every restart would re-hash every torrent. | Not yet: propose "allow when the factory is the session default". |
| `src/session.rs` | New `SessionOptions::bind_local_ip`, applied as reqwest's `local_address` for HTTP(S) trackers. | reqwest can't bind to an interface on Windows. Pinning the source address to the VPN adapter's IP keeps tracker requests inside the tunnel, and makes them fail rather than leak if it drops. | Not yet. |

### Security findings to report upstream

* **Windows path escape:** `FileIteratorName::to_pathbuf()` builds paths with `PathBuf::push`. A torrent path component with a drive prefix (for example `C:x`) *replaces* the whole path on Windows, so `FilesystemStorage` can write outside the output folder. Riptor is not affected: `riptor-storage` rebuilds every path from the raw components (`crates/riptor-storage/src/sanitize.rs`). Report this privately to the maintainer before it is mentioned publicly.
* `Session::delete` with `delete_files` calls `remove_directory_if_empty("")` on the torrent's output folder whenever that folder isn't the session default. That can remove a user-chosen, empty save folder. Riptor's storage only removes folders it created.

## librqbit-dualstack-sockets 0.7.0

Copied from crates.io and wired in via `[patch.crates-io]`, with the patch below. It is the
basis of Riptor's app-only VPN: only Riptor's sockets are pinned to the tunnel adapter, so
the rest of the system keeps its normal connection.

| File | Change | Why | Upstream |
| --- | --- | --- | --- |
| `src/bind_device.rs`, `Cargo.toml` | Windows `BindDevice`: resolve the adapter by friendly name or `#index` (via `network-interface`), and pin sockets with `IP_UNICAST_IF` / `IPV6_UNICAST_IF` (`windows-sys`). | `bind_device_name` was Linux/macOS only. On Windows it errored, so torrent traffic couldn't be tied to the VPN adapter. Pinned sockets fail instead of leaking if the adapter disappears. Tested in `crates/riptor-core/tests/bind_device.rs`. | Not yet; a good upstream candidate. |
