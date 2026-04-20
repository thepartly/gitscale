"""Cloud storage backends for manifests (S3-compatible + GCS).

Uses raw httpx — no SDK dependencies. Auth is detected from the URL:
- *.s3.amazonaws.com, *.r2.cloudflarestorage.com, or anything else → AWS Sig V4
- storage.googleapis.com → GCS Bearer token

Object paths follow the layout:
  {base_url}/{host}/{owner}/{repo}/{revision}.json
"""

from __future__ import annotations

import datetime
import hashlib
import hmac
import json
import os
import re
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any
from urllib.parse import urlparse

import httpx

from gitscale.urls import _extract_hostname, extract_owner_repo

if TYPE_CHECKING:
    from pathlib import Path


class StorageError(Exception):
    """Raised when a cloud storage operation fails."""


@dataclass(frozen=True, slots=True)
class HeadResult:
    """Result of a HEAD request."""

    exists: bool
    etag: str


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def object_url(
    storage_url: str, repo_url: str, revision: str
) -> str:
    """Build the full object URL for a given repo + revision."""
    hostname = _extract_hostname(repo_url)
    owner, repo = extract_owner_repo(repo_url)
    safe_rev = revision.replace("/", "_") if revision else "_default"
    base = storage_url.rstrip("/")
    return f"{base}/{hostname}/{owner}/{repo}/{safe_rev}.json"


def head_object(url: str) -> HeadResult:
    """HEAD an object. Returns existence and ETag."""
    if _is_gcs(url):
        return _gcs_head(url)
    return _s3_head(url)


def get_object(url: str) -> tuple[bytes, str] | None:
    """GET an object. Returns (body, etag) or None if 404."""
    if _is_gcs(url):
        return _gcs_get(url)
    return _s3_get(url)


def put_object(url: str, body: bytes) -> str:
    """PUT an object. Returns the ETag from the response."""
    if _is_gcs(url):
        return _gcs_put(url, body)
    return _s3_put(url, body)


def upload_metadata(
    storage_url: str,
    repo_url: str,
    revision: str,
    data: dict[str, Any],
) -> str:
    """Upload metadata JSON to cloud storage.

    Returns the full object URL.
    """
    url = object_url(storage_url, repo_url, revision)
    body = json.dumps(data, indent=2).encode()
    put_object(url, body)
    return url


def download_metadata(
    storage_url: str,
    repo_url: str,
    revision: str,
) -> dict[str, Any] | None:
    """Download metadata JSON from cloud storage.

    Returns None if the object doesn't exist (404).
    """
    url = object_url(storage_url, repo_url, revision)
    result = get_object(url)
    if result is None:
        return None
    body, _etag = result
    data: dict[str, Any] = json.loads(body)
    return data


def clone_manifest(
    storage_url: str,
    repo_url: str,
    revision: str,
    dest: Path,
) -> bool:
    """Download manifest and save to dest/manifest.json with ETags.

    Only downloads if the directory doesn't exist yet.
    Returns True if found and downloaded.
    """
    url = object_url(storage_url, repo_url, revision)
    result = get_object(url)
    if result is None:
        return False
    body, etag = result
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "manifest.json").write_bytes(body)
    (dest / ".etag").write_text(etag, encoding="utf-8")
    (dest / ".etag-remote").write_text(etag, encoding="utf-8")
    _apply_manifest_readonly(dest)
    return True


def fetch_manifest(
    storage_url: str,
    repo_url: str,
    revision: str,
    dest: Path,
) -> HeadResult:
    """HEAD the remote manifest and save .etag-remote."""
    url = object_url(storage_url, repo_url, revision)
    result = head_object(url)
    if result.exists:
        dest.mkdir(parents=True, exist_ok=True)
        (dest / ".etag-remote").write_text(result.etag, encoding="utf-8")
    return result


def pull_manifest(
    storage_url: str,
    repo_url: str,
    revision: str,
    dest: Path,
) -> bool:
    """HEAD + conditional GET for a manifest.

    Downloads only if remote ETag differs from local.
    Returns True if the manifest exists remotely.
    """
    url = object_url(storage_url, repo_url, revision)
    hr = head_object(url)
    if not hr.exists:
        return False

    dest.mkdir(parents=True, exist_ok=True)
    (dest / ".etag-remote").write_text(hr.etag, encoding="utf-8")

    # Check if local is already up to date
    local_etag_file = dest / ".etag"
    if local_etag_file.is_file():
        local_etag = local_etag_file.read_text(encoding="utf-8").strip()
        if local_etag == hr.etag:
            return True  # up to date

    # Download
    result = get_object(url)
    if result is None:
        return False
    body, etag = result
    _restore_manifest_writable(dest)
    (dest / "manifest.json").write_bytes(body)
    (dest / ".etag").write_text(etag, encoding="utf-8")
    _apply_manifest_readonly(dest)
    return True


