"""Show status of managed sub-repositories."""

import json
from pathlib import Path

import click
from rich.console import Console
from rich.table import Table

from gitscale.config import ConfigError, find_config, load_config
from gitscale.git import (
    RepoStatus,
    fetch_repo,
    get_metadata_status,
    get_repo_status,
    get_self_status,
)


@click.command()
@click.option(
    "-C",
    "--root",
    type=click.Path(exists=True, path_type=Path),
    default=None,
    help="Root directory containing .gitscale.toml (default: auto-detect).",
)
@click.option(
    "--fetch/--no-fetch",
    default=False,
    help="Run git fetch before checking status.",
)
@click.option(
    "-f",
    "--format",
    "output_format",
    type=click.Choice(["table", "json"], case_sensitive=False),
    default="table",
    help="Output format.",
)
@click.pass_context
def status(
    ctx: click.Context,
    root: Path | None,
    fetch: bool,
    output_format: str,
) -> None:
    """Show status of repos declared in .gitscale.toml."""
    verbose: bool = ctx.obj["verbose"]

    try:
        config_path = find_config(root)
    except ConfigError as e:
        raise click.ClickException(str(e)) from None

    config_root = config_path.parent
    config = load_config(config_path)

    if not config.repos:
        click.echo("No repos declared in .gitscale.toml")
        return

    if fetch:
        for entry in config.repos:
            if entry.is_metadata:
                continue
            dest = config_root / entry.directory
            if dest.exists():
                if verbose:
                    click.echo(f"Fetching {entry.directory}...")
                fetch_repo(entry, config_root)

    statuses: list[RepoStatus] = []

    self_status = get_self_status(config_root)
    if self_status is not None:
        statuses.append(self_status)

    for entry in config.repos:
        if entry.is_metadata:
            statuses.append(get_metadata_status(entry, config_root))
        else:
            statuses.append(get_repo_status(entry, config_root))

    if output_format == "json":
        _print_json(statuses)
    else:
        _print_table(statuses)


def _get_status_flags(s: RepoStatus) -> str:
    """Compute status flags string for a repo."""
    if not s.exists:
        return "NOT CLONED"

    flags: list[str] = []
    if not s.is_clean:
        flags.append("dirty")
    if s.is_detached:
        flags.append("detached")
    if s.ahead:
        flags.append(f"+{s.ahead}")
    if s.behind:
        flags.append(f"-{s.behind}")
    if (
        s.expected_ref
        and s.current_ref != s.expected_ref
        and not s.is_detached
        and s.current_ref != "metadata"
    ):
        flags.append("ref-mismatch")

    return ", ".join(flags) if flags else "ok"


def _status_style(flags: str) -> str:
    """Return a rich style string based on status flags."""
    if flags == "ok":
        return "green"
    if "NOT CLONED" in flags:
        return "red"
    if "dirty" in flags or "ref-mismatch" in flags:
        return "yellow"
    return "cyan"


def _print_table(statuses: list[RepoStatus]) -> None:
    """Print status in a human-readable table."""
    if not statuses:
        return

    console = Console()
    table = Table(show_edge=False, pad_edge=False, expand=False)

    table.add_column("Repo", style="bold")
    table.add_column("Ref")
    table.add_column("Expected")
    table.add_column("Status")

    for s in statuses:
        flags = _get_status_flags(s)
        style = _status_style(flags)
        ref = s.current_ref if s.exists else "—"
        table.add_row(s.directory, ref, s.expected_ref, f"[{style}]{flags}[/]")

    console.print(table)


def _print_json(statuses: list[RepoStatus]) -> None:
    """Print status as JSON."""
    data = [
        {
            "directory": s.directory,
            "exists": s.exists,
            "current_ref": s.current_ref,
            "expected_ref": s.expected_ref,
            "clean": s.is_clean,
            "detached": s.is_detached,
            "ahead": s.ahead,
            "behind": s.behind,
        }
        for s in statuses
    ]
    click.echo(json.dumps(data, indent=2))
