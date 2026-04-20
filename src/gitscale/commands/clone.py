"""Clone all sub-repositories declared in .gitscale."""

from pathlib import Path

import click

from gitscale.config import ConfigError, RepoEntry, find_config, load_config
from gitscale.git import GitError, clone_repo, is_ci
from gitscale.storage import StorageError, clone_artefact


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
def clone(
    ctx: click.Context,
    root: Path | None,
    names: tuple[str, ...],
) -> None:
    """Clone sub-repositories from .gitscale config.

    If NAMES are given, clone only those entries. Otherwise clone all.
    Metadata-only entries are skipped.
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
        click.echo("Nothing to clone.")
        return

    failed = 0
    for entry in selected:
        dest = config_root / entry.directory
        if entry.is_artefact:
            if dest.exists():
                click.echo(f"  skip  {entry.directory} (already exists)")
                continue
            if not config.storage_url:
                click.echo(
                    f"  FAIL  {entry.directory}: no [storage] configured",
                    err=True,
                )
                failed += 1
                continue
            try:
                revision = entry.revision or "HEAD"
                found = clone_artefact(
                    config.storage_url, entry.repo_url, revision, dest
                )
                if found:
                    click.echo(f"  ok    {entry.directory} (artefact)")
                else:
                    click.echo(f"  skip  {entry.directory} (no artefact data)")
            except StorageError as e:
                click.echo(f"  FAIL  {entry.directory}: {e}", err=True)
                failed += 1
            continue
        if dest.exists():
            click.echo(f"  skip  {entry.directory} (already exists)")
            continue
        try:
            if verbose:
                click.echo(
                    f"  clone {entry.repo_url} → "
                    f"{entry.directory} @ {entry.revision}"
                )
            ci = is_ci()
            shallow = ci or entry.is_readonly
            clone_repo(
                entry, config_root, verbose=verbose, shallow=shallow
            )
            click.echo(f"  ok    {entry.directory}")
        except GitError as e:
            click.echo(f"  FAIL  {entry.directory}: {e}", err=True)
            failed += 1

    if failed:
        raise click.ClickException(
            f"{failed} repo(s) failed to clone"
        )


def _filter_entries(
    entries: list[RepoEntry], names: tuple[str, ...]
) -> list[RepoEntry]:
    """Filter entries by name, or return all if names is empty."""
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
