"""Remove a sub-repository entry from .gitscale config."""

from pathlib import Path

import click

from gitscale.config import (
    find_config,
    load_config,
    write_config,
)


@click.command()
@click.argument("directory")
@click.option(
    "-C",
    "--root",
    type=click.Path(exists=True, path_type=Path),
    default=None,
    help="Root directory containing .gitscale.toml (default: auto-detect).",
)
@click.pass_context
def remove(
    ctx: click.Context,
    directory: str,
    root: Path | None,
) -> None:
    """Remove a sub-repository entry from .gitscale config.

    DIRECTORY is the local subdirectory name to remove.
    """
    config_path = find_config(root)
    config = load_config(config_path)

    remaining = [e for e in config.repos if e.directory != directory]

    if len(remaining) == len(config.repos):
        raise click.ClickException(
            f"'{directory}' is not declared in {config_path}"
        )

    write_config(config_path, remaining, storage_url=config.storage_url)
    click.echo(f"Removed {directory}")
