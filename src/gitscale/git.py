"""Git operations for managing sub-repositories."""

from __future__ import annotations

import os
import stat
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from gitscale.config import RepoEntry


def is_ci() -> bool:
    """Return True when running inside a CI environment."""
    return os.environ.get("CI", "").lower() in ("1", "true")


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
    shallow: bool = False,
) -> None:
    """Clone a repository into root/entry.directory.

    Manifest-only entries are skipped.
    When *shallow* is True, clones with ``--depth 1 --branch <revision>``.
    """
    if entry.is_manifest:
        return

    dest = root / entry.directory
    if dest.exists():
        raise GitError(f"Directory already exists: {dest}")

    args = ["clone", entry.repo_url, str(dest)]
    if shallow:
        args.extend(["--depth", "1", "--branch", entry.revision])
    if verbose:
        args.append("--progress")
    else:
        args.append("--quiet")

    _run_git(args)
    if not shallow:
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


def is_shallow(dest: Path) -> bool:
    """Return True if the repo at *dest* is a shallow clone."""
    result = _run_git(
        ["rev-parse", "--is-shallow-repository"],
        cwd=dest,
        check=False,
    )
    return result.stdout.strip() == "true"


def fetch_repo(entry: RepoEntry, root: Path) -> None:
    """Fetch latest from remote for a repo.

    Uses ``--depth 1`` for shallow clones.
    """
    dest = root / entry.directory
    if is_shallow(dest):
        _run_git(["fetch", "--depth", "1", "--quiet"], cwd=dest)
    else:
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
    shallow: bool = False,
) -> None:
    """Fetch and checkout declared revision for a repo.

    If the directory doesn't exist yet, clone it.
    Manifest-only entries are skipped.
    """
    if entry.is_manifest:
        return

    dest = root / entry.directory
    if not dest.exists():
        clone_repo(entry, root, verbose=verbose, shallow=shallow)
        return

    # Temporarily restore write so git can modify working tree
    if entry.is_readonly:
        restore_writable(dest)

    try:
        fetch_repo(entry, root)
        if is_shallow(dest):
            _run_git(
                ["reset", "--hard", "@{upstream}"],
                cwd=dest,
                check=False,
            )
        else:
            checkout_revision(entry, root)
            # For branches, also pull to fast-forward
            head_ref = get_current_ref(entry, root)
            if head_ref and not is_detached(entry, root):
                _run_git(
                    ["pull", "--ff-only", "--quiet"],
                    cwd=dest,
                    check=False,
                )
    finally:
        if entry.is_readonly:
            apply_readonly(dest)


def pull_repo(
    entry: RepoEntry,
    root: Path,
    *,
    verbose: bool = False,
    shallow: bool = False,
) -> None:
    """Pull (fetch + fast-forward) a repo.

    If the directory doesn't exist yet, clone it.
    For shallow clones: fetch --depth 1 + reset --hard.
    Manifest-only entries are skipped.
    """
    if entry.is_manifest:
        return

    dest = root / entry.directory
    if not dest.exists():
        clone_repo(entry, root, verbose=verbose, shallow=shallow)
        return

    if entry.is_readonly:
        restore_writable(dest)

    try:
        if is_shallow(dest):
            _run_git(["fetch", "--depth", "1", "--quiet"], cwd=dest)
            _run_git(
                ["reset", "--hard", "@{upstream}"],
                cwd=dest,
                check=False,
            )
        else:
            _run_git(
                ["pull", "--ff-only", "--quiet"], cwd=dest, check=False
            )
    finally:
        if entry.is_readonly:
            apply_readonly(dest)


def push_repo(
    entry: RepoEntry,
    root: Path,
    *,
    verbose: bool = False,
) -> None:
    """Push local commits to remote.

    Manifest-only and readonly entries are skipped.
    """
    if entry.is_manifest or entry.is_readonly:
        return

    dest = root / entry.directory
    if not dest.exists():
        return

    _run_git(["push", "--quiet"], cwd=dest)


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
    mode: str = ""
    is_stale: bool = False


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


def _is_stale(dest: Path) -> bool:
    """For shallow repos: check if HEAD differs from upstream hash."""
    local = _run_git(["rev-parse", "HEAD"], cwd=dest, check=False)
    remote = _run_git(
        ["rev-parse", "@{upstream}"], cwd=dest, check=False
    )
    if local.returncode != 0 or remote.returncode != 0:
        return False
    return local.stdout.strip() != remote.stdout.strip()


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
            mode=entry.mode.value,
        )

    current = get_current_ref(entry, root)
    detached = is_detached(entry, root)
    clean = is_clean(entry, root)
    shallow = is_shallow(dest)

    if shallow:
        stale = _is_stale(dest)
        return RepoStatus(
            directory=entry.directory,
            exists=True,
            current_ref=current,
            expected_ref=entry.revision,
            is_clean=clean,
            is_detached=detached,
            ahead=0,
            behind=0,
            mode=entry.mode.value,
            is_stale=stale,
        )

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
        mode=entry.mode.value,
    )


def get_self_status(root: Path) -> RepoStatus | None:
    """Get status of the repository that contains .gitscale.toml.

    Returns None if root is not a git repository.
    """
    result = _run_git(
        ["rev-parse", "--is-inside-work-tree"],
        cwd=root,
        check=False,
    )
    if result.returncode != 0:
        return None

    # Current branch/ref
    ref_result = _run_git(
        ["symbolic-ref", "--short", "HEAD"],
        cwd=root,
        check=False,
    )
    if ref_result.returncode == 0:
        current_ref = ref_result.stdout.strip()
        detached = False
    else:
        rev_result = _run_git(["rev-parse", "--short", "HEAD"], cwd=root)
        current_ref = rev_result.stdout.strip()
        detached = True

    # Clean?
    clean_result = _run_git(["status", "--porcelain"], cwd=root)
    clean = clean_result.stdout.strip() == ""

    # Ahead/behind
    ab_result = _run_git(
        ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        cwd=root,
        check=False,
    )
    ahead, behind = 0, 0
    if ab_result.returncode == 0:
        parts = ab_result.stdout.strip().split()
        if len(parts) == 2:
            ahead, behind = int(parts[0]), int(parts[1])

    return RepoStatus(
        directory=".",
        exists=True,
        current_ref=current_ref,
        expected_ref="",
        is_clean=clean,
        is_detached=detached,
        ahead=ahead,
        behind=behind,
    )


def get_manifest_status(
    entry: RepoEntry, root: Path
) -> RepoStatus:
    """Get status for a manifest-only entry.

    Compares .etag and .etag-remote to detect behind state.
    """
    dest = root / entry.directory
    manifest_file = dest / "manifest.json"
    exists = manifest_file.is_file()

    # Check if behind remote
    behind = 0
    etag_file = dest / ".etag"
    etag_remote_file = dest / ".etag-remote"
    if etag_file.is_file() and etag_remote_file.is_file():
        local = etag_file.read_text(encoding="utf-8").strip()
        remote = etag_remote_file.read_text(encoding="utf-8").strip()
        if local and remote and local != remote:
            behind = 1

    return RepoStatus(
        directory=entry.directory,
        exists=exists,
        current_ref="manifest" if exists else "",
        expected_ref=entry.revision,
        is_clean=True,
        is_detached=False,
        ahead=0,
        behind=behind,
        mode=entry.mode.value,
    )
