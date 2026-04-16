"""Config parser for .gitscale files."""

from dataclasses import dataclass
from enum import Enum
from pathlib import Path


class RepoMode(Enum):
    READONLY = "readonly"
    READWRITE = "readwrite"


@dataclass(frozen=True, slots=True)
class RepoEntry:
    """A single sub-repository declaration from .gitscale config."""

    directory: str
    repo_url: str
    revision: str
    mode: RepoMode

    @property
    def is_readonly(self) -> bool:
        return self.mode == RepoMode.READONLY


class ConfigError(Exception):
    """Raised when .gitscale config is malformed."""


CONFIG_FILENAME = ".gitscale"


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


def parse_config(config_path: Path) -> list[RepoEntry]:
    """Parse a .gitscale config file into a list of RepoEntry objects."""
    entries: list[RepoEntry] = []
    text = config_path.read_text(encoding="utf-8")

    for line_num, raw_line in enumerate(text.splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        parts = line.split()
        if len(parts) < 3:
            raise ConfigError(
                f"{config_path}:{line_num}: expected at least 3 fields "
                f"(directory repo revision [mode]), got {len(parts)}"
            )
        if len(parts) > 4:
            raise ConfigError(
                f"{config_path}:{line_num}: expected at most 4 fields, "
                f"got {len(parts)}"
            )

        directory, repo_url, revision = parts[0], parts[1], parts[2]
        mode_str = parts[3] if len(parts) == 4 else "readwrite"

        try:
            mode = RepoMode(mode_str)
        except ValueError:
            raise ConfigError(
                f"{config_path}:{line_num}: invalid mode '{mode_str}', "
                f"expected 'readonly' or 'readwrite'"
            ) from None

        entries.append(
            RepoEntry(
                directory=directory,
                repo_url=repo_url,
                revision=revision,
                mode=mode,
            )
        )

    return entries


def write_config(config_path: Path, entries: list[RepoEntry]) -> None:
    """Write a list of RepoEntry objects to a .gitscale config file."""
    lines: list[str] = []
    for entry in entries:
        lines.append(
            f"{entry.directory} {entry.repo_url} "
            f"{entry.revision} {entry.mode.value}"
        )
    config_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
