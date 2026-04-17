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
    assert "Fetch and checkout" in result.output


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


def test_parse_config_metadata_mode(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    cfg.write_text(
        '[repos]\n'
        '"meta/svc" = { url = "https://github.com/org/svc.git", '
        'revision = "main", mode = "metadata" }\n'
    )
    entries = parse_config(cfg)
    assert len(entries) == 1
    assert entries[0].mode == RepoMode.METADATA
    assert entries[0].is_metadata
    assert entries[0].is_readonly


def test_parse_config_hosts(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    cfg.write_text(
        '[hosts]\n'
        '"gh.corp.com" = "github"\n'
        '"gl.internal" = "gitlab"\n\n'
        '[repos]\n'
        '"libs/a" = { url = "https://gh.corp.com/org/a.git", '
        'revision = "main" }\n'
    )
    config = load_config(cfg)
    assert config.hosts == {"gh.corp.com": "github", "gl.internal": "gitlab"}
    assert len(config.repos) == 1


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


def test_write_config_with_hosts(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    entries = [
        RepoEntry("libs/a", "https://a.git", "main", RepoMode.READONLY),
    ]
    hosts = {"gh.corp.com": "github"}
    write_config(cfg, entries, hosts=hosts)
    config = load_config(cfg)
    assert config.hosts == hosts
    assert len(config.repos) == 1


def test_write_config_metadata_roundtrip(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale.toml"
    entries = [
        RepoEntry(
            "meta/svc", "https://github.com/org/svc.git",
            "main", RepoMode.METADATA,
        ),
    ]
    write_config(cfg, entries)
    parsed = parse_config(cfg)
    assert parsed == entries
    assert parsed[0].is_metadata


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


def test_add_metadata_mode(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path) as td:
        result = runner.invoke(
            cli,
            [
                "add", "meta/svc",
                "https://github.com/org/svc.git", "main",
                "--mode", "metadata",
            ],
        )
        assert result.exit_code == 0
        assert "Added meta/svc" in result.output
        assert "[metadata]" in result.output

        entries = parse_config(Path(td) / ".gitscale.toml")
        assert entries[0].mode == RepoMode.METADATA


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
        assert "NOT CLONED" in result.output


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
# API module unit tests
# ---------------------------------------------------------------------------


def test_detect_platform_github() -> None:
    from gitscale.api import detect_platform

    platform, base = detect_platform(
        "https://github.com/org/repo.git", {}
    )
    assert platform == "github"
    assert base == "https://api.github.com"


def test_detect_platform_gitlab() -> None:
    from gitscale.api import detect_platform

    platform, base = detect_platform(
        "https://gitlab.com/org/repo.git", {}
    )
    assert platform == "gitlab"
    assert base == "https://gitlab.com/api/v4"


def test_detect_platform_custom_host() -> None:
    from gitscale.api import detect_platform

    hosts = {"gh.corp.com": "github"}
    platform, base = detect_platform(
        "https://gh.corp.com/org/repo.git", hosts
    )
    assert platform == "github"
    assert base == "https://gh.corp.com/api/v3"


def test_detect_platform_ssh() -> None:
    from gitscale.api import detect_platform

    platform, _ = detect_platform(
        "git@github.com:org/repo.git", {}
    )
    assert platform == "github"


def test_extract_owner_repo() -> None:
    from gitscale.api import extract_owner_repo

    assert extract_owner_repo("https://github.com/org/repo.git") == (
        "org", "repo"
    )
    assert extract_owner_repo("git@github.com:org/repo.git") == (
        "org", "repo"
    )


def test_detect_platform_unknown_host() -> None:
    import pytest

    from gitscale.api import ApiError, detect_platform

    with pytest.raises(ApiError, match="Cannot detect platform"):
        detect_platform("https://unknown.example.com/org/repo.git", {})
