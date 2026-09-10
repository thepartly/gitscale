//! Cloning a sub-repository from a copy that is already on this machine.
//!
//! A linked worktree of a workspace, or a workspace cloned with `--reference`,
//! sits beside one that has already downloaded every sub-repository. Git will
//! not notice that by itself: nothing links a fresh clone to a sibling, and
//! git's worktree metadata describes the workspace only — it knows nothing
//! about the repositories `.gitscale.toml` declares inside it. So this module
//! works out which workspace the current one came from, maps each entry onto
//! the copy there, and hands `clone_repo` a path to borrow objects from.
//!
//! Only the *source workspace's own* checkouts are ever borrowed from, never a
//! sibling worktree's. Sibling worktrees get deleted routinely once their
//! branch merges; the workspace they were made from does not. An alternate
//! that disappears leaves the borrowing repository unable to read its own
//! history, and git reports nothing until something tries to read an object.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::RepoEntry;

/// A local repository a new clone may take its objects from.
#[derive(Debug, Clone)]
pub struct Reference {
    /// Passed to `git clone --reference`.
    pub path: PathBuf,
    /// Also pass `--dissociate`: copy the borrowed objects in once the clone
    /// is made and drop the link, trading the disk saving for independence
    /// from anything that later happens to `path`.
    pub dissociate: bool,
}

/// The workspace `root` was derived from, if it was derived from one.
///
/// Two different mechanisms can put an earlier copy next to this one, and they
/// leave completely different traces:
///
/// * `git worktree add` — no alternates file anywhere; the link is the `.git`
///   file pointing at the main worktree's git directory.
/// * `git clone --reference` — no worktree relationship; the link is
///   `objects/info/alternates`.
///
/// Checking only one of them silently does nothing for half the users.
pub fn source_workspace(root: &Path) -> Option<PathBuf> {
    main_worktree(root).or_else(|| alternates_workspace(root))
}

/// The main worktree of the repository at `root`, when `root` is some *other*
/// worktree of it.
fn main_worktree(root: &Path) -> Option<PathBuf> {
    let listing = git_query(&["worktree", "list", "--porcelain"], root)?;
    // `worktree list` always names the main worktree first, whichever worktree
    // it is asked from. That ordering is what makes the "never borrow from a
    // sibling" rule automatic rather than something to enforce separately.
    let first = listing.lines().next()?.strip_prefix("worktree ")?;
    let main = PathBuf::from(first);
    (!same_dir(&main, root)).then_some(main)
}

/// The workspace whose object store `root` already borrows from.
fn alternates_workspace(root: &Path) -> Option<PathBuf> {
    // Ask git for the path rather than building `<root>/.git/objects/...`: in
    // a worktree `.git` is a file, so the hand-built path matches nothing and
    // the check quietly fails instead of erroring.
    let relative = git_query(
        &["rev-parse", "--git-path", "objects/info/alternates"],
        root,
    )?;
    let contents = std::fs::read_to_string(root.join(relative)).ok()?;
    let objects = contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;
    workspace_of_object_store(Path::new(objects))
}

/// `<workspace>/.git/objects` back to `<workspace>`. Any other shape — a bare
/// repository, a path git wrote in some form we do not recognise — is not a
/// workspace whose layout entries can be mapped onto, so it yields nothing.
fn workspace_of_object_store(objects: &Path) -> Option<PathBuf> {
    if objects.file_name()? != "objects" {
        return None;
    }
    let git_dir = objects.parent()?;
    if git_dir.file_name()? != ".git" {
        return None;
    }
    Some(git_dir.parent()?.to_path_buf())
}

/// The copy of `entry` inside `source` worth cloning from, if there is one.
pub fn reference_for(source: &Path, entry: &RepoEntry, dissociate: bool) -> Option<Reference> {
    if entry.is_artefact() {
        // Not a clone at all — an unpacked archive with no object store.
        return None;
    }
    let candidate = source.join(&entry.directory);
    // A symlink here is one `resolve` created to dedupe a recursive
    // dependency, pointing at a checkout declared elsewhere in the workspace.
    // That checkout gets its own entry, and referencing it twice under two
    // names would be pointless rather than wrong.
    if candidate.is_symlink() || !candidate.is_dir() {
        return None;
    }
    // Occupying the same relative path does not make it the same repository.
    // Without this, two unrelated workspaces that both keep something at
    // `libs/core` would graft one's history onto the other.
    let origin = crate::git::origin_url(&candidate)?;
    if crate::urls::normalize(&origin) != crate::urls::normalize(&entry.repo_url) {
        return None;
    }
    Some(Reference {
        path: candidate,
        dissociate,
    })
}

/// Compare two paths by where they actually land, so that a symlinked or
/// non-normalised spelling of the current workspace is not mistaken for a
/// different one and borrowed from itself.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Run a read-only git query. Every caller treats failure as "no source
/// available", so a missing git, a directory that is not a repository and a
/// git too old for the subcommand all collapse to the same answer.
fn git_query(args: &[&str], cwd: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_store_maps_back_to_its_workspace() {
        assert_eq!(
            workspace_of_object_store(Path::new("/home/j/partly/.git/objects")),
            Some(PathBuf::from("/home/j/partly"))
        );
    }

    #[test]
    fn a_bare_repository_is_not_a_workspace() {
        // No working tree to map `libs/core` onto.
        assert_eq!(
            workspace_of_object_store(Path::new("/srv/mirrors/core.git/objects")),
            None
        );
    }

    #[test]
    fn an_unrecognised_alternate_path_yields_nothing() {
        assert_eq!(
            workspace_of_object_store(Path::new("/some/pack/directory")),
            None
        );
    }
}
