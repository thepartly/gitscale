"""E2E tests for manifest sync via ``gitscale clone/sync`` against live MinIO."""

from __future__ import annotations

import json
from pathlib import Path

from click.testing import CliRunner

from gitscale.cli import cli
from gitscale.storage import download_metadata, upload_metadata


def _write_config(root: Path, storage_url: str) -> Path:
    """Write a minimal .gitscale.toml with storage and manifest entries."""
    cfg = root / ".gitscale.toml"
    cfg.write_text(
        "[storage]\n"
        f'url = "{storage_url}"\n\n'
        "[repos]\n"
        '"libs/core" = { url = "https://github.com/org/core.git", '
        'revision = "main", mode = "manifest" }\n'
        '"libs/utils" = { url = "https://github.com/org/utils.git", '
        'revision = "v2.0", mode = "manifest" }\n'
    )
    return cfg


class TestManifestSync:
    """Verify clone/sync pull manifest data from MinIO to local directories."""

    def test_clone_pulls_manifest(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        payload = {"build": "ok", "sha": "deadbeef"}
        upload_metadata(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            payload,
        )

        result = runner.invoke(cli, ["-C", str(tmp_path), "clone"])
        assert result.exit_code == 0, result.output
        assert "ok    libs/core (manifest)" in result.output

        manifest_file = tmp_path / "libs" / "core" / "manifest.json"
        assert manifest_file.exists()
        assert json.loads(manifest_file.read_text()) == payload

    def test_sync_pulls_manifest(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        payload_v1 = {"version": 1}
        upload_metadata(
            minio_storage_url,
            "https://github.com/org/utils.git",
            "v2.0",
            payload_v1,
        )

        result = runner.invoke(cli, ["-C", str(tmp_path), "sync"])
        assert result.exit_code == 0, result.output
        assert "ok    libs/utils (manifest)" in result.output

        manifest_file = tmp_path / "libs" / "utils" / "manifest.json"
        assert manifest_file.exists()
        assert json.loads(manifest_file.read_text()) == payload_v1

        # Update and re-sync
        payload_v2 = {"version": 2}
        upload_metadata(
            minio_storage_url,
            "https://github.com/org/utils.git",
            "v2.0",
            payload_v2,
        )

        result = runner.invoke(cli, ["-C", str(tmp_path), "sync"])
        assert result.exit_code == 0, result.output
        assert json.loads(manifest_file.read_text()) == payload_v2

    def test_clone_no_manifest_data(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        result = runner.invoke(
            cli, ["-C", str(tmp_path), "clone"]
        )
        assert result.exit_code == 0, result.output
        assert "no manifest data" in result.output

    def test_clone_saves_etags(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        upload_metadata(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            {"k": "v"},
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

        upload_metadata(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            {"v": 1},
        )
        runner.invoke(cli, ["-C", str(tmp_path), "clone"])

        # Upload a new version
        upload_metadata(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            {"v": 2},
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

        upload_metadata(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            {"v": 1},
        )
        runner.invoke(cli, ["-C", str(tmp_path), "clone"])

        upload_metadata(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
            {"v": 2},
        )

        result = runner.invoke(cli, ["-C", str(tmp_path), "pull"])
        assert result.exit_code == 0, result.output

        dest = tmp_path / "libs" / "core"
        data = json.loads((dest / "manifest.json").read_text())
        assert data == {"v": 2}
        # ETags should match after pull
        assert (
            (dest / ".etag").read_text().strip()
            == (dest / ".etag-remote").read_text().strip()
        )

    def test_push_uploads_manifest(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        # Create manifest manually
        dest = tmp_path / "libs" / "core"
        dest.mkdir(parents=True)
        (dest / "manifest.json").write_text('{"pushed": true}')

        result = runner.invoke(cli, ["-C", str(tmp_path), "push"])
        assert result.exit_code == 0, result.output
        assert "pushed" in result.output

        # Verify it was uploaded
        data = download_metadata(
            minio_storage_url,
            "https://github.com/org/core.git",
            "main",
        )
        assert data == {"pushed": True}

    def test_pull_nonexistent_fails(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        result = runner.invoke(
            cli,
            ["-C", str(tmp_path), "manifest", "pull", "libs/core"],
        )
        assert result.exit_code != 0
        assert "No manifest data found" in result.output

    def test_push_overwrite(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        v1 = tmp_path / "v1.json"
        v1.write_text(json.dumps({"v": 1}))
        runner.invoke(
            cli,
            ["-C", str(tmp_path), "manifest", "push", "libs/core", str(v1)],
        )

        v2 = tmp_path / "v2.json"
        v2.write_text(json.dumps({"v": 2}))
        runner.invoke(
            cli,
            ["-C", str(tmp_path), "manifest", "push", "libs/core", str(v2)],
        )

        pull_result = runner.invoke(
            cli,
            ["-C", str(tmp_path), "manifest", "pull", "libs/core"],
        )
        assert pull_result.exit_code == 0
        assert json.loads(pull_result.output) == {"v": 2}

    def test_push_invalid_json_fails(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        bad = tmp_path / "bad.json"
        bad.write_text("not json at all")
        result = runner.invoke(
            cli,
            [
                "-C", str(tmp_path),
                "manifest", "push", "libs/core", str(bad),
            ],
        )
        assert result.exit_code != 0
        assert "Invalid JSON" in result.output

    def test_push_unknown_entry_fails(
        self, tmp_path: Path, minio_storage_url: str
    ) -> None:
        runner = CliRunner()
        _write_config(tmp_path, minio_storage_url)

        f = tmp_path / "data.json"
        f.write_text('{"x": 1}')
        result = runner.invoke(
            cli,
            [
                "-C", str(tmp_path),
                "manifest", "push", "nonexistent", str(f),
            ],
        )
        assert result.exit_code != 0
        assert "not found" in result.output
