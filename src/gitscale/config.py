"""Config parser for .gitscale TOML files."""

import tomllib
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Any


class RepoMode(Enum):
    READONLY = "readonly"
    READWRITE = "readwrite"
    MANIFEST = "manifest"


@dataclass(frozen=True, slots=True)
class RepoEntry:
    """A single sub-repository declaration from .gitscale config."""

    directory: str
    repo_url: str
    revision: str
    mode: RepoMode

    @property
    def is_readonly(self) -> bool:
        return self.mode in (RepoMode.READONLY, RepoMode.MANIFEST)

    @property
    def is_manifest(self) -> bool:
        return self.mode == RepoMode.MANIFEST


@dataclass(frozen=True, slots=True)
class GitScaleConfig:
    """Parsed .gitscale configuration."""

    repos: list[RepoEntry]
    storage_url: str = ""


class ConfigError(Exception):
    """Raised when .gitscale config is malformed."""


CONFIG_FILENAME = ".gitscale.toml"


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

    repos = _parse_repos(data.get("repos", {}), config_path)
    storage_url = _parse_storage(data.get("storage", {}), config_path)

    return GitScaleConfig(repos=repos, storage_url=storage_url)


def _parse_storage(
    raw: Any, config_path: Path
) -> str:
    """Parse the [storage] table. Returns the URL or empty string."""
    if not isinstance(raw, dict):
        raise ConfigError(
            f"{config_path}: [storage] must be a table"
        )
    if not raw:
        return ""
    url = raw.get("url")
    if not isinstance(url, str) or not url:
        raise ConfigError(
            f"{config_path}: storage.url is required"
        )
    return url.rstrip("/")


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
    storage_url: str = "",
) -> None:
    """Write a .gitscale config file in TOML format."""
    lines: list[str] = []

    if storage_url:
        lines.append("[storage]")
        lines.append(f'url = "{storage_url}"')
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
