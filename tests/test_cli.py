"""Tests for the CLI entry point and config parsing."""

from pathlib import Path

from click.testing import CliRunner

from gitscale.cli import cli
from gitscale.config import (
    ConfigError,
    RepoEntry,
    RepoMode,
    load_config,
    parse_config,
    write_config,
)

# ---------------------------------------------------------------------------
# CLI help / version
# ---------------------------------------------------------------------------


def test_cli_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--help"])
    assert result.exit_code == 0
    assert "GitScale" in result.output


def test_cli_version() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    assert result.exit_code == 0
    assert "0.1.0" in result.output


def test_clone_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["clone", "--help"])
    assert result.exit_code == 0
    assert "Clone sub-repositories" in result.output


def test_status_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["status", "--help"])
    assert result.exit_code == 0
    assert "Show status" in result.output


def test_sync_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["sync", "--help"])
    assert result.exit_code == 0
    assert "Full sync" in result.output


def test_fetch_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["fetch", "--help"])
    assert result.exit_code == 0
    assert "Fetch latest" in result.output


def test_pull_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["pull", "--help"])
    assert result.exit_code == 0
    assert "Pull latest" in result.output


def test_push_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["push", "--help"])
    assert result.exit_code == 0
    assert "Push local" in result.output


def test_add_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["add", "--help"])
    assert result.exit_code == 0
    assert "Add a sub-repository" in result.output


# ---------------------------------------------------------------------------
# Config parsing (TOML format)
# ---------------------------------------------------------------------------


