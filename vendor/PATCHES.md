# Vendored dependencies

## librqbit 9.0.1

Copied unmodified from crates.io (`librqbit-9.0.1`), plus the patches below.
Every change is marked in the source with a `RIPTOR PATCH` comment. Re-apply them
when upgrading, and try to upstream each one.

| File | Change | Why | Upstream |
| --- | --- | --- | --- |
| `src/session_persistence/json.rs` | Lift the "only FilesystemStorageFactory can be persisted" check. | Riptor installs `CachedStorageFactory` as the session's *default* factory, so restored torrents use it correctly. Without persistence there is no fast-resume, and every restart would re-hash every torrent. | Not yet: propose "allow when the factory is the session default". |
| `src/torrent_state/live/mod.rs` | `TorrentStateLive::ratelimits()` returns the torrent's own `Limits`. | Per-torrent speed limits that change at runtime. librqbit only set them from the add options; Riptor now applies each torrent's limits to its live limiter every tick. | Not yet; trivial to upstream. |
| `src/torrent_state/live/mod.rs`, `src/lib.rs` | Web-seed hooks on `TorrentStateLive`: `webseed_acquire` (reserve the next needed piece under a pseudo-peer handle, never stealing), `webseed_piece` (write a whole piece through the normal chunk path, hash-check it, flush it, then count and announce it), `webseed_release`, `webseed_done`; `TorrentStateLive` is re-exported. | librqbit has no BEP 19 web seeds. Riptor fetches pieces over HTTP (`riptor-core/src/webseed.rs`) and hands them over here, so they're verified and stored exactly like peer data. | Not yet; upstream would want a native web-seed peer type. |
| `src/torrent_state/live/mod.rs` | `TorrentStateLive::set_file_priority_levels` reorders the file list pieces are picked from. | Per-file High/Normal/Low priority. librqbit already downloads file by file in name order; this puts higher-priority files first. | Not yet; upstream has a TODO to make it configurable. |
| `src/torrent_state/mod.rs`, `src/session.rs`, `src/session_persistence/json.rs` | `ManagedTorrent::trackers()` / `set_trackers()`: a runtime tracker list used when the torrent next starts and saved with the session. | Editing a torrent's trackers without re-adding (and rechecking) it. | Not yet. |
| `src/torrent_state/mod.rs` | `ManagedTorrent::have_pieces()` for live and paused torrents. | The piece map, and knowing how much of a file's start is on disk for play-while-downloading. | Not yet; trivial. |
| `src/torrent_state/live/mod.rs`, `src/session.rs` | `TorrentStateLive::add_peer` (public wrapper) and `drop_peers_where`; `Session::set_banned` / `is_banned`, checked next to the blocklist for incoming and outgoing peers. | Adding a peer by address and banning peers by hand. | Not yet. |
| `src/session.rs` | `SessionOptions::blocklist_file`, loaded with the existing `IpRanges::load_from_file`. | Blocklists from a local file as well as a URL. | Not yet; trivial. |
| `src/session.rs` | New `SessionOptions::dns_resolver`, used for the HTTP(S) tracker client. | On the VPN, tracker hostnames are resolved through the tunnel (see `riptor-core/src/dns.rs`), not by the system resolver. | Not yet. |
| `src/session.rs` | New `SessionOptions::bind_local_ip`, applied as reqwest's `local_address` for HTTP(S) trackers. | reqwest can't bind to an interface on Windows. Pinning the source address to the VPN adapter's IP keeps tracker requests inside the tunnel, and makes them fail rather than leak if it drops. | Not yet. |
| `src/mse.rs` (new), `src/peer_connection.rs`, `src/session.rs`, `src/stream_connect.rs`, `src/lib.rs`, `Cargo.toml` | MSE/PE peer encryption: the handshake for both sides, RC4 reader/writer wrappers, `ConnectionOptions::encryption` (`EncryptionPolicy`: disabled, prefer, require), outgoing in `manage_peer_outgoing` with a plain reconnect under prefer and a bounded plain-only memory, incoming in `check_incoming_connection`. Adds `num-bigint` and `rc4`. | librqbit has no protocol encryption; some networks throttle plain BitTorrent and some peers require it. | Not yet; would need upstream's design for the option. |
| `src/read_buf.rs` | `read_handshake` keeps reading, under one deadline, until the whole 68-byte handshake is in. | It assumed one read returns all of it, which encrypted streams (and slow links) don't guarantee. | Not yet; a plain robustness fix. |
| `src/riptor_observe.rs`, `src/peer_connection.rs`, `src/lib.rs` | A process-wide observer told each peer's `yourip` from its extended handshake (BEP 10). | Riptor's swarm leak check compares the address peers see with the VPN's (`riptor-core/src/seen_ip.rs`). | Not yet; upstream might prefer it in per-peer stats. |

