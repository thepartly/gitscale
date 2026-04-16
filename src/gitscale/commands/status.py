"""Check status of multiple repositories."""

from pathlib import Path

import click


@click.command()
@click.argument("path", type=click.Path(exists=True, path_type=Path), default=".")
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
def status(ctx: click.Context, path: Path, fetch: bool, output_format: str) -> None:
    """Show status of repos under PATH."""
    verbose: bool = ctx.obj["verbose"]
    path = path.resolve()

    if verbose:
        click.echo(f"Scanning {path} for git repos...")

    # TODO: implement actual repo discovery and status checking
    click.echo(f"Would scan {path} for repos (fetch={fetch}, format={output_format})")
