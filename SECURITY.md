# Security

Riptor handles untrusted input from the network all the time: torrent metadata, peer messages, tracker and search responses. It can also run privileged VPN tooling. Security is a design requirement, not an afterthought.

## Reporting a vulnerability

Please do **not** open a public issue. Use GitHub's private vulnerability reporting ("Report a vulnerability" on the Security tab). You should get a first response within 7 days.

## Threat model (summary)

| Untrusted input | Mitigation |
| --- | --- |
| File names inside `.torrent` / magnet metadata | Every path is rebuilt by `riptor-storage`'s sanitizer. It handles separators, `..`, drive prefixes, reserved device names, alternate data streams, trailing dots and spaces, and length caps. A path must stay under the save folder or it is refused. |
| Downloaded content | Never opened or executed automatically. On Windows, completed files get a Mark-of-the-Web. The add dialog warns when a torrent contains executables. |
| `.torrent` files, URLs, watched folder | Size caps. Only http/https, with timeouts. Riptor's patched bencode parser caps nesting depth and rejects out-of-range numbers, so a crafted file or peer message can't crash the app. Fuzz-style tests cover torrents, bencode and backups. |
| Web seeds (`url-list` in a torrent) | http/https only, public addresses only (checked after DNS and on each redirect), exact byte ranges, pieces up to 16 MiB. Every piece is hash-checked like peer data, and a server that keeps failing or sends bad data is dropped. Skipped while a proxy or anonymous mode is in use. |
| Peers, trackers, DHT | Handled by librqbit, written in memory-safe Rust. IP blocklist supported. |
| Search plugins (nova3 `.py`) | Plugins are **code**. Installs need explicit confirmation and show the source and SHA-256, also when picked from the catalog (official or community lists fetched from GitHub; entries are stripped, capped, and must link to an https `.py` file). They run in a separate, non-elevated, time- and output-limited process (bubblewrap on Linux when available). Output is strictly parsed. |
| Riptor's own Python (Windows, opt-in) | python.org's embeddable package, fetched over https and rejected unless its SHA-256 matches the pinned value. Unpacked with path, link, file-count and size checks into Riptor's data folder; no admin rights. |
| Tracker scrapes for search results | The result picks the trackers and `.torrent` URL, so loopback, private, link-local and other non-public addresses are refused after DNS resolution and on redirects. Replies are size-capped, time-limited and parsed by librqbit's strict bencode parser; work per search is capped. Scraping reveals the looked-up torrents to those trackers and can be turned off. |
| Torznab responses | `quick-xml`, which never resolves external entities. Size caps. Mutation tests cover Torznab, Internet Archive replies, plugin results and scrapes. |
| VPN profiles (`.ovpn`, WireGuard `.conf`) | Script and plugin directives (`up`, `down`, `plugin`, `PostUp`, …) are rejected, because they would run as admin or root. The privileged helper re-validates every profile and takes only a fixed set of requests. |
| Deep links (`magnet:`) | Always open the add dialog. Nothing is added silently. |

## Hardening

* No Web UI or HTTP API server, so nothing listens except the BitTorrent ports.
* Tauri: strict CSP, minimal capabilities, no shell plugin, and validated command inputs.
* Secrets (OpenVPN credentials, Torznab API keys) are stored in the OS keyring and redacted from logs.
* `unsafe` is forbidden outside small, documented OS-call modules.
* CI runs `cargo deny` (advisories, licenses, sources) and `npm audit`. Actions are pinned to commit SHAs. Dependencies are updated by hand, and each upgrade is built, tested and audited before it lands, rather than auto-merged.

## Known limitations

* librqbit has no MSE/PE peer encryption, so peer traffic is not obfuscated. Use the VPN integration if your ISP throttles BitTorrent.
