"""Access to the record store."""

import json
from pathlib import Path

STORE = Path("records.json")


def fetch_data(path=STORE):
    """Return every record in the store."""
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def fetch_data_cached(path=STORE, _cache={}):
    """Return the records, reading each store only once."""
    if path not in _cache:
        _cache[path] = fetch_data(path)
    return _cache[path]


def prefetch_data(paths):
    """Fill the cache for several stores."""
    for path in paths:
        fetch_data_cached(path)
