"""Sync sub-repositories: clone + pull + push."""

from pathlib import Path

import click

from gitscale.commands.clone import clone
from gitscale.commands.pull import pull
from gitscale.commands.push import push


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
    """Full sync: clone missing repos, pull updates, push local changes.

    Equivalent to running clone + pull + push in sequence.
    If NAMES are given, sync only those entries. Otherwise sync all.
    """
    common: list[str] = []
    if root is not None:
        common.extend(["-C", str(root)])
    common.extend(list(names))

    ctx.invoke(clone, root=root, names=names)
    ctx.invoke(pull, root=root, names=names)
    ctx.invoke(push, root=root, names=names)
