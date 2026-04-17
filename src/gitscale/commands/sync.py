"""Sync sub-repositories to their declared revisions."""

from pathlib import Path

import click

from gitscale.config import ConfigError, find_config, load_config
from gitscale.git import GitError, sync_metadata, sync_repo


@click.command()
@click.option(
    "-C",
    "--root",
    type=click.Path(exists=True, path_type=Path),
    default=None,
    help="Root directory containing .gitscale.toml (default: auto-detect).",
)
@click.argument("names", nargs=-1)
@click.pass_context
def sync(
    ctx: click.Context,
    root: Path | None,
    names: tuple[str, ...],
) -> None:
    """Fetch and checkout declared revisions for sub-repositories.

    Clones repos that don't exist locally yet.
    Metadata-only entries fetch from the platform API.
    If NAMES are given, sync only those entries. Otherwise sync all.
    """
    verbose: bool = ctx.obj["verbose"]

    try:
        config_path = find_config(root)
    except ConfigError as e:
        raise click.ClickException(str(e)) from None

    config_root = config_path.parent
    config = load_config(config_path)
    entries = config.repos

    if names:
        name_set = set(names)
        entries = [e for e in entries if e.directory in name_set]
        unknown = name_set - {e.directory for e in entries}
        if unknown:
            raise click.ClickException(
                f"Unknown repos: {', '.join(sorted(unknown))}"
            )

    if not entries:
        click.echo("Nothing to sync.")
        return

    failed = 0
    for entry in entries:
        try:
            if entry.is_metadata:
                if verbose:
                    click.echo(
                        f"  meta  {entry.directory} → {entry.revision}"
                    )
                sync_metadata(entry, config_root, config.hosts)
                click.echo(f"  ok    {entry.directory} (metadata)")
            else:
                if verbose:
                    click.echo(
                        f"  sync  {entry.directory} → {entry.revision}"
                    )
                sync_repo(entry, config_root, verbose=verbose)
                click.echo(f"  ok    {entry.directory}")
        except (GitError, Exception) as e:
            click.echo(f"  FAIL  {entry.directory}: {e}", err=True)
            failed += 1

    if failed:
        raise click.ClickException(
            f"{failed} repo(s) failed to sync"
        )