def test_parse_config_basic(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    cfg.write_text(
        '[repos]\n'
        '"libs/core" = { url = "git@github.com:org/core.git", '
        'revision = "main", mode = "readonly" }\n'
        '"libs/utils" = { url = "https://github.com/org/utils.git", '
        'revision = "v2.1.0" }\n'
    )
    entries = parse_config(cfg)
    assert len(entries) == 2

    assert entries[0].directory == "libs/core"
    assert entries[0].repo_url == "git@github.com:org/core.git"
    assert entries[0].revision == "main"
    assert entries[0].mode == RepoMode.READONLY

    assert entries[1].directory == "libs/utils"
    assert entries[1].revision == "v2.1.0"
    assert entries[1].mode == RepoMode.READWRITE


def test_parse_config_default_mode(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    cfg.write_text(
        '[repos]\n'
        '"vendor/lib" = { url = "https://example.com/lib.git", '
        'revision = "main" }\n'
    )
    entries = parse_config(cfg)
    assert len(entries) == 1
    assert entries[0].mode == RepoMode.READWRITE


def test_parse_config_artefact_mode(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    cfg.write_text(
        '[repos]\n'
        '"meta/svc" = { url = "https://github.com/org/svc.git", '
        'revision = "main", mode = "artefact" }\n'
    )
    entries = parse_config(cfg)
    assert len(entries) == 1
    assert entries[0].mode == RepoMode.ARTEFACT
    assert entries[0].is_artefact
    assert entries[0].is_readonly


def test_parse_config_missing_url(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    cfg.write_text(
        '[repos]\n'
        '"bad" = { revision = "main" }\n'
    )
    import pytest

    with pytest.raises(ConfigError, match="url is required"):
        parse_config(cfg)


def test_parse_config_bad_mode(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    cfg.write_text(
        '[repos]\n'
        '"dir" = { url = "https://x.git", revision = "main", '
        'mode = "badmode" }\n'
    )
    import pytest

    with pytest.raises(ConfigError, match="invalid mode"):
        parse_config(cfg)


# ---------------------------------------------------------------------------
# Config writing
# ---------------------------------------------------------------------------


def test_write_config_roundtrip(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    entries = [
        RepoEntry("libs/a", "https://a.git", "main", RepoMode.READONLY),
        RepoEntry("libs/b", "https://b.git", "v1.0", RepoMode.READWRITE),
    ]
    write_config(cfg, entries)
    parsed = parse_config(cfg)
    assert parsed == entries


def test_write_config_artefact_roundtrip(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    entries = [
        RepoEntry(
            "meta/svc", "https://github.com/org/svc.git",
            "main", RepoMode.ARTEFACT,
        ),
    ]
    write_config(cfg, entries)
    parsed = parse_config(cfg)
    assert parsed == entries
    assert parsed[0].is_artefact


# ---------------------------------------------------------------------------
# Add command (no git needed)
# ---------------------------------------------------------------------------


def test_add_creates_config(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path) as td:
        result = runner.invoke(
            cli,
            ["add", "libs/core", "https://github.com/org/core.git", "main"],
        )
        assert result.exit_code == 0
        assert "Added libs/core" in result.output

        cfg = Path(td) / ".gitscale.toml"
        assert cfg.exists()
        entries = parse_config(cfg)
        assert len(entries) == 1
        assert entries[0].directory == "libs/core"


def test_add_duplicate_fails(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path):
        runner.invoke(
            cli,
            ["add", "libs/core", "https://github.com/org/core.git", "main"],
        )
        result = runner.invoke(
            cli,
            ["add", "libs/core", "https://github.com/org/other.git", "dev"],
        )
        assert result.exit_code != 0
        assert "already declared" in result.output


def test_add_artefact_mode(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path) as td:
        result = runner.invoke(
            cli,
            [
                "add", "meta/svc",
                "https://github.com/org/svc.git", "main",
                "--mode", "artefact",
            ],
        )
        assert result.exit_code == 0
        assert "Added meta/svc" in result.output
        assert "[artefact]" in result.output

        entries = parse_config(Path(td) / ".gitscale.toml")
        assert entries[0].mode == RepoMode.ARTEFACT


# ---------------------------------------------------------------------------
# Remove command
# ---------------------------------------------------------------------------


def test_remove_entry(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path) as td:
        runner.invoke(
            cli,
            ["add", "libs/core", "https://github.com/org/core.git", "main"],
        )
        runner.invoke(
            cli,
            ["add", "libs/utils", "https://github.com/org/utils.git", "main"],
        )
        result = runner.invoke(cli, ["remove", "libs/core"])
        assert result.exit_code == 0
        assert "Removed libs/core" in result.output

        entries = parse_config(Path(td) / ".gitscale.toml")
        assert len(entries) == 1
        assert entries[0].directory == "libs/utils"


def test_remove_nonexistent_fails(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path):
        runner.invoke(
            cli,
            ["add", "libs/core", "https://github.com/org/core.git", "main"],
        )
        result = runner.invoke(cli, ["remove", "libs/nope"])
        assert result.exit_code != 0
        assert "not declared" in result.output


# ---------------------------------------------------------------------------
# Clone / status with no config
# ---------------------------------------------------------------------------


def test_clone_no_config(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path):
        result = runner.invoke(cli, ["clone"])
        assert result.exit_code != 0
        assert "No .gitscale.toml config found" in result.output


def test_status_no_config(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path):
        result = runner.invoke(cli, ["status"])
        assert result.exit_code != 0
        assert "No .gitscale.toml config found" in result.output


# ---------------------------------------------------------------------------
# Status with config but no cloned repos
# ---------------------------------------------------------------------------


def test_status_not_cloned(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path) as td:
        cfg = Path(td) / ".gitscale.toml"
        cfg.write_text(
            '[repos]\n'
            '"libs/missing" = { url = "https://x.git", '
            'revision = "main", mode = "readonly" }\n'
        )
        result = runner.invoke(cli, ["status"])
        assert result.exit_code == 0
        assert "missed" in result.output


def test_status_json_format(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path) as td:
        cfg = Path(td) / ".gitscale.toml"
        cfg.write_text(
            '[repos]\n'
            '"libs/x" = { url = "https://x.git", '
            'revision = "main", mode = "readonly" }\n'
        )
        result = runner.invoke(cli, ["status", "--format", "json"])
        assert result.exit_code == 0
        assert '"exists": false' in result.output


# ---------------------------------------------------------------------------
# URL utility tests
# ---------------------------------------------------------------------------


def test_extract_owner_repo() -> None:
    from gitscale.urls import extract_owner_repo

    assert extract_owner_repo("https://github.com/org/repo.git") == (
        "org", "repo"
    )
    assert extract_owner_repo("git@github.com:org/repo.git") == (
        "org", "repo"
    )


def test_extract_hostname() -> None:
    from gitscale.urls import _extract_hostname

    assert _extract_hostname("https://github.com/org/repo.git") == "github.com"
    assert _extract_hostname("git@gitlab.com:org/repo.git") == "gitlab.com"


# ---------------------------------------------------------------------------
# Config: storage section
# ---------------------------------------------------------------------------


def test_parse_config_with_storage(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    cfg.write_text(
        '[storage]\n'
        'url = "https://my-bucket.s3.us-east-1.amazonaws.com/gitscale"\n\n'
        '[repos]\n'
        '"libs/a" = { url = "https://a.git", revision = "main" }\n'
    )
    config = load_config(cfg)
    assert config.storage_url == (
        "https://my-bucket.s3.us-east-1.amazonaws.com/gitscale"
    )
    assert len(config.repos) == 1


def test_parse_config_no_storage(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    cfg.write_text(
        '[repos]\n'
        '"libs/a" = { url = "https://a.git", revision = "main" }\n'
    )
    config = load_config(cfg)
    assert config.storage_url == ""


def test_write_config_preserves_storage(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    entries = [
        RepoEntry("libs/a", "https://a.git", "main", RepoMode.READONLY),
    ]
    write_config(
        cfg, entries,
        storage_url="https://bucket.s3.amazonaws.com/gs",
    )
    config = load_config(cfg)
    assert config.storage_url == "https://bucket.s3.amazonaws.com/gs"
    assert len(config.repos) == 1


# ---------------------------------------------------------------------------
# Storage module unit tests
# ---------------------------------------------------------------------------


def test_object_url_construction() -> None:
    from gitscale.storage import object_url

    url = object_url(
        "https://bucket.s3.amazonaws.com/meta",
        "https://github.com/org/repo.git",
        "main",
    )
    assert url == (
        "https://bucket.s3.amazonaws.com/meta"
        "/github.com/org/repo/main.tar.gz"
    )


def test_object_url_slash_in_revision() -> None:
    from gitscale.storage import object_url

    url = object_url(
        "https://bucket.s3.amazonaws.com/meta",
        "https://github.com/org/repo.git",
        "feature/foo",
    )
    assert url.endswith("/feature_foo.tar.gz")


def test_is_gcs() -> None:
    from gitscale.storage import _is_gcs

    assert _is_gcs("https://storage.googleapis.com/bucket/key")
    assert not _is_gcs("https://bucket.s3.amazonaws.com/key")
    assert not _is_gcs("https://minio.corp.com/bucket/key")


def test_s3_region_from_url() -> None:
    from gitscale.storage import _s3_region

    assert _s3_region(
        "https://bucket.s3.eu-west-1.amazonaws.com/key"
    ) == "eu-west-1"


def test_s3_region_default() -> None:
    import os

    from gitscale.storage import _s3_region

    # Ensure env var doesn't interfere
    old = os.environ.pop("AWS_DEFAULT_REGION", None)
    try:
        assert _s3_region("https://minio.local/bucket/key") == "us-east-1"
    finally:
        if old is not None:
            os.environ["AWS_DEFAULT_REGION"] = old


# ---------------------------------------------------------------------------
# CI detection
# ---------------------------------------------------------------------------


def test_is_ci_true() -> None:
    import os

    from gitscale.git import is_ci

    old = os.environ.pop("CI", None)
    try:
        os.environ["CI"] = "true"
        assert is_ci() is True
        os.environ["CI"] = "1"
        assert is_ci() is True
        os.environ["CI"] = "TRUE"
        assert is_ci() is True
    finally:
        if old is not None:
            os.environ["CI"] = old
        else:
            os.environ.pop("CI", None)


def test_is_ci_false() -> None:
    import os

    from gitscale.git import is_ci

    old = os.environ.pop("CI", None)
    try:
        assert is_ci() is False
        os.environ["CI"] = ""
        assert is_ci() is False
        os.environ["CI"] = "false"
        assert is_ci() is False
    finally:
        if old is not None:
            os.environ["CI"] = old
        else:
            os.environ.pop("CI", None)


# ---------------------------------------------------------------------------
# RepoStatus.is_stale default
# ---------------------------------------------------------------------------


def test_repo_status_stale_default() -> None:
    from gitscale.git import RepoStatus

    s = RepoStatus(
        directory="x", exists=True, current_ref="main",
        expected_ref="main", is_clean=True, is_detached=False,
        ahead=0, behind=0,
    )
    assert s.is_stale is False

    s2 = RepoStatus(
        directory="x", exists=True, current_ref="main",
        expected_ref="main", is_clean=True, is_detached=False,
        ahead=0, behind=0, is_stale=True,
    )
    assert s2.is_stale is True


