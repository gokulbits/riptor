# Vendored dependencies

## librqbit 9.0.1

Copied unmodified from crates.io (`librqbit-9.0.1`), plus the patches below.
Every change is marked in the source with a `RIPTOR PATCH` comment. Re-apply them
when upgrading, and try to upstream each one.

| File | Change | Why | Upstream |
| --- | --- | --- | --- |
| `src/session_persistence/json.rs` | Lift the "only FilesystemStorageFactory can be persisted" check. | Riptor installs `CachedStorageFactory` as the session's *default* factory, so restored torrents use it correctly. Without persistence there is no fast-resume, and every restart would re-hash every torrent. | Not yet: propose "allow when the factory is the session default". |

### Security findings to report upstream

* **Windows path escape:** `FileIteratorName::to_pathbuf()` builds paths with `PathBuf::push`. A torrent path component with a drive prefix (for example `C:x`) *replaces* the whole path on Windows, so `FilesystemStorage` can write outside the output folder. Riptor is not affected: `riptor-storage` rebuilds every path from the raw components (`crates/riptor-storage/src/sanitize.rs`). Report this privately to the maintainer before it is mentioned publicly.
* `Session::delete` with `delete_files` calls `remove_directory_if_empty("")` on the torrent's output folder whenever that folder isn't the session default. That can remove a user-chosen, empty save folder. Riptor's storage only removes folders it created.

## librqbit-dualstack-sockets 0.7.0

Copied unmodified from crates.io, in preparation for the VPN kill switch. It
is **not wired into the build yet**: there is no `[patch.crates-io]` entry, so
the crates.io version is still used.

| File | Change | Why | Upstream |
| --- | --- | --- | --- |
| `src/bind_device.rs` | *(planned)* Windows `BindDevice`: resolve the adapter by name/index, and pin sockets with `IP_UNICAST_IF` / `IPV6_UNICAST_IF`. | `bind_device_name` is Linux/macOS only. On Windows it errors, so torrent traffic can't be tied to the VPN adapter. | Not yet. |
