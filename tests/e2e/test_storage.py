"""E2E tests for the storage module against a real MinIO instance."""

from __future__ import annotations

from gitscale.storage import download_metadata, upload_metadata


# ------------------------------------------------------------------
# upload / download round-trip
# ------------------------------------------------------------------


class TestStorageRoundTrip:
    """Verify upload_metadata + download_metadata against live MinIO."""

    REPO_URL = "https://github.com/acme/widgets.git"

    def test_upload_then_download(self, minio_storage_url: str) -> None:
        payload = {"build": "ok", "commit": "abc123"}
        url = upload_metadata(
            minio_storage_url, self.REPO_URL, "main", payload
        )
        assert url.endswith("/github.com/acme/widgets/main.json")

        result = download_metadata(
            minio_storage_url, self.REPO_URL, "main"
        )
        assert result == payload

    def test_download_nonexistent_returns_none(
        self, minio_storage_url: str
    ) -> None:
        result = download_metadata(
            minio_storage_url, self.REPO_URL, "no-such-rev"
        )
        assert result is None

    def test_overwrite_replaces_data(self, minio_storage_url: str) -> None:
        upload_metadata(
            minio_storage_url, self.REPO_URL, "v1", {"version": 1}
        )
        upload_metadata(
            minio_storage_url, self.REPO_URL, "v1", {"version": 2}
        )
        result = download_metadata(
            minio_storage_url, self.REPO_URL, "v1"
        )
        assert result == {"version": 2}

    def test_slash_in_revision(self, minio_storage_url: str) -> None:
        upload_metadata(
            minio_storage_url,
            self.REPO_URL,
            "feature/login",
            {"branch": True},
        )
        result = download_metadata(
            minio_storage_url, self.REPO_URL, "feature/login"
        )
        assert result == {"branch": True}

    def test_multiple_repos_isolated(self, minio_storage_url: str) -> None:
        repo_a = "https://github.com/acme/alpha.git"
        repo_b = "https://github.com/acme/beta.git"
        upload_metadata(minio_storage_url, repo_a, "main", {"repo": "a"})
        upload_metadata(minio_storage_url, repo_b, "main", {"repo": "b"})

        assert download_metadata(minio_storage_url, repo_a, "main") == {
            "repo": "a"
        }
        assert download_metadata(minio_storage_url, repo_b, "main") == {
            "repo": "b"
        }

    def test_large_payload(self, minio_storage_url: str) -> None:
        payload = {f"key_{i}": f"value_{i}" for i in range(500)}
        upload_metadata(
            minio_storage_url, self.REPO_URL, "big", payload
        )
        result = download_metadata(
            minio_storage_url, self.REPO_URL, "big"
        )
        assert result == payload
