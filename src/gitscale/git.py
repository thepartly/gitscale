"""Git operations for managing sub-repositories."""

import os
import stat
import subprocess
from dataclasses import dataclass
from pathlib import Path

from gitscale.config import RepoEntry


class GitError(Exception):
    """Raised when a git command fails."""


def _run_git(
    args: list[str],
    cwd: Path | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    """Run a git command and return the result."""
    cmd = ["git", *args]
    result = subprocess.run(
        cmd,
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise GitError(
            f"git {' '.join(args)} failed (exit {result.returncode}):\n"
            f"{result.stderr.strip()}"
        )
    return result


def clone_repo(
    entry: RepoEntry,
    root: Path,
    *,
    verbose: bool = False,
) -> None:
    """Clone a repository into root/entry.directory."""
    dest = root / entry.directory
    if dest.exists():
        raise GitError(f"Directory already exists: {dest}")

    args = ["clone", entry.repo_url, str(dest)]
    if verbose:
        args.append("--progress")
    else:
        args.append("--quiet")

    _run_git(args)
    checkout_revision(entry, root)
    if entry.is_readonly:
        apply_readonly(dest)


def checkout_revision(entry: RepoEntry, root: Path) -> None:
    """Checkout the declared revision/branch/tag for a repo."""
    dest = root / entry.directory
    # First try as a branch/tag name
    result = _run_git(
        ["checkout", entry.revision],
        cwd=dest,
        check=False,
    )
    if result.returncode != 0:
        # Try as a detached HEAD (commit hash)
        _run_git(["checkout", "--detach", entry.revision], cwd=dest)


def fetch_repo(entry: RepoEntry, root: Path) -> None:
    """Fetch latest from remote for a repo."""
    dest = root / entry.directory
    _run_git(["fetch", "--all", "--quiet"], cwd=dest)


def apply_readonly(dest: Path) -> None:
    """Remove write permission from all files in a repo working tree.

    Skips the .git directory so git operations still work.
    """
    for dirpath, dirnames, filenames in os.walk(dest):
        # Never touch .git internals
        if ".git" in dirnames:
            dirnames.remove(".git")
        for name in filenames:
            fpath = Path(dirpath) / name
            if fpath.is_symlink():
                continue
            mode = fpath.stat().st_mode
            fpath.chmod(mode & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH))


def restore_writable(dest: Path) -> None:
    """Restore owner write permission on all files in a repo working tree.

    Used before git operations that need to modify files (checkout, pull).
    Skips the .git directory.
    """
    for dirpath, dirnames, filenames in os.walk(dest):
        if ".git" in dirnames:
            dirnames.remove(".git")
        for name in filenames:
            fpath = Path(dirpath) / name
            if fpath.is_symlink():
                continue
            mode = fpath.stat().st_mode
            fpath.chmod(mode | stat.S_IWUSR)


def sync_repo(
    entry: RepoEntry,
    root: Path,
    *,
    verbose: bool = False,
) -> None:
    """Fetch and checkout declared revision for a repo.

    If the directory doesn't exist yet, clone it.
    """
    dest = root / entry.directory
    if not dest.exists():
        clone_repo(entry, root, verbose=verbose)
        return

    # Temporarily restore write so git can modify working tree
    if entry.is_readonly:
        restore_writable(dest)

    try:
        fetch_repo(entry, root)
        checkout_revision(entry, root)

        # For branches, also pull to fast-forward
        head_ref = get_current_ref(entry, root)
        if head_ref and not is_detached(entry, root):
            _run_git(["pull", "--ff-only", "--quiet"], cwd=dest, check=False)
    finally:
        if entry.is_readonly:
            apply_readonly(dest)


@dataclass(frozen=True, slots=True)
class RepoStatus:
    """Status information for a managed repo."""

    directory: str
    exists: bool
    current_ref: str
    expected_ref: str
    is_clean: bool
    is_detached: bool
    ahead: int
    behind: int


def get_current_ref(entry: RepoEntry, root: Path) -> str:
    """Get current branch name or commit hash."""
    dest = root / entry.directory
    # Try symbolic ref first (branch name)
    result = _run_git(
        ["symbolic-ref", "--short", "HEAD"],
        cwd=dest,
        check=False,
    )
    if result.returncode == 0:
        return result.stdout.strip()
    # Detached HEAD — return short commit hash
    result = _run_git(["rev-parse", "--short", "HEAD"], cwd=dest)
    return result.stdout.strip()


def is_detached(entry: RepoEntry, root: Path) -> bool:
    """Check whether HEAD is detached."""
    dest = root / entry.directory
    result = _run_git(
        ["symbolic-ref", "HEAD"],
        cwd=dest,
        check=False,
    )
    return result.returncode != 0


def is_clean(entry: RepoEntry, root: Path) -> bool:
    """Check whether the working tree is clean."""
    dest = root / entry.directory
    result = _run_git(
        ["status", "--porcelain"],
        cwd=dest,
    )
    return result.stdout.strip() == ""


def get_ahead_behind(
    entry: RepoEntry, root: Path
) -> tuple[int, int]:
    """Get number of commits ahead/behind the tracking branch."""
    dest = root / entry.directory
    result = _run_git(
        ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        cwd=dest,
        check=False,
    )
    if result.returncode != 0:
        return 0, 0
    parts = result.stdout.strip().split()
    if len(parts) == 2:
        return int(parts[0]), int(parts[1])
    return 0, 0


def get_repo_status(entry: RepoEntry, root: Path) -> RepoStatus:
    """Get full status for a managed repo."""
    dest = root / entry.directory
    if not dest.exists():
        return RepoStatus(
            directory=entry.directory,
            exists=False,
            current_ref="",
            expected_ref=entry.revision,
            is_clean=True,
            is_detached=False,
            ahead=0,
            behind=0,
        )

    current = get_current_ref(entry, root)
    detached = is_detached(entry, root)
    clean = is_clean(entry, root)
    ahead, behind = get_ahead_behind(entry, root)

    return RepoStatus(
        directory=entry.directory,
        exists=True,
        current_ref=current,
        expected_ref=entry.revision,
        is_clean=clean,
        is_detached=detached,
        ahead=ahead,
        behind=behind,
    )
