"""Tests for the CLI entry point and config parsing."""

from pathlib import Path

from click.testing import CliRunner

from gitscale.cli import cli
from gitscale.config import (
    ConfigError,
    RepoEntry,
    RepoMode,
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
# Config parsing
# ---------------------------------------------------------------------------


def test_parse_config_basic(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale"
    cfg.write_text(
        "libs/core git@github.com:org/core.git main readonly\n"
        "libs/utils https://github.com/org/utils.git v2.1.0 readwrite\n"
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
    cfg = tmp_path / ".gitscale"
    cfg.write_text("vendor/lib https://example.com/lib.git main\n")
    entries = parse_config(cfg)
    assert len(entries) == 1
    assert entries[0].mode == RepoMode.READWRITE


def test_parse_config_comments_and_blank(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale"
    cfg.write_text(
        "# This is a comment\n"
        "\n"
        "  # indented comment\n"
        "libs/a https://a.git main readonly\n"
        "\n"
    )
    entries = parse_config(cfg)
    assert len(entries) == 1


def test_parse_config_too_few_fields(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale"
    cfg.write_text("only_two_fields https://x.git\n")
    import pytest

    with pytest.raises(ConfigError, match="at least 3 fields"):
        parse_config(cfg)


def test_parse_config_bad_mode(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale"
    cfg.write_text("dir https://x.git main badmode\n")
    import pytest

    with pytest.raises(ConfigError, match="invalid mode"):
        parse_config(cfg)


# ---------------------------------------------------------------------------
# Config writing
# ---------------------------------------------------------------------------


def test_write_config_roundtrip(tmp_path: Path) -> None:
    cfg = tmp_path / ".gitscale"
    entries = [
        RepoEntry("libs/a", "https://a.git", "main", RepoMode.READONLY),
        RepoEntry("libs/b", "https://b.git", "v1.0", RepoMode.READWRITE),
    ]
    write_config(cfg, entries)
    parsed = parse_config(cfg)
    assert parsed == entries


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

        cfg = Path(td) / ".gitscale"
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


# ---------------------------------------------------------------------------
# Clone / status with no config
# ---------------------------------------------------------------------------


def test_clone_no_config(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path):
        result = runner.invoke(cli, ["clone"])
        assert result.exit_code != 0
        assert "No .gitscale config found" in result.output


def test_status_no_config(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path):
        result = runner.invoke(cli, ["status"])
        assert result.exit_code != 0
        assert "No .gitscale config found" in result.output


# ---------------------------------------------------------------------------
# Status with config but no cloned repos
# ---------------------------------------------------------------------------


def test_status_not_cloned(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path) as td:
        cfg = Path(td) / ".gitscale"
        cfg.write_text("libs/missing https://x.git main readonly\n")
        result = runner.invoke(cli, ["status"])
        assert result.exit_code == 0
        assert "NOT CLONED" in result.output


def test_status_json_format(tmp_path: Path) -> None:
    runner = CliRunner()
    with runner.isolated_filesystem(temp_dir=tmp_path) as td:
        cfg = Path(td) / ".gitscale"
        cfg.write_text("libs/x https://x.git main readonly\n")
        result = runner.invoke(cli, ["status", "--format", "json"])
        assert result.exit_code == 0
        assert '"exists": false' in result.output
