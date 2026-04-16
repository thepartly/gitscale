"""Clone repositories at scale."""

from pathlib import Path

import click


@click.command()
@click.argument("repos", nargs=-1, required=True)
@click.option(
    "-d",
    "--dest",
    type=click.Path(path_type=Path),
    default=Path("."),
    help="Destination directory for cloned repos.",
)
@click.option(
    "--depth",
    type=int,
    default=None,
    help="Create a shallow clone with the given depth.",
)
@click.option(
    "--branch",
    "-b",
    type=str,
    default=None,
    help="Branch to clone.",
)
@click.pass_context
def clone(
    ctx: click.Context,
    repos: tuple[str, ...],
    dest: Path,
    depth: int | None,
    branch: str | None,
) -> None:
    """Clone one or more repositories into DEST."""
    verbose: bool = ctx.obj["verbose"]
    dest = dest.resolve()

    for repo in repos:
        if verbose:
            click.echo(f"Cloning {repo} → {dest}")

        # TODO: implement actual git clone logic
        opts = ""
        if branch or depth:
            opts = f" (branch={branch}, depth={depth})"
        click.echo(f"Would clone {repo} into {dest}{opts}")
