"""E2E tests for artefact sync via ``gitscale clone/sync`` against live MinIO."""

from __future__ import annotations

import io
import tarfile
from pathlib import Path

from click.testing import CliRunner

from gitscale.cli import cli
from gitscale.storage import object_url, put_object


def _make_tar_gz(files: dict[str, bytes]) -> bytes:
    """Create an in-memory tar.gz archive from a dict of name→content."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        for name, data in files.items():
            info = tarfile.TarInfo(name=name)
            info.size = len(data)
            tf.addfile(info, io.BytesIO(data))
    return buf.getvalue()


def _upload_artefact(
    storage_url: str, repo_url: str, revision: str, body: bytes
) -> None:
    """Upload a tar.gz artefact directly to storage."""
    url = object_url(storage_url, repo_url, revision)
    put_object(url, body)


def _write_config(root: Path, storage_url: str) -> Path:
    """Write a minimal .gitscale.toml with storage and artefact entries."""
    cfg = root / ".gitscale.toml"
    cfg.write_text(
        "[storage]\n"
        f'url = "{storage_url}"\n\n'
        "[repos]\n"
        '"libs/core" = { url = "https://github.com/org/core.git", '
        'revision = "main", mode = "artefact" }\n'
        '"libs/utils" = { url = "https://github.com/org/utils.git", '
        'revision = "v2.0", mode = "artefact" }\n'
    )
    return cfg


class TestArtefactSync:
    """Verify clone/sync pull artefact archives from MinIO to local dirs."""

    def test_clone_pulls_artefact(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        body = _make_tar_gz({"data.txt": b"hello world"})
        _upload_artefact(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            body,
        )

        result = runner.invoke(cli, ["-C", str(tmp_path), "clone"])
        assert result.exit_code == 0, result.output
        assert "ok    libs/core (artefact)" in result.output

        extracted = tmp_path / "libs" / "core" / "data.txt"
        assert extracted.exists()
        assert extracted.read_text() == "hello world"

    def test_sync_pulls_artefact(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        body_v1 = _make_tar_gz({"version.txt": b"1"})
        _upload_artefact(
            minio_storage_url,
            "https://github.com/org/utils.git",
            "v2.0",
            body_v1,
        )

        result = runner.invoke(cli, ["-C", str(tmp_path), "sync"])
        assert result.exit_code == 0, result.output
        assert "ok    libs/utils (artefact)" in result.output

        dest = tmp_path / "libs" / "utils" / "version.txt"
        assert dest.exists()
        assert dest.read_text() == "1"

        # Update and re-sync
        body_v2 = _make_tar_gz({"version.txt": b"2"})
        _upload_artefact(
            minio_storage_url,
            "https://github.com/org/utils.git",
            "v2.0",
            body_v2,
        )

        result = runner.invoke(cli, ["-C", str(tmp_path), "sync"])
        assert result.exit_code == 0, result.output
        assert (tmp_path / "libs" / "utils" / "version.txt").read_text() == "2"

    def test_clone_no_artefact_data(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        result = runner.invoke(cli, ["-C", str(tmp_path), "clone"])
        assert result.exit_code == 0, result.output
        assert "no artefact data" in result.output

    def test_clone_saves_etags(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        body = _make_tar_gz({"k.txt": b"v"})
        _upload_artefact(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            body,
        )

        runner.invoke(cli, ["-C", str(tmp_path), "clone"])
        dest = tmp_path / "libs" / "core"
        assert (dest / ".etag").exists()
        assert (dest / ".etag-remote").exists()
        etag = (dest / ".etag").read_text().strip()
        assert etag != ""
        assert (dest / ".etag-remote").read_text().strip() == etag

    def test_fetch_updates_etag_remote(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        body_v1 = _make_tar_gz({"v.txt": b"1"})
        _upload_artefact(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            body_v1,
        )
        runner.invoke(cli, ["-C", str(tmp_path), "clone"])

        # Upload a new version
        body_v2 = _make_tar_gz({"v.txt": b"2"})
        _upload_artefact(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            body_v2,
        )

        result = runner.invoke(cli, ["-C", str(tmp_path), "fetch"])
        assert result.exit_code == 0, result.output

        dest = tmp_path / "libs" / "core"
        local_etag = (dest / ".etag").read_text().strip()
        remote_etag = (dest / ".etag-remote").read_text().strip()
        # After fetch, remote should differ from local
        assert local_etag != remote_etag

    def test_pull_downloads_newer(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        body_v1 = _make_tar_gz({"v.txt": b"1"})
        _upload_artefact(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            body_v1,
        )
        runner.invoke(cli, ["-C", str(tmp_path), "clone"])

        body_v2 = _make_tar_gz({"v.txt": b"2"})
        _upload_artefact(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            body_v2,
        )

        result = runner.invoke(cli, ["-C", str(tmp_path), "pull"])
        assert result.exit_code == 0, result.output

        dest = tmp_path / "libs" / "core"
        assert (dest / "v.txt").read_text() == "2"
        # ETags should match after pull
        assert (
            (dest / ".etag").read_text().strip()
            == (dest / ".etag-remote").read_text().strip()
        )

    def test_push_skips_artefact(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        result = runner.invoke(cli, ["-C", str(tmp_path), "push"])
        assert result.exit_code == 0, result.output
        assert "skip" in result.output
        assert "artefact" in result.output
