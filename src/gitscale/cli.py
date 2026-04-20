"""Main CLI entry point."""

import click

from gitscale import __version__


@click.group()
@click.version_option(version=__version__, prog_name="gitscale")
@click.option("-v", "--verbose", is_flag=True, help="Enable verbose output.")
@click.pass_context
def cli(ctx: click.Context, verbose: bool) -> None:
    """GitScale — manage multiple sub-repositories from a .gitscale.toml config."""
    ctx.ensure_object(dict)
    ctx.obj["verbose"] = verbose


# Import and register subcommands
from gitscale.commands import (  # noqa: E402
    add,
    clone,
    fetch,
    pull,
    push,
    remove,
    status,
    sync,
)

cli.add_command(clone.clone)
cli.add_command(fetch.fetch)
cli.add_command(pull.pull)
cli.add_command(push.push)
cli.add_command(sync.sync)
cli.add_command(status.status)
cli.add_command(add.add)
cli.add_command(remove.remove)
