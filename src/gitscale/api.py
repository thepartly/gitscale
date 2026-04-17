"""Platform API client for fetching commit metadata."""

import os
import re
from typing import Any
from urllib.parse import urlparse

import httpx


class ApiError(Exception):
    """Raised when a platform API call fails."""


def detect_platform(
    repo_url: str, hosts: dict[str, str]
) -> tuple[str, str]:
    """Detect platform type and API base URL from a repo URL.

    Returns (platform, api_base_url).
    Checks explicit host overrides first, then well-known hosts.
    Raises ApiError if platform cannot be determined.
    """
    hostname = _extract_hostname(repo_url)

    # Check explicit overrides
    if hostname in hosts:
        platform = hosts[hostname]
        api_base = _api_base_for(hostname, platform)
        return platform, api_base

    # Well-known hosts
    if hostname == "github.com":
        return "github", "https://api.github.com"
    if hostname == "gitlab.com":
        return "gitlab", "https://gitlab.com/api/v4"

    raise ApiError(
        f"Cannot detect platform for host '{hostname}'. "
        f"Add it to [hosts] in .gitscale.toml, e.g.:\n"
        f'  [hosts]\n  "{hostname}" = "github"'
    )


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

    raise ApiError(f"Cannot extract hostname from URL: {repo_url}")


def _api_base_for(hostname: str, platform: str) -> str:
    """Construct API base URL for a self-hosted instance."""
    if platform == "github":
        return f"https://{hostname}/api/v3"
    if platform == "gitlab":
        return f"https://{hostname}/api/v4"
    raise ApiError(f"Unknown platform: {platform}")


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
        raise ApiError(
            f"Cannot extract owner/repo from URL: {repo_url}"
        )
    return parts[0], parts[1]


def resolve_token(
    hostname: str, platform: str
) -> str | None:
    """Resolve API token from environment.

    Checks platform-specific env vars, then falls back to
    host-specific vars.
    Returns None if no token is found (public repos may still work).
    """
    # Platform-wide env vars
    if platform == "github":
        candidates = ["GITHUB_TOKEN", "GH_TOKEN"]
    elif platform == "gitlab":
        candidates = ["GITLAB_TOKEN", "GL_TOKEN"]
    else:
        candidates = []

    # Host-specific env var: GITSCALE_TOKEN_<HOSTNAME>
    # e.g. GITSCALE_TOKEN_GH_CORP_COM
    safe_host = re.sub(r"[^A-Za-z0-9]", "_", hostname).upper()
    candidates.append(f"GITSCALE_TOKEN_{safe_host}")

    for var in candidates:
        token = os.environ.get(var)
        if token:
            return token

    return None


def fetch_metadata(
    repo_url: str,
    revision: str,
    hosts: dict[str, str],
) -> dict[str, Any]:
    """Fetch all metadata for a revision from the platform API.

    Returns the raw JSON response as a dict (platform-agnostic).
    """
    platform, api_base = detect_platform(repo_url, hosts)
    hostname = _extract_hostname(repo_url)
    token = resolve_token(hostname, platform)
    owner, repo = extract_owner_repo(repo_url)

    if platform == "github":
        return _fetch_github(api_base, owner, repo, revision, token)
    if platform == "gitlab":
        if token is None:
            raise ApiError(
                f"No API token found for {hostname} (gitlab). "
                f"GitLab requires authentication."
            )
        return _fetch_gitlab(api_base, owner, repo, revision, token)

    raise ApiError(f"Unsupported platform: {platform}")


def _fetch_github(
    api_base: str,
    owner: str,
    repo: str,
    revision: str,
    token: str | None,
) -> dict[str, Any]:
    """Fetch commit metadata from GitHub API."""
    headers: dict[str, str] = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"

    with httpx.Client(
        base_url=api_base, headers=headers, timeout=30
    ) as client:
        # Commit info
        commit_resp = client.get(
            f"/repos/{owner}/{repo}/commits/{revision}"
        )
        _check_response(commit_resp, "commit")
        commit_data: dict[str, Any] = commit_resp.json()

        # Combined status
        sha = commit_data.get("sha", revision)
        status_resp = client.get(
            f"/repos/{owner}/{repo}/commits/{sha}/status"
        )
        status_data: dict[str, Any] | None = None
        if status_resp.status_code == 200:
            status_data = status_resp.json()

        # Check runs
        checks_resp = client.get(
            f"/repos/{owner}/{repo}/commits/{sha}/check-runs"
        )
        checks_data: dict[str, Any] | None = None
        if checks_resp.status_code == 200:
            checks_data = checks_resp.json()

    result: dict[str, Any] = {
        "platform": "github",
        "owner": owner,
        "repo": repo,
        "revision": revision,
        "commit": commit_data,
    }
    if status_data is not None:
        result["status"] = status_data
    if checks_data is not None:
        result["check_runs"] = checks_data
    return result


def _fetch_gitlab(
    api_base: str,
    owner: str,
    repo: str,
    revision: str,
    token: str,
) -> dict[str, Any]:
    """Fetch commit metadata from GitLab API."""
    from urllib.parse import quote

    project_path = quote(f"{owner}/{repo}", safe="")
    headers = {"PRIVATE-TOKEN": token}

    with httpx.Client(
        base_url=api_base, headers=headers, timeout=30
    ) as client:
        # Commit info
        commit_resp = client.get(
            f"/projects/{project_path}/repository/commits/{revision}"
        )
        _check_response(commit_resp, "commit")
        commit_data: dict[str, Any] = commit_resp.json()

        # Pipeline statuses
        sha = commit_data.get("id", revision)
        pipelines_resp = client.get(
            f"/projects/{project_path}/pipelines",
            params={"sha": sha},
        )
        pipelines_data: list[Any] | None = None
        if pipelines_resp.status_code == 200:
            pipelines_data = pipelines_resp.json()

    result: dict[str, Any] = {
        "platform": "gitlab",
        "owner": owner,
        "repo": repo,
        "revision": revision,
        "commit": commit_data,
    }
    if pipelines_data is not None:
        result["pipelines"] = pipelines_data
    return result


def _check_response(resp: httpx.Response, label: str) -> None:
    """Raise ApiError if the response is not successful."""
    if resp.status_code == 401:
        raise ApiError(f"Authentication failed fetching {label} (401)")
    if resp.status_code == 403:
        raise ApiError(
            f"Access denied fetching {label} (403) — "
            f"check token permissions"
        )
    if resp.status_code == 404:
        raise ApiError(f"{label.capitalize()} not found (404)")
    if resp.status_code >= 400:
        raise ApiError(
            f"API error fetching {label}: "
            f"{resp.status_code} {resp.text[:200]}"
        )
