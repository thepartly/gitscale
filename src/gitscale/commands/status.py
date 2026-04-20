"""Show status of managed sub-repositories."""

import json
from pathlib import Path

import click

from gitscale.config import ConfigError, find_config, load_config
from gitscale.git import (
    RepoStatus,
    fetch_repo,
    get_artefact_status,
    get_repo_status,
    get_self_status,
)
from gitscale.storage import fetch_artefact


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
            dest = config_root / entry.directory
            if entry.is_artefact:
                if config.storage_url:
                    revision = entry.revision or "HEAD"
                    if verbose:
                        click.echo(f"Fetching {entry.directory}...")
                    fetch_artefact(
                        config.storage_url, entry.repo_url, revision, dest
                    )
                continue
            if dest.exists():
                if verbose:
                    click.echo(f"Fetching {entry.directory}...")
                fetch_repo(entry, config_root)

    statuses: list[RepoStatus] = []

    self_status = get_self_status(config_root)
    if self_status is not None:
        statuses.append(self_status)

    for entry in config.repos:
        if entry.is_artefact:
            statuses.append(get_artefact_status(entry, config_root))
        else:
            statuses.append(get_repo_status(entry, config_root))

    if output_format == "json":
        _print_json(statuses)
    else:
        _print_table(statuses)


def _get_status_flags(s: RepoStatus) -> str:
    """Compute status flags string for a repo."""
    if not s.exists:
        return "missed"

    flags: list[str] = []
    if not s.is_clean:
        flags.append("dirty")
    if s.is_stale:
        flags.append("stale")
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
        and s.current_ref != "artefact"
    ):
        flags.append("ref-mismatch")

    return ", ".join(flags) if flags else "ok"


def _status_icon(flags: str) -> str:
    """Return a UTF-8 status icon for the given flags."""
    if flags == "ok":
        return "✔"
    if "missed" in flags:
        return "✘"
    if "dirty" in flags:
        return "!"
    if "stale" in flags or "ref-mismatch" in flags:
        return "≠"
    has_ahead = "+" in flags
    has_behind = "-" in flags
    if has_ahead and has_behind:
        return "⇅"
    if has_ahead:
        return "⇑"
    if has_behind:
        return "⇓"
    return "◆"


def _status_color(flags: str) -> str:
    """Return an ANSI color name for the given flags."""
    if flags == "ok":
        return "green"
    if "missed" in flags:
        return "red"
    if "dirty" in flags or "ref-mismatch" in flags or "stale" in flags:
        return "bright_red"  # renders as orange in most terminals
    if "+" in flags or "-" in flags:
        return "yellow"
    return "cyan"


def _print_table(statuses: list[RepoStatus]) -> None:
    """Print status in a docker ps style borderless table with colors."""
    if not statuses:
        return

    headers = ("", "REPO", "MODE", "REF", "EXPECTED", "STATUS")
    rows: list[tuple[str, str, str, str, str, str]] = []
    for s in statuses:
        flags = _get_status_flags(s)
        icon = _status_icon(flags)
        ref = s.current_ref if s.exists else "—"
        rows.append((icon, s.directory, s.mode, ref, s.expected_ref, flags))

    # Compute column widths (minimum = header width)
    widths = [len(h) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))
    # Icon column is always 1 char wide
    widths[0] = max(widths[0], 1)

    gap = "   "
    fmt = gap.join(f"{{:<{w}}}" for w in widths)

    click.echo(fmt.format(*headers))
    for row in rows:
        flags = row[5]
        color = _status_color(flags)
        plain = fmt.format(*row)
        # Colorize the icon and status columns
        icon_plain = row[0].ljust(widths[0])
        status_plain = row[5].ljust(widths[5])
        icon_bold = "missed" not in flags
        plain = plain.replace(
            icon_plain, click.style(icon_plain, fg=color, bold=icon_bold), 1
        )
        # Replace the last occurrence of status text with colored version
        idx = plain.rfind(status_plain)
        if idx >= 0:
            end = idx + len(status_plain)
            colored = click.style(status_plain, fg=color)
            plain = plain[:idx] + colored + plain[end:]
        click.echo(plain)


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
            "mode": s.mode,
            "stale": s.is_stale,
        }
        for s in statuses
    ]
    click.echo(json.dumps(data, indent=2))
