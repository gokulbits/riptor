"""nova3 printer API. Prints JSON lines, so a '|' or newline in a name can't
corrupt the output.
"""

import json
import re
import sys
from typing import Any, Dict

# Newer plugins import this type; any dict works.
SearchResults = Dict[str, Any]

_UNITS = {"B": 0, "KB": 1, "KIB": 1, "MB": 2, "MIB": 2, "GB": 3, "GIB": 3, "TB": 4, "TIB": 4, "PB": 5, "PIB": 5}
_SIZE_RE = re.compile(r"^\s*([0-9]+(?:[.,][0-9]+)?)\s*([a-zA-Z]*)\s*$")


def anySizeToBytes(size_string):
    """'1.5 GB', '700 MiB', '12345' -> bytes; -1 if unknown."""
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
    """Counts and dates: 12, "12", " 1,234 ", 12.0; -1 or junk means unknown."""
    try:
        s = str(v).strip().replace(",", "").replace("_", "")
        n = int(float(s)) if "." in s else int(s)
        return n if n >= 0 else None
    except (TypeError, ValueError, OverflowError):
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
