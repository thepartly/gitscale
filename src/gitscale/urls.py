"""URL parsing utilities for git repository URLs."""

import re
from urllib.parse import urlparse


class UrlError(Exception):
    """Raised when a URL cannot be parsed."""


def _extract_hostname(repo_url: str) -> str:
    """Extract hostname from git URL (HTTPS or SSH)."""
    # SSH: git@hostname:owner/repo.git
    ssh_match = re.match(r"^[\w-]+@([\w.\-]+):", repo_url)
    if ssh_match:
        return ssh_match.group(1)

    # HTTPS or other scheme
    parsed = urlparse(repo_url)
    if parsed.hostname:
        return parsed.hostname

    raise UrlError(f"Cannot extract hostname from URL: {repo_url}")


def extract_owner_repo(repo_url: str) -> tuple[str, str]:
    """Extract owner and repo name from a git URL."""
    # SSH: git@hostname:owner/repo.git
    ssh_match = re.match(r"^[\w-]+@[\w.\-]+:(.*)", repo_url)
    path = ssh_match.group(1) if ssh_match else urlparse(repo_url).path

    # Strip leading slash and .git suffix
    path = path.strip("/")
    if path.endswith(".git"):
        path = path[:-4]

    parts = path.split("/", 1)
    if len(parts) != 2 or not parts[0] or not parts[1]:
        raise UrlError(
            f"Cannot extract owner/repo from URL: {repo_url}"
        )
    return parts[0], parts[1]
