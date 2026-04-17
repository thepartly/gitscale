"""Config parser for .gitscale TOML files."""

import tomllib
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Any


class RepoMode(Enum):
    READONLY = "readonly"
    READWRITE = "readwrite"
    METADATA = "metadata"


@dataclass(frozen=True, slots=True)
class RepoEntry:
    """A single sub-repository declaration from .gitscale config."""

    directory: str
    repo_url: str
    revision: str
    mode: RepoMode

    @property
    def is_readonly(self) -> bool:
        return self.mode in (RepoMode.READONLY, RepoMode.METADATA)

    @property
    def is_metadata(self) -> bool:
        return self.mode == RepoMode.METADATA


@dataclass(frozen=True, slots=True)
class GitScaleConfig:
    """Parsed .gitscale configuration."""

    repos: list[RepoEntry]
    hosts: dict[str, str] = field(default_factory=dict)


class ConfigError(Exception):
    """Raised when .gitscale config is malformed."""


CONFIG_FILENAME = ".gitscale.toml"

_VALID_PLATFORMS = {"github", "gitlab"}


def find_config(start: Path | None = None) -> Path:
    """Find .gitscale config file, searching upward from start directory.

    Raises ConfigError if not found.
    """
    current = (start or Path.cwd()).resolve()
    while True:
        candidate = current / CONFIG_FILENAME
        if candidate.is_file():
            return candidate
        parent = current.parent
        if parent == current:
            break
        current = parent
    raise ConfigError(
        f"No {CONFIG_FILENAME} config found "
        f"(searched upward from {start or Path.cwd()})"
    )


def load_config(config_path: Path) -> GitScaleConfig:
    """Load and parse a .gitscale TOML config file."""
    text = config_path.read_text(encoding="utf-8")
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as e:
        raise ConfigError(f"{config_path}: invalid TOML: {e}") from None

    hosts = _parse_hosts(data.get("hosts", {}), config_path)
    repos = _parse_repos(data.get("repos", {}), config_path)

    return GitScaleConfig(repos=repos, hosts=hosts)


def _parse_hosts(
    raw: Any, config_path: Path
) -> dict[str, str]:
    """Parse the [hosts] table."""
    if not isinstance(raw, dict):
        raise ConfigError(
            f"{config_path}: [hosts] must be a table"
        )
    hosts: dict[str, str] = {}
    for hostname, platform in raw.items():
        if not isinstance(platform, str):
            raise ConfigError(
                f"{config_path}: hosts.{hostname} must be a string"
            )
        if platform not in _VALID_PLATFORMS:
            raise ConfigError(
                f"{config_path}: hosts.{hostname}: "
                f"unknown platform '{platform}', "
                f"expected one of: {', '.join(sorted(_VALID_PLATFORMS))}"
            )
        hosts[hostname] = platform
    return hosts


def _parse_repos(
    raw: Any, config_path: Path
) -> list[RepoEntry]:
    """Parse the [repos] table."""
    if not isinstance(raw, dict):
        raise ConfigError(
            f"{config_path}: [repos] must be a table"
        )
    entries: list[RepoEntry] = []
    for directory, spec in raw.items():
        if not isinstance(spec, dict):
            raise ConfigError(
                f"{config_path}: repos.{directory} must be "
                f"an inline table with 'url' field"
            )

        url = spec.get("url")
        if not isinstance(url, str) or not url:
            raise ConfigError(
                f"{config_path}: repos.{directory}.url is required"
            )

        revision = spec.get("revision", "")
        if not isinstance(revision, str):
            raise ConfigError(
                f"{config_path}: repos.{directory}.revision "
                f"must be a string"
            )

        mode_str = spec.get("mode", "readwrite")
        if not isinstance(mode_str, str):
            raise ConfigError(
                f"{config_path}: repos.{directory}.mode "
                f"must be a string"
            )
        try:
            mode = RepoMode(mode_str)
        except ValueError:
            raise ConfigError(
                f"{config_path}: repos.{directory}.mode: "
                f"invalid mode '{mode_str}', expected one of: "
                f"{', '.join(m.value for m in RepoMode)}"
            ) from None

        entries.append(
            RepoEntry(
                directory=directory,
                repo_url=url,
                revision=revision,
                mode=mode,
            )
        )
    return entries


# --- Legacy helpers kept for write support (add command) ---


def parse_config(config_path: Path) -> list[RepoEntry]:
    """Parse a .gitscale TOML config and return repo entries."""
    return load_config(config_path).repos


def write_config(
    config_path: Path,
    entries: list[RepoEntry],
    hosts: dict[str, str] | None = None,
) -> None:
    """Write a .gitscale config file in TOML format."""
    lines: list[str] = []

    if hosts:
        lines.append("[hosts]")
        for hostname, platform in sorted(hosts.items()):
            lines.append(f'"{hostname}" = "{platform}"')
        lines.append("")

    if entries:
        lines.append("[repos]")
        for entry in entries:
            parts = [f'url = "{entry.repo_url}"']
            if entry.revision:
                parts.append(f'revision = "{entry.revision}"')
            if entry.mode != RepoMode.READWRITE:
                parts.append(f'mode = "{entry.mode.value}"')
            inline = ", ".join(parts)
            lines.append(f'"{entry.directory}" = {{ {inline} }}')

    lines.append("")  # trailing newline
    config_path.write_text("\n".join(lines), encoding="utf-8")