def push_manifest(
    storage_url: str,
    repo_url: str,
    revision: str,
    dest: Path,
) -> bool:
    """HEAD + conditional PUT for a manifest.

    Uploads only if remote ETag differs from local.
    Returns True if upload happened, False if already up to date.
    """
    manifest_file = dest / "manifest.json"
    if not manifest_file.is_file():
        return False

    url = object_url(storage_url, repo_url, revision)

    # Check remote
    hr = head_object(url)
    local_etag_file = dest / ".etag"
    local_etag = ""
    if local_etag_file.is_file():
        local_etag = local_etag_file.read_text(encoding="utf-8").strip()

    if hr.exists and hr.etag == local_etag:
        # Remote matches local — nothing to push
        (dest / ".etag-remote").write_text(hr.etag, encoding="utf-8")
        return False

    body = manifest_file.read_bytes()
    etag = put_object(url, body)
    (dest / ".etag").write_text(etag, encoding="utf-8")
    (dest / ".etag-remote").write_text(etag, encoding="utf-8")
    return True


def _apply_manifest_readonly(dest: Path) -> None:
    """Remove write permission from manifest files."""
    import stat

    for name in ("manifest.json",):
        fpath = dest / name
        if fpath.is_file():
            mode = fpath.stat().st_mode
            fpath.chmod(mode & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH))


def _restore_manifest_writable(dest: Path) -> None:
    """Restore write permission on manifest files before update."""
    import stat

    for name in ("manifest.json",):
        fpath = dest / name
        if fpath.is_file():
            mode = fpath.stat().st_mode
            fpath.chmod(mode | stat.S_IWUSR)


def _object_url(
    storage_url: str, repo_url: str, revision: str
) -> str:
    """Deprecated alias for object_url."""
    return object_url(storage_url, repo_url, revision)


# ---------------------------------------------------------------------------
# Backend detection
# ---------------------------------------------------------------------------


def _is_gcs(url: str) -> bool:
    """Check if the URL points to Google Cloud Storage."""
    parsed = urlparse(url)
    host = parsed.hostname or ""
    return host == "storage.googleapis.com" or host.endswith(
        ".storage.googleapis.com"
    )


# ---------------------------------------------------------------------------
# GET / PUT / HEAD dispatch — now handled by public API directly
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
# GCS (Bearer token)
# ---------------------------------------------------------------------------


def _gcs_token() -> str:
    """Resolve GCS bearer token from environment."""
    token = os.environ.get("GOOGLE_TOKEN") or os.environ.get(
        "GCLOUD_ACCESS_TOKEN"
    )
    if token:
        return token
    raise StorageError(
        "No GCS token found. Set GOOGLE_TOKEN or GCLOUD_ACCESS_TOKEN."
    )


def _gcs_head(url: str) -> HeadResult:
    token = _gcs_token()
    resp = httpx.head(
        url,
        headers={"Authorization": f"Bearer {token}"},
        timeout=30,
    )
    if resp.status_code == 404:
        return HeadResult(exists=False, etag="")
    _check_storage_response(resp, "HEAD")
    return HeadResult(
        exists=True, etag=resp.headers.get("etag", "")
    )


def _gcs_get(url: str) -> tuple[bytes, str] | None:
    token = _gcs_token()
    resp = httpx.get(
        url,
        headers={"Authorization": f"Bearer {token}"},
        timeout=30,
    )
    if resp.status_code == 404:
        return None
    _check_storage_response(resp, "GET")
    return resp.content, resp.headers.get("etag", "")


def _gcs_put(url: str, body: bytes) -> str:
    token = _gcs_token()
    resp = httpx.put(
        url,
        content=body,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        timeout=30,
    )
    _check_storage_response(resp, "PUT")
    return str(resp.headers.get("etag", ""))


# ---------------------------------------------------------------------------
# S3-compatible (AWS Signature V4)
# ---------------------------------------------------------------------------


def _s3_credentials() -> tuple[str, str]:
    """Resolve S3 access key and secret from environment."""
    access_key = os.environ.get("AWS_ACCESS_KEY_ID", "")
    secret_key = os.environ.get("AWS_SECRET_ACCESS_KEY", "")
    if not access_key or not secret_key:
        raise StorageError(
            "No S3 credentials found. "
            "Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY."
        )
    return access_key, secret_key


