"""Cloud storage backends for metadata (S3-compatible + GCS).

Uses raw httpx — no SDK dependencies. Auth is detected from the URL:
- *.s3.amazonaws.com, *.r2.cloudflarestorage.com, or anything else → AWS Sig V4
- storage.googleapis.com → GCS Bearer token

Object paths mirror the local cache layout:
  {base_url}/{host}/{owner}/{repo}/{revision}.json
"""

from __future__ import annotations

import datetime
import hashlib
import hmac
import json
import os
import re
from typing import Any
from urllib.parse import urlparse

import httpx

from gitscale.api import _extract_hostname, extract_owner_repo


class StorageError(Exception):
    """Raised when a cloud storage operation fails."""


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def upload_metadata(
    storage_url: str,
    repo_url: str,
    revision: str,
    data: dict[str, Any],
) -> str:
    """Upload metadata JSON to cloud storage.

    Returns the full object URL.
    """
    object_url = _object_url(storage_url, repo_url, revision)
    body = json.dumps(data, indent=2).encode()
    _put_object(object_url, body)
    return object_url


def download_metadata(
    storage_url: str,
    repo_url: str,
    revision: str,
) -> dict[str, Any] | None:
    """Download metadata JSON from cloud storage.

    Returns None if the object doesn't exist (404).
    """
    object_url = _object_url(storage_url, repo_url, revision)
    body = _get_object(object_url)
    if body is None:
        return None
    result: dict[str, Any] = json.loads(body)
    return result


def _object_url(
    storage_url: str, repo_url: str, revision: str
) -> str:
    """Build the full object URL for a given repo + revision."""
    hostname = _extract_hostname(repo_url)
    owner, repo = extract_owner_repo(repo_url)
    safe_rev = revision.replace("/", "_") if revision else "_default"
    base = storage_url.rstrip("/")
    return f"{base}/{hostname}/{owner}/{repo}/{safe_rev}.json"


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
# GET / PUT dispatch
# ---------------------------------------------------------------------------


def _get_object(url: str) -> bytes | None:
    """GET an object. Returns None on 404."""
    if _is_gcs(url):
        return _gcs_get(url)
    return _s3_get(url)


def _put_object(url: str, body: bytes) -> None:
    """PUT an object."""
    if _is_gcs(url):
        _gcs_put(url, body)
    else:
        _s3_put(url, body)


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


def _gcs_get(url: str) -> bytes | None:
    token = _gcs_token()
    resp = httpx.get(
        url,
        headers={"Authorization": f"Bearer {token}"},
        timeout=30,
    )
    if resp.status_code == 404:
        return None
    _check_storage_response(resp, "GET")
    return resp.content


def _gcs_put(url: str, body: bytes) -> None:
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


def _s3_get(url: str) -> bytes | None:
    access_key, secret_key = _s3_credentials()
    region = _s3_region(url)
    headers = _s3_sign("GET", url, b"", access_key, secret_key, region)

    resp = httpx.get(url, headers=headers, timeout=30)
    if resp.status_code == 404:
        return None
    _check_storage_response(resp, "GET")
    return resp.content


def _s3_put(url: str, body: bytes) -> None:
    access_key, secret_key = _s3_credentials()
    region = _s3_region(url)
    headers = _s3_sign("PUT", url, body, access_key, secret_key, region)
    headers["Content-Type"] = "application/json"

    resp = httpx.put(url, content=body, headers=headers, timeout=30)
    _check_storage_response(resp, "PUT")


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
