"""E2E test fixtures — MinIO-backed S3 storage.

Provides:
- ``s3_env``              – sets AWS env vars pointing at local MinIO
- ``minio_storage_url``   – creates a unique bucket per test session and
                            returns its base URL ready for ``storage.py``
"""

from __future__ import annotations

import os
import uuid

import httpx
import pytest

MINIO_ENDPOINT = os.environ.get("MINIO_ENDPOINT", "minio:9000")
MINIO_ROOT_USER = "minioadmin"
MINIO_ROOT_PASSWORD = "minioadmin"
MINIO_REGION = "us-east-1"


def _minio_available() -> bool:
    """Return True if MinIO answers on the expected endpoint."""
    try:
        r = httpx.get(
            f"http://{MINIO_ENDPOINT}/minio/health/live", timeout=2
        )
        return r.status_code == 200
    except (httpx.ConnectError, httpx.TimeoutException):
        return False


def pytest_collection_modifyitems(items: list[pytest.Item]) -> None:
    """Add the ``e2e`` marker to every item in this package and skip if
    MinIO is unreachable."""
    skip = pytest.mark.skip(
        reason=f"MinIO not reachable at {MINIO_ENDPOINT}"
    )
    available = _minio_available()
    for item in items:
        item.add_marker(pytest.mark.e2e)
        if not available:
            item.add_marker(skip)


# ------------------------------------------------------------------
# Fixtures
# ------------------------------------------------------------------


@pytest.fixture(autouse=True)
def s3_env(monkeypatch: pytest.MonkeyPatch) -> None:
    """Inject MinIO credentials into the environment for ``storage.py``."""
    monkeypatch.setenv("AWS_ACCESS_KEY_ID", MINIO_ROOT_USER)
    monkeypatch.setenv("AWS_SECRET_ACCESS_KEY", MINIO_ROOT_PASSWORD)
    monkeypatch.setenv("AWS_DEFAULT_REGION", MINIO_REGION)


@pytest.fixture(scope="session")
def _session_bucket() -> str:
    """Create a unique MinIO bucket for this test session.

    Uses the S3 ``PUT /<bucket>`` API directly so we don't need ``mc``
    or ``boto3``.
    """
    bucket = f"gitscale-e2e-{uuid.uuid4().hex[:8]}"
    url = f"http://{MINIO_ENDPOINT}/{bucket}"

    from gitscale.storage import _s3_sign

    headers = _s3_sign(
        "PUT",
        url + "/",
        b"",
        MINIO_ROOT_USER,
        MINIO_ROOT_PASSWORD,
        MINIO_REGION,
    )
    resp = httpx.put(url + "/", headers=headers, timeout=10)
    # 200 = created, 409 = already exists
    assert resp.status_code in (200, 409), (
        f"Bucket creation failed ({resp.status_code}): {resp.text[:200]}"
    )
    return bucket


@pytest.fixture()
def minio_storage_url(_session_bucket: str) -> str:
    """Return the base storage URL for the session bucket.

    Example: ``http://localhost:9000/gitscale-e2e-a1b2c3d4``
    """
    return f"http://{MINIO_ENDPOINT}/{_session_bucket}"