### Security findings to report upstream

* **Windows path escape:** `FileIteratorName::to_pathbuf()` builds paths with `PathBuf::push`. A torrent path component with a drive prefix (for example `C:x`) *replaces* the whole path on Windows, so `FilesystemStorage` can write outside the output folder. Riptor is not affected: `riptor-storage` rebuilds every path from the raw components (`crates/riptor-storage/src/sanitize.rs`). Report this privately to the maintainer before it is mentioned publicly.
* `Session::delete` with `delete_files` calls `remove_directory_if_empty("")` on the torrent's output folder whenever that folder isn't the session default. That can remove a user-chosen, empty save folder. Riptor's storage only removes folders it created.

## librqbit-tracker-comms 9.0.1 and librqbit-dht 9.0.1

Copied from crates.io and wired in via `[patch.crates-io]`.

| File | Change | Why | Upstream |
| --- | --- | --- | --- |
| `src/resolve_hook.rs` (both), `src/lib.rs` | An optional process-wide resolver (`set_resolver`). Unset, lookups use `tokio::net::lookup_host` as before. | Riptor installs its tunnel resolver while the VPN is up, so UDP tracker hostnames and DHT bootstrap hosts are looked up through the tunnel instead of the system DNS. | Not yet; a resolver option on the session would be cleaner upstream. |
| `tracker-comms/src/tracker_comms.rs`, `dht/src/dht.rs` | The two `lookup_host` calls go through `resolve_hook::lookup`. | Same. | Same. |
| `tracker-comms/src/observe.rs`, `tracker_comms.rs`, `tracker_comms_http.rs`, `lib.rs` | A process-wide observer told about every announce: host, success, seeders and leechers (HTTP and UDP), and the BEP 24 `external ip`. HTTP `complete`/`incomplete` became optional so "not sent" isn't read as zero. | Seeding where the swarm needs it, the swarm leak check and the connections ledger. | Not yet; a per-torrent stats channel would be cleaner upstream. |

## librqbit-bencode 9.0.1

Copied from crates.io and wired in via `[patch.crates-io]`. Found by `crates/riptor-core/tests/hostile_input.rs`.

| File | Change | Why | Upstream |
| --- | --- | --- | --- |
| `src/deserialize.rs` | Lists and dicts nest at most 64 deep (`Error::TooDeep`). | Deserializing recurses per level. About 40 KB of `llll...` in a `.torrent`, or in a peer's extended message, overflowed the stack and aborted the process. | Not yet; report privately first. |
| `src/deserialize.rs` | Integers and string lengths use atoi's checked parsers. | Long digit strings panicked in debug builds and wrapped silently in release. | Not yet. |

## librqbit-dualstack-sockets 0.7.0

Copied from crates.io and wired in via `[patch.crates-io]`, with the patch below. It is the
basis of Riptor's app-only VPN: only Riptor's sockets are pinned to the tunnel adapter, so
the rest of the system keeps its normal connection.

| File | Change | Why | Upstream |
| --- | --- | --- | --- |
| `src/bind_device.rs`, `Cargo.toml` | Windows `BindDevice`: resolve the adapter by friendly name or `#index` (via `network-interface`), and pin sockets with `IP_UNICAST_IF` / `IPV6_UNICAST_IF` (`windows-sys`). | `bind_device_name` was Linux/macOS only. On Windows it errored, so torrent traffic couldn't be tied to the VPN adapter. Pinned sockets fail instead of leaking if the adapter disappears. Tested in `crates/riptor-core/tests/bind_device.rs`. | Not yet; a good upstream candidate. |
