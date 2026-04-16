"""Tests for the CLI entry point."""

from click.testing import CliRunner

from gitscale.cli import cli


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
    assert "Clone one or more" in result.output


def test_status_help() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["status", "--help"])
    assert result.exit_code == 0
    assert "Show status" in result.output


def test_clone_basic() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["clone", "https://github.com/example/repo.git"])
    assert result.exit_code == 0
    assert "Would clone" in result.output


def test_status_basic() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["status", "."])
    assert result.exit_code == 0
    assert "Would scan" in result.output