def _s3_region(url: str) -> str:
    """Extract region from URL or fall back to env/default."""
    env_region = os.environ.get("AWS_DEFAULT_REGION", "")
    if env_region:
        return env_region

    # Try to extract from s3.{region}.amazonaws.com
    parsed = urlparse(url)
    host = parsed.hostname or ""
    m = re.search(r"s3[.-]([a-z0-9-]+)\.amazonaws\.com", host)
    if m:
        region = m.group(1)
        if region != "amazonaws":
            return region

    return "us-east-1"


def _s3_sign(
    method: str,
    url: str,
    body: bytes,
    access_key: str,
    secret_key: str,
    region: str,
) -> dict[str, str]:
    """Sign an S3 request using AWS Signature V4.

    Returns the headers to add to the request.
    """
    parsed = urlparse(url)
    host = parsed.hostname or ""
    path = parsed.path or "/"

    now = datetime.datetime.now(datetime.UTC)
    datestamp = now.strftime("%Y%m%d")
    amzdate = now.strftime("%Y%m%dT%H%M%SZ")

    payload_hash = hashlib.sha256(body).hexdigest()

    # Canonical headers (must be sorted)
    canonical_headers = (
        f"host:{host}\n"
        f"x-amz-content-sha256:{payload_hash}\n"
        f"x-amz-date:{amzdate}\n"
    )
    signed_headers = "host;x-amz-content-sha256;x-amz-date"

    # Canonical request
    canonical_request = (
        f"{method}\n"
        f"{path}\n"
        f"\n"  # empty query string
        f"{canonical_headers}\n"
        f"{signed_headers}\n"
        f"{payload_hash}"
    )

    # String to sign
    scope = f"{datestamp}/{region}/s3/aws4_request"
    string_to_sign = (
        f"AWS4-HMAC-SHA256\n"
        f"{amzdate}\n"
        f"{scope}\n"
        f"{hashlib.sha256(canonical_request.encode()).hexdigest()}"
    )

    # Signing key
    def _sign(key: bytes, msg: str) -> bytes:
        return hmac.new(key, msg.encode(), hashlib.sha256).digest()

    k_date = _sign(f"AWS4{secret_key}".encode(), datestamp)
    k_region = _sign(k_date, region)
    k_service = _sign(k_region, "s3")
    k_signing = _sign(k_service, "aws4_request")

    signature = hmac.new(
        k_signing, string_to_sign.encode(), hashlib.sha256
    ).hexdigest()

    authorization = (
        f"AWS4-HMAC-SHA256 "
        f"Credential={access_key}/{scope}, "
        f"SignedHeaders={signed_headers}, "
        f"Signature={signature}"
    )

    return {
        "Authorization": authorization,
        "x-amz-date": amzdate,
        "x-amz-content-sha256": payload_hash,
        "Host": host,
    }


def _s3_head(url: str) -> HeadResult:
    access_key, secret_key = _s3_credentials()
    region = _s3_region(url)
    headers = _s3_sign("HEAD", url, b"", access_key, secret_key, region)

    resp = httpx.head(url, headers=headers, timeout=30)
    if resp.status_code == 404:
        return HeadResult(exists=False, etag="")
    _check_storage_response(resp, "HEAD")
    return HeadResult(
        exists=True, etag=resp.headers.get("etag", "")
    )


def _s3_get(url: str) -> tuple[bytes, str] | None:
    access_key, secret_key = _s3_credentials()
    region = _s3_region(url)
    headers = _s3_sign("GET", url, b"", access_key, secret_key, region)

    resp = httpx.get(url, headers=headers, timeout=30)
    if resp.status_code == 404:
        return None
    _check_storage_response(resp, "GET")
    return resp.content, resp.headers.get("etag", "")


def _s3_put(url: str, body: bytes) -> str:
    access_key, secret_key = _s3_credentials()
    region = _s3_region(url)
    headers = _s3_sign("PUT", url, body, access_key, secret_key, region)
    headers["Content-Type"] = "application/json"

    resp = httpx.put(url, content=body, headers=headers, timeout=30)
    _check_storage_response(resp, "PUT")
    return str(resp.headers.get("etag", ""))


# ---------------------------------------------------------------------------
# Shared
# ---------------------------------------------------------------------------


def _check_storage_response(resp: httpx.Response, method: str) -> None:
    """Raise StorageError on non-success responses."""
    if resp.status_code in (200, 201, 204):
        return
    raise StorageError(
        f"Storage {method} failed ({resp.status_code}): "
        f"{resp.text[:300]}"
    )
