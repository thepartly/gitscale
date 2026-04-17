"""Add a new sub-repository entry to .gitscale config."""

from pathlib import Path

import click

from gitscale.config import (
    CONFIG_FILENAME,
    ConfigError,
    RepoEntry,
    RepoMode,
    find_config,
    load_config,
    write_config,
)


@click.command()
@click.argument("directory")
@click.argument("repo_url")
@click.argument("revision")
@click.option(
    "--mode",
    type=click.Choice(
        ["readonly", "readwrite", "metadata"], case_sensitive=False
    ),
    default="readwrite",
    help="Access mode for the sub-repository.",
)
@click.option(
    "-C",
    "--root",
    type=click.Path(exists=True, path_type=Path),
    default=None,
    help="Root directory containing .gitscale.toml (default: auto-detect).",
)
@click.pass_context
def add(
    ctx: click.Context,
    directory: str,
    repo_url: str,
    revision: str,
    mode: str,
    root: Path | None,
) -> None:
    """Add a sub-repository entry to .gitscale config.

    DIRECTORY is the local subdirectory name.
    REPO_URL is the git repository URL.
    REVISION is the branch, tag, or commit to checkout.
    """
    hosts: dict[str, str] = {}
    try:
        config_path = find_config(root)
        config = load_config(config_path)
        entries = list(config.repos)
        hosts = config.hosts
    except ConfigError:
        # No config yet — create one
        config_path = (root or Path.cwd()).resolve() / CONFIG_FILENAME
        entries = []

    # Check for duplicate
    for entry in entries:
        if entry.directory == directory:
            raise click.ClickException(
                f"'{directory}' is already declared in "
                f"{config_path}"
            )

    new_entry = RepoEntry(
        directory=directory,
        repo_url=repo_url,
        revision=revision,
        mode=RepoMode(mode),
    )
    entries.append(new_entry)
    write_config(config_path, entries, hosts=hosts)
    click.echo(
        f"Added {directory} → {repo_url} @ {revision} [{mode}]"
    )
