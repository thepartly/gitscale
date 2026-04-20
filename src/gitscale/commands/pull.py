"""Pull latest changes from remote."""

from pathlib import Path

import click

from gitscale.config import ConfigError, RepoEntry, find_config, load_config
from gitscale.git import GitError, is_ci, pull_repo
from gitscale.storage import StorageError, pull_manifest


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
def pull(
    ctx: click.Context,
    root: Path | None,
    names: tuple[str, ...],
) -> None:
    """Pull latest changes for sub-repositories.

    For git repos: runs git pull --ff-only. Clones if not yet cloned.
    For manifests: downloads from cloud storage if remote is newer.
    If NAMES are given, pull only those entries. Otherwise pull all.
    """
    verbose: bool = ctx.obj["verbose"]

    try:
        config_path = find_config(root)
    except ConfigError as e:
        raise click.ClickException(str(e)) from None

    config_root = config_path.parent
    config = load_config(config_path)
    selected = _filter_entries(config.repos, names)

    if not selected:
        click.echo("Nothing to pull.")
        return

    failed = 0
    for entry in selected:
        try:
            if entry.is_manifest:
                if not config.storage_url:
                    click.echo(
                        f"  FAIL  {entry.directory}: no [storage] configured",
                        err=True,
                    )
                    failed += 1
                    continue
                dest = config_root / entry.directory
                revision = entry.revision or "HEAD"
                found = pull_manifest(
                    config.storage_url, entry.repo_url, revision, dest
                )
                if found:
                    click.echo(f"  ok    {entry.directory} (manifest)")
                else:
                    click.echo(
                        f"  skip  {entry.directory} (no remote manifest)"
                    )
                continue

            if verbose:
                click.echo(f"  pull  {entry.directory}")
            ci = is_ci()
            shallow = ci or entry.is_readonly
            pull_repo(
                entry, config_root, verbose=verbose, shallow=shallow
            )
            click.echo(f"  ok    {entry.directory}")
        except (GitError, StorageError) as e:
            click.echo(f"  FAIL  {entry.directory}: {e}", err=True)
            failed += 1

    if failed:
        raise click.ClickException(f"{failed} repo(s) failed to pull")


def _filter_entries(
    entries: list[RepoEntry], names: tuple[str, ...]
) -> list[RepoEntry]:
    if not names:
        return entries
    name_set = set(names)
    matched = [e for e in entries if e.directory in name_set]
    unknown = name_set - {e.directory for e in matched}
    if unknown:
        raise click.ClickException(
            f"Unknown repos: {', '.join(sorted(unknown))}"
        )
    return matched
