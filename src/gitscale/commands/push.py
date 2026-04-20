"""Push local changes to remote."""

from pathlib import Path

import click

from gitscale.config import ConfigError, RepoEntry, find_config, load_config
from gitscale.git import GitError, push_repo
from gitscale.storage import StorageError, push_manifest


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
def push(
    ctx: click.Context,
    root: Path | None,
    names: tuple[str, ...],
) -> None:
    """Push local changes for sub-repositories.

    For git repos: runs git push. Readonly entries are skipped.
    For manifests: uploads to cloud storage if local differs from remote.
    If NAMES are given, push only those entries. Otherwise push all.
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
        click.echo("Nothing to push.")
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
                uploaded = push_manifest(
                    config.storage_url, entry.repo_url, revision, dest
                )
                if uploaded:
                    click.echo(f"  ok    {entry.directory} (pushed)")
                else:
                    click.echo(f"  skip  {entry.directory} (up to date)")
                continue

            if entry.is_readonly:
                click.echo(f"  skip  {entry.directory} (readonly)")
                continue

            dest = config_root / entry.directory
            if not dest.exists():
                click.echo(f"  skip  {entry.directory} (not cloned)")
                continue
            if verbose:
                click.echo(f"  push  {entry.directory}")
            push_repo(entry, config_root, verbose=verbose)
            click.echo(f"  ok    {entry.directory}")
        except (GitError, StorageError) as e:
            click.echo(f"  FAIL  {entry.directory}: {e}", err=True)
            failed += 1

    if failed:
        raise click.ClickException(f"{failed} repo(s) failed to push")


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
