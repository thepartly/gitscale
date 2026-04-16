"""Show status of managed sub-repositories."""

import json
from pathlib import Path

import click

from gitscale.config import ConfigError, find_config, parse_config
from gitscale.git import RepoStatus, fetch_repo, get_repo_status


@click.command()
@click.option(
    "-C",
    "--root",
    type=click.Path(exists=True, path_type=Path),
    default=None,
    help="Root directory containing .gitscale (default: auto-detect).",
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
    """Show status of repos declared in .gitscale."""
    verbose: bool = ctx.obj["verbose"]

    try:
        config_path = find_config(root)
    except ConfigError as e:
        raise click.ClickException(str(e)) from None

    config_root = config_path.parent
    entries = parse_config(config_path)

    if not entries:
        click.echo("No repos declared in .gitscale")
        return

    if fetch:
        for entry in entries:
            dest = config_root / entry.directory
            if dest.exists():
                if verbose:
                    click.echo(f"Fetching {entry.directory}...")
                fetch_repo(entry, config_root)

    statuses = [get_repo_status(e, config_root) for e in entries]

    if output_format == "json":
        _print_json(statuses)
    else:
        _print_table(statuses)


def _print_table(statuses: list[RepoStatus]) -> None:
    """Print status in a human-readable table."""
    if not statuses:
        return

    # Header
    click.echo(
        f"{'REPO':<25} {'REF':<20} {'EXPECTED':<20} {'STATUS'}"
    )
    click.echo("-" * 80)

    for s in statuses:
        if not s.exists:
            click.echo(
                f"{s.directory:<25} {'—':<20} "
                f"{s.expected_ref:<20} NOT CLONED"
            )
            continue

        flags: list[str] = []
        if not s.is_clean:
            flags.append("dirty")
        if s.is_detached:
            flags.append("detached")
        if s.ahead:
            flags.append(f"+{s.ahead}")
        if s.behind:
            flags.append(f"-{s.behind}")
        if s.current_ref != s.expected_ref and not s.is_detached:
            flags.append("ref-mismatch")

        status_str = ", ".join(flags) if flags else "ok"
        click.echo(
            f"{s.directory:<25} {s.current_ref:<20} "
            f"{s.expected_ref:<20} {status_str}"
        )


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
