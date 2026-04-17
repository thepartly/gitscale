"""Upload and download custom metadata for repo revisions."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import click

from gitscale.config import ConfigError, find_config, load_config
from gitscale.storage import StorageError, download_metadata, upload_metadata

if TYPE_CHECKING:
    from gitscale.config import GitScaleConfig, RepoEntry


@click.group()
def metadata() -> None:
    """Manage custom metadata for repo revisions."""


@metadata.command()
@click.argument("directory")
@click.argument("file", type=click.Path(exists=True), required=False)
@click.option(
    "-C",
    "--root",
    type=click.Path(exists=True, path_type=Path),
    default=None,
    help="Root directory containing .gitscale.toml (default: auto-detect).",
)
@click.pass_context
def push(
    ctx: click.Context,
    directory: str,
    file: str | None,
    root: Path | None,
) -> None:
    """Upload metadata JSON for a repo's current revision.

    DIRECTORY is the repo entry name from .gitscale.toml.
    FILE is a JSON file to upload. Reads from stdin if omitted.

    Example:
        gitscale metadata push fe/app checksums.json
        echo '{"build": "ok"}' | gitscale metadata push fe/app
    """
    config, config_root = _load(root)
    entry = _find_entry(config, directory)
    _require_storage(config)

    # Read JSON
    if file:
        raw = Path(file).read_text(encoding="utf-8")
    else:
        if sys.stdin.isatty():
            raise click.ClickException(
                "No file given and stdin is a terminal. "
                "Provide a JSON file or pipe JSON to stdin."
            )
        raw = sys.stdin.read()

    try:
        data: dict[str, object] = json.loads(raw)
    except json.JSONDecodeError as e:
        raise click.ClickException(f"Invalid JSON: {e}") from None

    if not isinstance(data, dict):
        raise click.ClickException(
            "Metadata must be a JSON object (key-value pairs)."
        )

    revision = entry.revision or "HEAD"
    try:
        url = upload_metadata(
            config.storage_url, entry.repo_url, revision, data
        )
    except StorageError as e:
        raise click.ClickException(str(e)) from None

    click.echo(f"Uploaded {directory} @ {revision} → {url}")


@metadata.command()
@click.argument("directory")
@click.option(
    "-C",
    "--root",
    type=click.Path(exists=True, path_type=Path),
    default=None,
    help="Root directory containing .gitscale.toml (default: auto-detect).",
)
@click.pass_context
def pull(
    ctx: click.Context,
    directory: str,
    root: Path | None,
) -> None:
    """Download metadata JSON for a repo's current revision.

    Prints the JSON to stdout.

    DIRECTORY is the repo entry name from .gitscale.toml.
    """
    config, config_root = _load(root)
    entry = _find_entry(config, directory)
    _require_storage(config)

    revision = entry.revision or "HEAD"
    try:
        data = download_metadata(
            config.storage_url, entry.repo_url, revision
        )
    except StorageError as e:
        raise click.ClickException(str(e)) from None

    if data is None:
        raise click.ClickException(
            f"No metadata found for {directory} @ {revision}"
        )

    click.echo(json.dumps(data, indent=2))


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _load(root: Path | None) -> tuple[GitScaleConfig, Path]:
    try:
        config_path = find_config(root)
    except ConfigError as e:
        raise click.ClickException(str(e)) from None
    config = load_config(config_path)
    return config, config_path.parent


def _find_entry(config: GitScaleConfig, directory: str) -> RepoEntry:
    for entry in config.repos:
        if entry.directory == directory:
            return entry
    raise click.ClickException(
        f"'{directory}' not found in .gitscale.toml"
    )


def _require_storage(config: GitScaleConfig) -> None:
    if not config.storage_url:
        raise click.ClickException(
            "No [storage] configured in .gitscale.toml. Add:\n"
            "  [storage]\n"
            '  url = "https://your-bucket.s3.amazonaws.com/prefix"'
        )
