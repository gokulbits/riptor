"""Riptor's implementation of the qBittorrent search-plugin printer API.

Plugins call prettyPrinter(dict) once per result. Riptor reads one JSON object
per line from stdout, so names containing '|' or newlines cannot corrupt the
output. Written for Riptor; API-compatible with qBittorrent's novaprinter.
"""

import json
import re
import sys

_UNITS = {"B": 0, "KB": 1, "KIB": 1, "MB": 2, "MIB": 2, "GB": 3, "GIB": 3, "TB": 4, "TIB": 4, "PB": 5, "PIB": 5}
_SIZE_RE = re.compile(r"^\s*([0-9]+(?:[.,][0-9]+)?)\s*([a-zA-Z]*)\s*$")


def anySizeToBytes(size_string):
    """Convert '1.5 GB', '700 MiB', '12345' etc. to bytes; -1 if unknown."""
    if isinstance(size_string, (int, float)):
        return int(size_string) if size_string >= 0 else -1
    m = _SIZE_RE.match(str(size_string or ""))
    if not m:
        return -1
    number = float(m.group(1).replace(",", "."))
    unit = (m.group(2) or "B").upper()
    if unit not in _UNITS:
        return -1
    return int(number * (1024 ** _UNITS[unit]))


def _num(v):
    try:
        n = int(str(v).strip())
        return n if n >= 0 else None
    except (TypeError, ValueError):
        return None


def prettyPrinter(dictionary):
    d = dictionary or {}
    size = anySizeToBytes(d.get("size", -1))
    out = {
        "link": str(d.get("link", "")),
        "name": str(d.get("name", "")),
        "size": size if size >= 0 else None,
        "seeds": _num(d.get("seeds")),
        "leech": _num(d.get("leech")),
        "engine_url": str(d.get("engine_url", "")),
        "desc_link": str(d.get("desc_link", "")) or None,
        "pub_date": _num(d.get("pub_date")),
    }
    sys.stdout.write(json.dumps(out, ensure_ascii=False) + "\n")
    sys.stdout.flush()
