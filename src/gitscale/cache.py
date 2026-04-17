"""Local cache for API metadata under ~/.gitscale/."""

import contextlib
import json
import os
import tempfile
from pathlib import Path
from typing import Any

from gitscale.api import _extract_hostname, extract_owner_repo

CACHE_DIR = Path.home() / ".gitscale" / "cache"


def _cache_path(repo_url: str, revision: str) -> Path:
    """Build cache path: ~/.gitscale/cache/<host>/<owner>/<repo>/<revision>.json."""
    hostname = _extract_hostname(repo_url)
    owner, repo = extract_owner_repo(repo_url)
    safe_rev = revision.replace("/", "_") if revision else "_default"
    return CACHE_DIR / hostname / owner / repo / f"{safe_rev}.json"


def read_cache(repo_url: str, revision: str) -> dict[str, Any] | None:
    """Read cached metadata. Returns None if not cached."""
    path = _cache_path(repo_url, revision)
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8")
    result: dict[str, Any] = json.loads(text)
    return result


def write_cache(
    repo_url: str, revision: str, data: dict[str, Any]
) -> Path:
    """Write metadata to cache atomically. Returns the cache path."""
    path = _cache_path(repo_url, revision)
    path.parent.mkdir(parents=True, exist_ok=True)

    # Atomic write: write to temp file, then rename
    fd, tmp = tempfile.mkstemp(
        dir=path.parent, suffix=".tmp", prefix=".gitscale_"
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
        os.rename(tmp, path)
    except BaseException:
        with contextlib.suppress(OSError):
            os.unlink(tmp)
        raise
    return path
