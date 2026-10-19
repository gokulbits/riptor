"""Riptor's search-plugin runner (nova3 plugin compatible).

  nova2.py capabilities              one JSON line per installed engine
  nova2.py search ENGINE CAT WORDS   one JSON line per result
  nova2.py download ENGINE URL       prints 'path url' for a .torrent file

Riptor starts Python with -I (isolated mode), so this file adds only its own
folder (which holds just the shim modules) to sys.path. Engines are loaded by
file path and never put on sys.path, so an engine named e.g. "json.py" cannot
shadow a standard-library module for other engines.
"""

import importlib.util
import json
import os
import re
import sys
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
ENGINES = os.path.join(HERE, "engines")
sys.path.insert(0, HERE)

CATEGORIES = ("all", "anime", "books", "games", "movies", "music", "pictures", "software", "tv")
_NAME = re.compile(r"^[a-z0-9_]{1,64}$")


def load(engine_id):
    if not _NAME.match(engine_id):
        raise ValueError("invalid engine name")
    path = os.path.join(ENGINES, engine_id + ".py")
    spec = importlib.util.spec_from_file_location("riptor_engine_" + engine_id, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return getattr(module, engine_id)()


def capabilities():
    for fname in sorted(os.listdir(ENGINES)):
        engine_id, ext = os.path.splitext(fname)
        if ext != ".py" or not _NAME.match(engine_id):
            continue
        try:
            engine = load(engine_id)
            cats = [c for c in getattr(engine, "supported_categories", {"all": ""}) if c in CATEGORIES]
            info = {"id": engine_id, "name": str(engine.name), "url": str(engine.url), "categories": cats}
        except BaseException as e:  # one broken plugin must not hide the others
            info = {"id": engine_id, "error": "%s: %s" % (type(e).__name__, e)}
        sys.stdout.write(json.dumps(info) + "\n")


def search(engine_id, category, words):
    engine = load(engine_id)
    supported = getattr(engine, "supported_categories", {"all": ""})
    if category not in supported:
        return
    query = "+".join(urllib.parse.quote(w, safe="") for w in words)
    engine.search(query, category)


def download(engine_id, url):
    engine = load(engine_id)
    if hasattr(engine, "download_torrent"):
        engine.download_torrent(url)
    else:
        from helpers import download_file

        print(download_file(url))


def main(argv):
    if len(argv) >= 1 and argv[0] == "capabilities":
        capabilities()
    elif len(argv) >= 4 and argv[0] == "search" and argv[2] in CATEGORIES:
        search(argv[1], argv[2], argv[3:])
    elif len(argv) == 3 and argv[0] == "download":
        download(argv[1], argv[2])
    else:
        sys.stderr.write(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
