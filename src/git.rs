use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::ci;
use crate::config::RepoEntry;
use crate::share::{Pinned, Source};

pub fn is_ci() -> bool {
    std::env::var("CI")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true"))
        .unwrap_or(false)
}

/// Variables that tell git which repository to operate on, whatever the working
/// directory — the location half of `git rev-parse --local-env-vars`. The
/// config half (`GIT_CONFIG_PARAMETERS`, `GIT_CONFIG_COUNT`) is kept: CI
/// runners pass credentials through it, and git's own submodule code keeps it
/// for the same reason.
const REPO_LOCATION_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_SHALLOW_FILE",
    "GIT_GRAFT_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_CONFIG",
];

/// A `git` command that finds its repository from the working directory.
///
/// gitscale runs inside git hooks, and git exports `GIT_DIR` to them — during
/// `git clone`, as the absolute path of the new clone's `.git`. Inherited, it
/// sends every command meant for a sub-repository to the root instead: a
/// pinned tag "does not exist" because the root has no such tag, and a
/// checkout of a branch the root does have moves the root.
pub(crate) fn git_command() -> Command {
    let mut cmd = Command::new("git");
    for var in REPO_LOCATION_ENV {
        cmd.env_remove(var);
    }
    cmd
}

pub(crate) fn run_git(
    args: &[&str],
    cwd: Option<&Path>,
    check: bool,
) -> Result<std::process::Output> {
    let mut cmd = git_command();
    // Under CI, teach git how to authenticate to the CI server. Scoped to that
    // one host, and carrying the name of the token variable rather than the
    // token, so nothing secret reaches argv or a config file.
    if let Some(auth) = ci::active() {
        cmd.args(auth.git_config_args());
    }
    cmd.args(args);
    cmd.stdin(Stdio::null());
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    // Marks every git call gitscale makes, so an installed gitscale git hook
    // can tell re-entry from a genuine user operation and bail out. Without
    // it, a hook that runs `gitscale pull` recurses without bound.
    cmd.env("GITSCALE_HOOK", "1");
    cmd.env("GIT_ASKPASS", "");
    cmd.env("SSH_ASKPASS", "");
    cmd.env("SSH_ASKPASS_REQUIRE", "never");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd
        .output()
        .with_context(|| format!("failed to run: git {}", args.join(" ")))?;
    if check && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{}", git_failure(&stderr, ci::active()));
    }
    Ok(output)
}

/// Pick the most relevant line from git/ssh stderr output. Advisory notices
/// (e.g. openssh's post-quantum key exchange warning) print early, ahead of
/// the actual failure, so a plain "first line" pick reports the wrong thing.
fn git_error_line(stderr: &str) -> &str {
    let trimmed = stderr.trim();
    trimmed
        .lines()
        .find(|l| l.contains("fatal:") || l.contains("error:"))
        .or_else(|| trimmed.lines().last())
        .unwrap_or("unknown error")
}

/// The message to report for a failed git call. Two failures need more than
/// git's own wording. A 403 from the CI server: the token is valid but the
/// target project has not allowed this one to read it. And an ssh key refused
/// over SSH: often one whose passphrase gitscale could not ask for.
fn git_failure(stderr: &str, auth: Option<&crate::ci::CiAuth>) -> String {
    let line = git_error_line(stderr);
    let hint = match auth {
        Some(auth) if stderr.contains("403") => Some(auth.forbidden_hint()),
        _ => crate::ssh::failure_hint(stderr),
    };
    match hint {
        Some(hint) => format!("{}\nhint: {}", line, hint),
        None => line.to_string(),
    }
}

fn stdout_str(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A read-only git query in `dir`: its trimmed output, or `None` when git
/// fails — no repository there, no such ref, a git too old for the command.
/// Callers that need to tell those apart use [`run_git`].
pub(crate) fn query(dir: &Path, args: &[&str]) -> Option<String> {
    let output = run_git(args, Some(dir), false).ok()?;
    output.status.success().then(|| stdout_str(&output))
}

/// Point `dir`'s existing `origin` at `url`. `Ok(false)` when it already did,
/// or when there is no `origin` to repoint.
pub(crate) fn set_origin(dir: &Path, url: &str) -> Result<bool> {
    match origin_url(dir) {
        Some(current) if current != url => {
            run_git(&["remote", "set-url", "origin", url], Some(dir), true)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// Core operations
// ---------------------------------------------------------------------------

/// A commit spelled out in full: 40 hex digits for SHA-1, 64 for SHA-256 (a
/// repository made with `--object-format=sha256`).
///
/// The one way a revision names a commit. Anything else is a branch or tag
/// name, so a tag called `20241001` is a tag, not a commit: an abbreviated SHA
/// cannot be told apart from such a name, and git cannot expand one without
/// the history a shallow clone or an artefact entry never downloads. `git clone
/// --branch` accepts only branch and tag names, so a SHA-pinned entry cannot be
/// cloned shallowly the usual way.
pub fn is_full_sha(revision: &str) -> bool {
    matches!(revision.len(), 40 | 64) && revision.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether `revision` looks like an abbreviated commit: hex digits only, but
/// too short to be a full SHA. Taken as a branch or tag name like any other;
/// this only decides whether a failure to find one says why.
fn is_abbreviated_sha(revision: &str) -> bool {
    revision.len() >= 7 && !is_full_sha(revision) && revision.chars().all(|c| c.is_ascii_hexdigit())
}

/// `error`, with a hint when the revision that could not be found looks like
/// an abbreviated commit — the likeliest reason, and one git cannot name.
pub(crate) fn with_revision_hint(revision: &str, error: anyhow::Error) -> anyhow::Error {
    let message = format!("{:#}", error);
    if !is_abbreviated_sha(revision) || message.contains("\nhint: ") {
        return error;
    }
    anyhow::anyhow!(
        "{}\nhint: '{}' is not a branch or tag. A commit must be given as its full SHA \
         (40 or 64 hex digits): run `git rev-parse {}` in a checkout of the repository",
        message,
        revision,
        revision
    )
}

/// The commit `revision` names in the repository at `url`, asked of the remote
/// itself: one ref advertisement, no objects.
///
/// A full SHA is its own answer. An empty revision is the remote's default
/// branch, whatever it is called today. Anything else must be a branch or a
/// tag, matched by its exact name — `ls-remote`'s own patterns match any ref
/// ending in the name, so `main` would also find `feature/main` — and a tag
/// resolves to the commit it points at, annotated or not.
pub fn resolve_remote_commit(url: &str, revision: &str) -> Result<String> {
    if is_full_sha(revision) {
        return Ok(revision.to_lowercase());
    }
    if revision.is_empty() {
        let listed = run_git(&["ls-remote", "--symref", url, "HEAD"], None, true)?;
        let text = String::from_utf8_lossy(&listed.stdout);
        return text
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .find(|(sha, name)| *name == "HEAD" && is_full_sha(sha))
            .map(|(sha, _)| sha.to_string())
            .ok_or_else(|| anyhow::anyhow!("{} has no default branch", url));
    }
    let listed = ls_remote_revision(url, revision, true)?;
    listed.map(|(sha, _)| sha).ok_or_else(|| {
        with_revision_hint(
            revision,
            anyhow::anyhow!("'{}' is not a branch or tag of {}", revision, url),
        )
    })
}

/// The commit a branch or tag named `revision` points at on the remote, and
/// whether it is a tag — `None` when the remote has neither.
///
/// Matched by exact name: `ls-remote`'s own patterns match any ref *ending*
/// in the name, so `main` would also find `zzz/main`. A branch wins over a tag
/// of the same name, and a tag resolves to the commit it points at: the peeled
/// line of an annotated tag is listed only when asked for by name, and without
/// it the answer is the tag object. With `check` false, a failing `ls-remote`
/// is `None` too.
pub(crate) fn ls_remote_revision(
    url: &str,
    revision: &str,
    check: bool,
) -> Result<Option<(String, bool)>> {
    let (branch, tag) = if revision.starts_with("refs/") {
        (revision.to_string(), revision.to_string())
    } else {
        (
            format!("refs/heads/{}", revision),
            format!("refs/tags/{}", revision),
        )
    };
    let peeled_name = format!("{}^{{}}", tag);
    let listed = run_git(
        &["ls-remote", url, &branch, &tag, &peeled_name],
        None,
        check,
    )?;
    if !listed.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&listed.stdout);
    let (mut on_branch, mut peeled, mut on_tag) = (None, None, None);
    for (sha, name) in text.lines().filter_map(|line| line.split_once('\t')) {
        if name == branch && branch != tag {
            on_branch = Some(sha);
        } else if name == peeled_name {
            peeled = Some(sha);
        } else if name == tag {
            on_tag = Some(sha);
        }
    }
    Ok(match (on_branch, peeled.or(on_tag)) {
        (Some(sha), _) => Some((sha.to_string(), false)),
        // Spelled `refs/heads/…`, the one name is a branch's, not a tag's.
        (None, Some(sha)) => Some((sha.to_string(), !revision.starts_with("refs/heads/"))),
        (None, None) => None,
    })
}

/// Every branch and tag of the remote at `url`, as `(commit, name)` with the
/// `refs/heads/` or `refs/tags/` taken off. An annotated tag is listed under
/// the commit it points at, not the tag object.
pub fn ls_remote_refs(url: &str) -> Result<Vec<(String, String)>> {
    let listed = run_git(&["ls-remote", "--heads", "--tags", url], None, true)?;
    let text = String::from_utf8_lossy(&listed.stdout);
    let lines: Vec<(&str, &str)> = text.lines().filter_map(|l| l.split_once('\t')).collect();
    let peeled: std::collections::HashSet<&str> = lines
        .iter()
        .filter_map(|(_, name)| name.strip_suffix("^{}"))
        .collect();
    Ok(lines
        .iter()
        .filter(|(_, name)| !peeled.contains(name))
        .map(|(sha, name)| {
            let name = name.strip_suffix("^{}").unwrap_or(name);
            let short = name
                .strip_prefix("refs/heads/")
                .or_else(|| name.strip_prefix("refs/tags/"))
                .unwrap_or(name);
            (sha.to_string(), short.to_string())
        })
        .collect())
}

/// A commit as people read it: its first 7 hex digits, git's (and GitHub's)
/// default abbreviation. For display only — never resolve one of these.
pub fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// Shallow-clone a repo pinned to a commit SHA: init an empty repo and fetch
/// just that commit. Returns `false` if the remote refused to serve the commit
/// (not every server allows fetching an arbitrary SHA), leaving the caller to
/// fall back to a full clone.
fn shallow_clone_at_sha(entry: &RepoEntry, dest: &Path, verbose: bool) -> Result<bool> {
    let progress = if verbose { "--progress" } else { "--quiet" };
    fs::create_dir_all(dest).with_context(|| format!("cannot create {}", dest.display()))?;
    run_git(&["init", "--quiet"], Some(dest), true)?;
    run_git(
        &["remote", "add", "origin", &remote_url(entry)],
        Some(dest),
        true,
    )?;
    let fetched = run_git(
        &["fetch", "--depth", "1", progress, "origin", &entry.revision],
        Some(dest),
        false,
    )?;
    if !fetched.status.success() {
        return Ok(false);
    }
    run_git(
        &["checkout", "--detach", progress, "FETCH_HEAD"],
        Some(dest),
        true,
    )?;
    Ok(true)
}

/// Whether `dir` is a checkout of its own, rather than just a directory.
///
/// Existing is not enough. A failed clone, an interrupted delete or an outside
/// cleaner can leave an entry's directory behind with no repository in it,
/// and git run there does not stop at it: it walks up and finds the
/// workspace's own repository. Every git command gitscale meant for the
/// checkout would then land on the workspace instead — a pull checking out the
/// pinned branch over the user's, a commit or a push of the wrong repository.
/// A `.git` directory, or the `.git` file of a linked worktree, is what makes
/// git stop.
pub fn is_checkout(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Clone `entry` into `root`.
///
/// `source` says where the objects come from: the remote, a copy already on
/// this machine whose objects the new clone borrows, or a pinned commit in a
/// snapshot cache entry. See [`crate::share`] and [`crate::cache`] for how one
/// is worked out; [`Source::default`] always produces an ordinary clone.
pub fn clone_repo(entry: &RepoEntry, root: &Path, verbose: bool, source: &Source) -> Result<()> {
    check_names_a_ref(entry)?;
    clone_repo_at(entry, root, verbose, source).map_err(|e| with_revision_hint(&entry.revision, e))
}

/// Refuse an abbreviated commit before git gets to expand it. A full clone
/// would — it has the history — while a shallow one cannot, so the same entry
/// would work on a developer machine and fail in CI. A revision that only
/// looks like one, an all-hex branch or tag name, is checked with the remote
/// and goes ahead.
fn check_names_a_ref(entry: &RepoEntry) -> Result<()> {
    if entry.is_artefact() || !is_abbreviated_sha(&entry.revision) {
        return Ok(());
    }
    let url = remote_url(entry);
    if ls_remote_revision(&url, &entry.revision, true)?.is_some() {
        return Ok(());
    }
    Err(with_revision_hint(
        &entry.revision,
        anyhow::anyhow!("'{}' is not a branch or tag of {}", entry.revision, url),
    ))
}

fn clone_repo_at(entry: &RepoEntry, root: &Path, verbose: bool, source: &Source) -> Result<()> {
    if entry.is_artefact() {
        return Ok(());
    }
    let dest = root.join(&entry.directory);
    if dest.exists() {
        // An empty directory is what a clone that never finished leaves, and
        // holds nothing to lose. Anything else is somebody's.
        let empty = fs::read_dir(&dest)
            .map(|mut listing| listing.next().is_none())
            .unwrap_or(false);
        if !empty || dest.is_symlink() {
            if is_checkout(&dest) {
                bail!("Directory already exists: {}", dest.display());
            }
            bail!(
                "{} exists but holds no git repository; `gitscale clean -f {}` removes it",
                dest.display(),
                entry.directory
            );
        }
        fs::remove_dir(&dest).with_context(|| format!("cannot remove {}", dest.display()))?;
    }

    if let Some(pinned) = &source.pinned {
        clone_pinned(entry, root, &dest, pinned, verbose)?;
    } else if source.shallow && is_full_sha(&entry.revision) {
        // This path builds the repository with `init` + a one-commit `fetch`
        // rather than `clone`, and there is no `--reference` for fetch. Little
        // is lost: a depth-1 fetch of a single commit transfers about as much
        // as wiring up an alternate would save.
        if !shallow_clone_at_sha(entry, &dest, verbose)? {
            // Remote would not serve the bare commit; retry unshallowed.
            fs::remove_dir_all(&dest)
                .with_context(|| format!("cannot clean up {}", dest.display()))?;
            let deep = Source {
                shallow: false,
                ..source.clone()
            };
            return clone_repo(entry, root, verbose, &deep);
        }
    } else {
        let dest_str = dest.to_string_lossy().to_string();
        let url = remote_url(entry);
        let reference_path = source
            .reference
            .as_ref()
            .map(|r| r.path.to_string_lossy().to_string());
        let mut args: Vec<&str> = vec!["clone", &url, &dest_str];
        if source.shallow && !entry.revision.is_empty() {
            args.extend_from_slice(&["--depth", "1", "--branch", &entry.revision]);
        } else if source.shallow {
            args.extend_from_slice(&["--depth", "1"]);
        }
        if let (Some(reference), Some(path)) =
            (source.reference.as_ref(), reference_path.as_deref())
        {
            args.extend_from_slice(&["--reference", path]);
            if reference.dissociate {
                args.push("--dissociate");
            }
        }
        if verbose {
            args.push("--progress");
        } else {
            args.push("--quiet");
        }
        run_git(&args, None, true)?;

        if !source.shallow && !entry.revision.is_empty() {
            checkout_revision(entry, root)?;
        }
    }
    if entry.is_readonly() {
        apply_readonly(&dest)?;
    }
    Ok(())
}

/// Clone `url` into `dest`, borrowing objects from `reference` when there is
/// one. Used to bootstrap a workspace, where there is no entry to describe the
/// repository yet — only a URL the user typed.
pub fn clone_url(url: &str, dest: &Path, reference: Option<&Path>, verbose: bool) -> Result<()> {
    let dest_str = dest.to_string_lossy().to_string();
    let reference_str = reference.map(|p| p.to_string_lossy().to_string());
    let mut args: Vec<&str> = vec!["clone", url, &dest_str];
    if let Some(path) = reference_str.as_deref() {
        args.extend_from_slice(&["--reference", path]);
    }
    args.push(if verbose { "--progress" } else { "--quiet" });
    run_git(&args, None, true)?;
    Ok(())
}

/// Build a checkout from a snapshot cache entry.
///
/// An ordinary local clone of one pinned commit: it copies the objects instead
/// of borrowing them, so the entry can be evicted — or the whole cache deleted
/// — under a running job without it noticing. Git refuses a shallow repository
/// as a `--reference`, so copying is not merely the safer choice here, it is
/// the only one.
fn clone_pinned(
    entry: &RepoEntry,
    root: &Path,
    dest: &Path,
    pinned: &Pinned,
    verbose: bool,
) -> Result<()> {
    let progress = if verbose { "--progress" } else { "--quiet" };
    let from = pinned.entry.to_string_lossy().to_string();
    let dest_str = dest.to_string_lossy().to_string();
    run_git(
        &[
            "clone",
            // Without this the job clones every other pin in the entry too.
            "--single-branch",
            "--branch",
            &pinned.reference,
            progress,
            &from,
            &dest_str,
        ],
        None,
        true,
    )?;
    // `origin` currently names the cache entry. The workspace's remote has to
    // be the real one, for pushes and for anything the user runs by hand.
    reconcile_remote(entry, root)?;
    // Land where a `--depth 1 --branch` clone would have: on the branch the
    // config names, or detached for a tag or a SHA.
    if pinned.detach {
        run_git(
            &["checkout", "--detach", progress, &pinned.sha],
            Some(dest),
            true,
        )?;
    } else {
        run_git(
            &["checkout", progress, "-B", &entry.revision, &pinned.sha],
            Some(dest),
            true,
        )?;
    }
    run_git(&["branch", "-D", &pinned.reference], Some(dest), false)?;
    Ok(())
}

/// Ensure an existing clone's `origin` remote URL matches the configured URL.
/// Returns `true` if the remote was updated.
pub fn reconcile_remote(entry: &RepoEntry, root: &Path) -> Result<bool> {
    if entry.is_artefact() {
        return Ok(false);
    }
    let dest = root.join(&entry.directory);
    if !is_checkout(&dest) {
        return Ok(false);
    }
    set_origin(&dest, &remote_url(entry))
}

/// The `origin` URL of the repository `dir` belongs to, or `None` if it is not
/// in one or has no such remote. Resolved by git rather than by looking for a
/// `.git` directory, so a path inside a repository answers for that repository.
pub fn origin_url(dir: &Path) -> Option<String> {
    query(dir, &["remote", "get-url", "origin"]).filter(|url| !url.is_empty())
}

/// The URL to use as `origin` for `entry`: the configured one, unless CI
/// credentials cover its host and can fetch it over HTTPS instead.
pub fn remote_url(entry: &RepoEntry) -> String {
    ci::remote_url(&entry.repo_url)
}

/// Repoint an existing clone at the CI server's HTTPS URL before a network
/// operation. A no-op outside CI, and for any host the CI server doesn't own:
/// a workspace cloned over SSH (restored from cache, or checked out by the
/// runner itself) otherwise keeps failing on an SSH key the job doesn't have.
fn ensure_ci_remote(entry: &RepoEntry, dest: &Path) -> Result<()> {
    let Some(auth) = ci::active() else {
        return Ok(());
    };
    let Some(url) = auth.remote_url(&entry.repo_url) else {
        return Ok(());
    };
    set_origin(dest, &url)?;
    Ok(())
}

pub fn checkout_revision(entry: &RepoEntry, root: &Path) -> Result<()> {
    let dest = root.join(&entry.directory);
    // Try branch checkout first, then detached HEAD for tags/SHAs
    let result = run_git(&["checkout", &entry.revision], Some(&dest), false)?;
    if !result.status.success() {
        if !ref_exists(&dest, &format!("{}^{{commit}}", entry.revision)) {
            bail!(
                "revision '{}' does not exist in {}",
                entry.revision,
                entry.directory
            );
        }
        run_git(
            &["checkout", "--detach", &entry.revision],
            Some(&dest),
            true,
        )?;
    }
    Ok(())
}

pub fn is_shallow(dest: &Path) -> bool {
    run_git(&["rev-parse", "--is-shallow-repository"], Some(dest), false)
        .map(|o| stdout_str(&o) == "true")
        .unwrap_or(false)
}

pub fn fetch_repo(entry: &RepoEntry, root: &Path, source: &Source) -> Result<()> {
    check_names_a_ref(entry)?;
    refresh(entry, &root.join(&entry.directory), source)
        .map_err(|e| with_revision_hint(&entry.revision, e))
}

/// Bring into `dest` the refs `entry.revision` needs, without moving the
/// checkout: the one place `fetch` and `pull` decide between cache and remote,
/// shallow and full.
///
/// From the cache entry when there is one, which was itself just updated from
/// the remote — except a shallow checkout pinned to a SHA: an arbitrary
/// commit is not something a mirror serves by default, so that one asks the
/// remote. A shallow checkout fetches only what its revision names, at depth 1;
/// a full one fetches its remote.
fn refresh(entry: &RepoEntry, dest: &Path, source: &Source) -> Result<()> {
    let shallow = is_shallow(dest);
    let sha_pin = shallow && is_full_sha(&entry.revision) && source.pinned.is_none();
    if source.local.is_some() && !sha_pin {
        return fetch_from_cache(dest, source);
    }
    ensure_ci_remote(entry, dest)?;
    if shallow {
        fetch_shallow(entry, dest)
    } else {
        run_git(&["fetch", "--quiet"], Some(dest), true)?;
        Ok(())
    }
}

/// Step two of cache-first: take locally everything the entry just fetched
/// from the remote.
///
/// `origin` in the workspace stays the real URL, so pushes and a hand-run
/// `git fetch` still go where they always did — only gitscale's own refresh
/// reads from the cache.
fn fetch_from_cache(dest: &Path, source: &Source) -> Result<()> {
    let Some(local) = &source.local else {
        return Ok(());
    };
    let from = local.to_string_lossy().to_string();
    if let Some(pinned) = &source.pinned {
        run_git(
            &["fetch", "--depth", "1", "--quiet", &from, &pinned.reference],
            Some(dest),
            true,
        )?;
        return Ok(());
    }
    let mut args = vec!["fetch", "--quiet"];
    // A checkout that is already shallow keeps its shape. Deepening one
    // behind the user's back is not this command's business — and a checkout
    // made shallow before there was a cache is exactly what this meets.
    if is_shallow(dest) {
        args.extend_from_slice(&["--depth", "1"]);
    }
    args.extend_from_slice(&[
        &from,
        "+refs/heads/*:refs/remotes/origin/*",
        "+refs/tags/*:refs/tags/*",
    ]);
    run_git(&args, Some(dest), true)?;
    Ok(())
}

pub fn apply_readonly(dest: &Path) -> Result<()> {
    set_write_bits(dest, false, &|path| is_inside_dotgit(dest, path))
}

pub fn restore_writable(dest: &Path) -> Result<()> {
    set_write_bits(dest, true, &|path| is_inside_dotgit(dest, path))
}

/// Clear every write bit (`writable = false`) or restore the owner's
/// (`writable = true`) on each file under `dest`, at any depth. Symlinks are
/// left alone, and so is anything `skip` names; a skipped directory is still
/// walked.
pub(crate) fn set_write_bits(
    dest: &Path,
    writable: bool,
    skip: &dyn Fn(&Path) -> bool,
) -> Result<()> {
    for path in walkdir(dest) {
        let path = path.as_path();
        if path.is_symlink() || skip(path) || !path.is_file() {
            continue;
        }
        let mut perms = fs::metadata(path)?.permissions();
        let mode = perms.mode();
        perms.set_mode(if writable {
            mode | 0o200 // add S_IWUSR
        } else {
            mode & !0o222 // remove S_IWUSR | S_IWGRP | S_IWOTH
        });
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

fn is_inside_dotgit(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().any(|c| c.as_os_str() == ".git")
}

fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    walkdir_recursive(root, &mut result);
    result
}

fn walkdir_recursive(dir: &Path, result: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip .git directories
        if path.is_dir() && path.file_name().map(|n| n == ".git").unwrap_or(false) {
            continue;
        }
        result.push(path.clone());
        if path.is_dir() && !path.is_symlink() {
            walkdir_recursive(&path, result);
        }
    }
}

/// Fast-forward the checked-out branch to its just-refreshed upstream — a
/// local move, checked: a merge that fails (local changes in the way) is an
/// error, not a checkout reported as updated.
///
/// Only a branch that is behind is moved. One that is up to date, ahead or has
/// diverged is left alone rather than forced, and so is a detached HEAD or a
/// branch with no upstream: there is nothing to fast-forward to.
fn fast_forward(dest: &Path) -> Result<()> {
    if !ref_exists(dest, "@{upstream}") {
        return Ok(());
    }
    let behind = run_git(
        &["merge-base", "--is-ancestor", "HEAD", "@{upstream}"],
        Some(dest),
        false,
    )?
    .status
    .success();
    if behind {
        run_git(
            &["merge", "--ff-only", "--quiet", "@{upstream}"],
            Some(dest),
            true,
        )?;
    }
    Ok(())
}

/// Bring `entry` up to date, cloning it first if it is not there yet — which
/// is the usual case in a freshly created worktree, where the hook-triggered
/// pull is the first thing to run. `reference` is used only for that clone.
pub fn pull_repo(entry: &RepoEntry, root: &Path, verbose: bool, source: &Source) -> Result<()> {
    check_names_a_ref(entry)?;
    pull_repo_at(entry, root, verbose, source).map_err(|e| with_revision_hint(&entry.revision, e))
}

fn pull_repo_at(entry: &RepoEntry, root: &Path, verbose: bool, source: &Source) -> Result<()> {
    if entry.is_artefact() {
        return Ok(());
    }
    let dest = root.join(&entry.directory);
    if !is_checkout(&dest) {
        return clone_repo(entry, root, verbose, source);
    }

    if entry.is_readonly() {
        restore_writable(&dest)?;
    }

    let result = (|| -> Result<()> {
        refresh(entry, &dest, source)?;
        if let Some(pinned) = &source.pinned {
            // CI, cached: the commit came out of the snapshot entry, so a job
            // that runs after another has nothing at all to download.
            run_git(
                &["reset", "--hard", "--quiet", &pinned.sha],
                Some(&dest),
                true,
            )?;
        } else if is_shallow(&dest) {
            if is_full_sha(&entry.revision) {
                // Detach rather than reset: a checkout moved here from a
                // branch pin would otherwise drag that branch to this commit.
                run_git(
                    &["checkout", "--quiet", "-f", "--detach", "FETCH_HEAD"],
                    Some(&dest),
                    true,
                )?;
            } else {
                land_shallow(entry, &dest)?;
            }
        } else {
            // No revision means whatever branch the clone landed on, as in
            // `clone_repo`: there is nothing to switch to, only to
            // fast-forward.
            if !entry.revision.is_empty() && get_current_ref(entry, root)? != entry.revision {
                checkout_revision(entry, root)?;
            }
            fast_forward(&dest)?;
        }
        Ok(())
    })();

    if entry.is_readonly() {
        apply_readonly(&dest)?;
    }
    result
}

/// Bring what `entry.revision` names into a shallow checkout from the remote,
/// at depth 1 and nothing more: a commit lands in `FETCH_HEAD`, a branch or tag
/// in its own ref, and no revision means the one branch the clone tracks.
fn fetch_shallow(entry: &RepoEntry, dest: &Path) -> Result<()> {
    if entry.revision.is_empty() {
        run_git(&["fetch", "--depth", "1", "--quiet"], Some(dest), true)?;
    } else if is_full_sha(&entry.revision) {
        run_git(
            &[
                "fetch",
                "--depth",
                "1",
                "--quiet",
                "origin",
                &entry.revision,
            ],
            Some(dest),
            true,
        )?;
    } else {
        fetch_shallow_revision(entry, dest)?;
    }
    Ok(())
}

/// Fetch the ref `entry.revision` names into a shallow checkout, at depth 1 —
/// a branch into its tracking ref, a tag as itself.
///
/// Named explicitly because a shallow checkout's default refspec is the one
/// branch it was cloned at: a plain `fetch` never brings a tag, or another
/// branch, that the config has since moved to.
fn fetch_shallow_revision(entry: &RepoEntry, dest: &Path) -> Result<()> {
    let rev = &entry.revision;
    // Branch first, then tag: the order `clone --branch` resolves a name in.
    let specs = [
        format!("+refs/heads/{0}:refs/remotes/origin/{0}", rev),
        format!("+refs/tags/{0}:refs/tags/{0}", rev),
    ];
    for spec in &specs {
        let fetched = run_git(
            &[
                "fetch",
                "--depth",
                "1",
                "--quiet",
                "--no-tags",
                "origin",
                spec,
            ],
            Some(dest),
            false,
        )?;
        if fetched.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&fetched.stderr);
        // Anything but "no such ref" is a real failure — auth, network — and
        // must not be reported as a revision that does not exist.
        if !stderr.contains("couldn't find remote ref") {
            bail!("{}", git_failure(&stderr, ci::active()));
        }
    }
    bail!(
        "revision '{}' does not exist in {}",
        entry.revision,
        entry.directory
    );
}

/// Move a shallow checkout to `entry.revision`, leaving it where
/// `clone --depth 1 --branch` would have: on the branch and tracking it, or
/// detached at a tag. Tracked files are forced to match, as `reset --hard`
/// did; files that do not change keep their mtimes.
fn land_shallow(entry: &RepoEntry, dest: &Path) -> Result<()> {
    let rev = &entry.revision;
    if rev.is_empty() {
        // Detached, or a branch with no upstream: nothing to follow. Otherwise
        // the reset is checked, like every other move here.
        if ref_exists(dest, "@{upstream}") {
            run_git(&["reset", "--hard", "@{upstream}"], Some(dest), true)?;
        }
        return Ok(());
    }
    let tracking = format!("refs/remotes/origin/{}", rev);
    if ref_exists(dest, &tracking) {
        run_git(
            &["checkout", "--quiet", "-f", "-B", rev, &tracking],
            Some(dest),
            true,
        )?;
        track_branch(dest, rev)?;
        return Ok(());
    }
    let tag = format!("refs/tags/{}", rev);
    if ref_exists(dest, &format!("{}^{{commit}}", tag)) {
        run_git(
            &["checkout", "--quiet", "-f", "--detach", &tag],
            Some(dest),
            true,
        )?;
        return Ok(());
    }
    bail!(
        "revision '{}' does not exist in {}",
        entry.revision,
        entry.directory
    );
}

/// What `name` resolves to in `dir` (a ref, `HEAD`, `<rev>^{commit}`…), or
/// `None` when it does not.
pub(crate) fn resolve_ref(dir: &Path, name: &str) -> Option<String> {
    query(dir, &["rev-parse", "--verify", "--quiet", name])
}

pub(crate) fn ref_exists(dir: &Path, name: &str) -> bool {
    resolve_ref(dir, name).is_some()
}

/// Make `branch` track `origin/<branch>`, as a clone at that branch does.
///
/// A single-branch clone's refspec maps only the branch it was cloned at, and
/// `@{upstream}` resolves only through a refspec — so a checkout moved to
/// another branch needs one for it, or status could no longer count ahead and
/// behind.
fn track_branch(dest: &Path, branch: &str) -> Result<()> {
    let spec = format!("+refs/heads/{0}:refs/remotes/origin/{0}", branch);
    let listed = run_git(
        &["config", "--get-all", "remote.origin.fetch"],
        Some(dest),
        false,
    )?;
    let covered = stdout_str(&listed)
        .lines()
        .any(|line| line == spec || line == "+refs/heads/*:refs/remotes/origin/*");
    if !covered {
        run_git(
            &["config", "--add", "remote.origin.fetch", &spec],
            Some(dest),
            true,
        )?;
    }
    run_git(
        &["config", &format!("branch.{}.remote", branch), "origin"],
        Some(dest),
        true,
    )?;
    run_git(
        &[
            "config",
            &format!("branch.{}.merge", branch),
            &format!("refs/heads/{}", branch),
        ],
        Some(dest),
        true,
    )?;
    Ok(())
}

pub fn push_repo(entry: &RepoEntry, root: &Path, _verbose: bool) -> Result<()> {
    if entry.is_artefact() || entry.is_readonly() {
        return Ok(());
    }
    let dest = root.join(&entry.directory);
    if !is_checkout(&dest) {
        return Ok(());
    }
    ensure_ci_remote(entry, &dest)?;
    run_git(&["push", "--quiet"], Some(&dest), true)?;
    Ok(())
}

/// Stage all changes (including untracked) and commit them with `message`.
/// Returns `Ok(true)` if a commit was created, `Ok(false)` if the working tree
/// was already clean (nothing to commit).
pub fn commit_path(dir: &Path, message: &str) -> Result<bool> {
    if !is_checkout(dir) {
        return Ok(false);
    }
    // Nothing to commit if the working tree is clean.
    let porcelain = run_git(&["status", "--porcelain"], Some(dir), true)?;
    if stdout_str(&porcelain).is_empty() {
        return Ok(false);
    }
    run_git(&["add", "-A"], Some(dir), true)?;
    run_git(&["commit", "-m", message], Some(dir), true)?;
    Ok(true)
}

/// Returns true when `dir` is the top level of its own git repository (not
/// merely nested inside some ancestor git repo).
/// Remove untracked files from `dir`'s working tree, returning the paths
/// removed (or, when `force` is false, the paths that would be).
///
/// `excludes` are gitignore-syntax patterns handed to `git clean -e`
/// unchanged, so each one means what the same text on a `.gitignore` line
/// means — matching at any depth unless it is anchored with a leading `/`,
/// and directories only when it ends in `/`.
///
/// `-ff` is deliberately not passed. Without it git reports a nested git
/// repository instead of deleting it, which is the right side of that mistake
/// to be on: a stray clone someone forgot about is recoverable only while it
/// still exists.
pub fn clean_repo(dir: &Path, excludes: &[String], force: bool) -> Result<Vec<String>> {
    let mut args: Vec<&str> = vec!["clean", "-xd", if force { "-f" } else { "-n" }];
    for pattern in excludes {
        args.push("-e");
        args.push(pattern);
    }
    let output = run_git(&args, Some(dir), true)?;
    Ok(clean_report(&output))
}

/// The paths a `git clean` run reports, removed or (with `-n`) to be removed,
/// as git prints them: a directory keeps its trailing `/`.
pub(crate) fn clean_report(output: &std::process::Output) -> Vec<String> {
    stdout_str(output)
        .lines()
        .filter_map(|line| {
            line.strip_prefix("Removing ")
                .or_else(|| line.strip_prefix("Would remove "))
                .map(str::to_string)
        })
        .collect()
}

pub fn is_repo_root(dir: &Path) -> bool {
    query(dir, &["rev-parse", "--show-toplevel"]).is_some_and(|top| {
        match (fs::canonicalize(top), fs::canonicalize(dir)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    })
}

/// Resolve a path inside `repo`'s git directory, the way git itself would.
///
/// Built by asking git rather than by joining `.git/…`: in a linked worktree
/// `.git` is a file, so the hand-built path matches nothing and the caller
/// quietly does the wrong thing instead of erroring.
pub fn git_path(repo: &Path, name: &str) -> Option<PathBuf> {
    query(repo, &["rev-parse", "--git-path", name]).map(|path| repo.join(path))
}

/// Repack `repo` against its alternates and drop what it no longer needs to
/// own — the step that turns a full local copy into a thin borrower.
///
/// `-l` is what keeps this honest: only objects this repository actually has
/// are repacked, and anything reachable through the alternate stays there.
pub fn repack_local(repo: &Path) -> Result<()> {
    run_git(&["repack", "-a", "-d", "-l", "--quiet"], Some(repo), true)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub directory: String,
    pub exists: bool,
    pub current_ref: String,
    pub expected_ref: String,
    pub is_clean: bool,
    pub is_detached: bool,
    pub ahead: i32,
    pub behind: i32,
    pub mode: String,
    pub is_stale: bool,
    pub is_symlink: bool,
    pub symlink_target: String,
    pub has_unlinked: bool,
    pub has_unlinked_modified: bool,
    /// What the cache is doing for this checkout. Filled in by the caller,
    /// which is the one that knows where the cache is.
    pub cache: crate::cache::CacheUse,
    /// The checkout sits on exactly the commit the configured revision names.
    ///
    /// Separate from comparing `current_ref` with `expected_ref` as text,
    /// because a tag or a SHA leaves HEAD detached and git then spells the
    /// answer as an abbreviated commit — a spelling the revision can never
    /// match, however right the checkout is.
    pub at_expected: bool,
    /// What an artefact entry has installed and what the last fetch saw —
    /// `None` for a git entry.
    pub artefact: Option<crate::artefact::State>,
}

impl RepoStatus {
    /// An existing, clean checkout of `entry` with nothing to report — what
    /// each status below starts from and says only how it differs.
    fn new(entry: &RepoEntry) -> Self {
        RepoStatus {
            directory: entry.directory.clone(),
            exists: true,
            current_ref: String::new(),
            expected_ref: entry.revision.clone(),
            is_clean: true,
            is_detached: false,
            ahead: 0,
            behind: 0,
            mode: entry.mode.to_string(),
            is_stale: false,
            is_symlink: false,
            symlink_target: String::new(),
            has_unlinked: false,
            has_unlinked_modified: false,
            cache: crate::cache::CacheUse::Unused,
            at_expected: true,
            artefact: None,
        }
    }

    /// An entry whose path is a symlink: where it points, and — for a git
    /// entry, `with_ref` — the ref of the checkout it points at.
    fn symlink(entry: &RepoEntry, dest: &Path, with_ref: bool) -> Self {
        let symlink_target = fs::read_link(dest)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let current_ref = if with_ref {
            dest.canonicalize()
                .ok()
                .and_then(|resolved| head_ref(&resolved))
                .map(|(name, _)| name)
                .unwrap_or_default()
        } else {
            String::new()
        };
        RepoStatus {
            current_ref,
            is_symlink: true,
            symlink_target,
            ..RepoStatus::new(entry)
        }
    }
}

/// HEAD in `dir`, as status shows it: the branch name, or the abbreviated
/// commit when detached, with whether it is detached. `None` when HEAD does not
/// resolve at all.
fn head_ref(dir: &Path) -> Option<(String, bool)> {
    let branch = run_git(&["symbolic-ref", "--short", "HEAD"], Some(dir), false).ok()?;
    if branch.status.success() {
        return Some((stdout_str(&branch), false));
    }
    let commit = run_git(&["rev-parse", "--short", "HEAD"], Some(dir), false).ok()?;
    commit.status.success().then(|| (stdout_str(&commit), true))
}

/// Commits `dir`'s HEAD has that its upstream does not, and the reverse;
/// `(0, 0)` without an upstream.
pub(crate) fn ahead_behind(dir: &Path) -> (i32, i32) {
    let Ok(output) = run_git(
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        Some(dir),
        false,
    ) else {
        return (0, 0);
    };
    let counts = stdout_str(&output);
    match counts.split_whitespace().collect::<Vec<_>>()[..] {
        [ahead, behind] if output.status.success() => {
            (ahead.parse().unwrap_or(0), behind.parse().unwrap_or(0))
        }
        _ => (0, 0),
    }
}

/// Whether any commit reachable from HEAD or a local branch is missing from
/// every remote-tracking ref. Unlike `ahead_behind`, this needs no upstream: a
/// branch that was never pushed is all unpushed. A git failure counts as yes,
/// since the answer decides whether a directory is deleted.
fn has_unpushed_commits(path: &Path) -> bool {
    run_git(
        &[
            "rev-list",
            "-n",
            "1",
            "HEAD",
            "--branches",
            "--not",
            "--remotes",
        ],
        Some(path),
        false,
    )
    .map(|o| !o.status.success() || !stdout_str(&o).is_empty())
    .unwrap_or(true)
}

fn has_stash(path: &Path) -> bool {
    run_git(
        &["rev-parse", "--verify", "--quiet", "refs/stash"],
        Some(path),
        false,
    )
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// Whether the checkout at `path`, or any gitscale checkout nested in it,
/// holds work that replacing it would lose: uncommitted changes, commits no
/// remote has (on HEAD or any local branch, tracking or not), or a stash.
/// Ignored files are not work: they are what a build leaves behind.
pub fn is_tree_modified(path: &Path) -> bool {
    let dirty = run_git(&["status", "--porcelain"], Some(path), false)
        .map(|o| !stdout_str(&o).is_empty())
        .unwrap_or(false);
    if dirty || has_unpushed_commits(path) || has_stash(path) {
        return true;
    }
    let Some(config) =
        crate::config::load_config_optional(&path.join(crate::config::CONFIG_FILENAME))
    else {
        return false;
    };
    config
        .repos
        .iter()
        .filter(|e| !e.is_artefact())
        .any(|entry| {
            let child = path.join(&entry.directory);
            is_checkout(&child) && !child.is_symlink() && is_tree_modified(&child)
        })
}

/// Whether `dest` is at the commit `revision` names.
///
/// Both sides are resolved rather than compared as text. Anything that will
/// not resolve — a shallow clone without the tag, a revision the remote has
/// since deleted — answers `true`: status should not raise a complaint it
/// cannot substantiate, and the flags that do cover those cases are separate.
fn is_at_revision(dest: &Path, revision: &str) -> bool {
    if revision.is_empty() {
        return true;
    }
    let wanted = resolve_ref(dest, &format!("{}^{{commit}}", revision));
    match (wanted, resolve_ref(dest, "HEAD")) {
        (Some(wanted), Some(head)) => wanted == head,
        _ => true,
    }
}

pub fn get_current_ref(entry: &RepoEntry, root: &Path) -> Result<String> {
    head_ref(&root.join(&entry.directory))
        .map(|(name, _)| name)
        .ok_or_else(|| anyhow::anyhow!("HEAD does not resolve in {}", entry.directory))
}

pub fn is_detached(entry: &RepoEntry, root: &Path) -> bool {
    let dest = root.join(&entry.directory);
    run_git(&["symbolic-ref", "HEAD"], Some(&dest), false)
        .map(|o| !o.status.success())
        .unwrap_or(true)
}

pub fn is_clean(entry: &RepoEntry, root: &Path) -> bool {
    let dest = root.join(&entry.directory);
    run_git(&["status", "--porcelain"], Some(&dest), false)
        .map(|o| stdout_str(&o).is_empty())
        .unwrap_or(false)
}

fn is_stale(dest: &Path) -> bool {
    match (resolve_ref(dest, "HEAD"), resolve_ref(dest, "@{upstream}")) {
        (Some(local), Some(remote)) => local != remote,
        _ => false,
    }
}

pub fn get_repo_status(entry: &RepoEntry, root: &Path) -> RepoStatus {
    let dest = root.join(&entry.directory);
    if !is_checkout(&dest) {
        return RepoStatus {
            exists: false,
            is_symlink: dest.is_symlink(),
            ..RepoStatus::new(entry)
        };
    }
    if dest.is_symlink() {
        return RepoStatus::symlink(entry, &dest, true);
    }

    let (current_ref, is_detached) = head_ref(&dest).unwrap_or_default();
    let base = RepoStatus {
        current_ref,
        is_detached,
        is_clean: is_clean(entry, root),
        at_expected: is_at_revision(&dest, &entry.revision),
        ..RepoStatus::new(entry)
    };
    // A shallow checkout has no history to count ahead or behind against;
    // whether it sits on its upstream is all there is to say.
    if is_shallow(&dest) {
        return RepoStatus {
            is_stale: is_stale(&dest),
            ..base
        };
    }
    let (ahead, behind) = ahead_behind(&dest);
    RepoStatus {
        ahead,
        behind,
        ..base
    }
}

pub fn get_artefact_status(entry: &RepoEntry, root: &Path) -> RepoStatus {
    let dest = root.join(&entry.directory);
    if dest.is_symlink() {
        return RepoStatus::symlink(entry, &dest, false);
    }
    let state = crate::artefact::state(entry, root);
    RepoStatus {
        exists: state.installed.is_some(),
        current_ref: state
            .installed
            .as_ref()
            .map(|i| short_sha(&i.commit).to_string())
            .unwrap_or_default(),
        artefact: Some(state),
        ..RepoStatus::new(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::{git_error_line, git_failure};
    use crate::ci::CiAuth;
    use std::collections::HashMap;

    #[test]
    fn picks_fatal_line_over_leading_warning() {
        let stderr = "** WARNING: connection is not using a post-quantum key exchange algorithm.\n\
                       git@gitlab.example.com: Permission denied (publickey).\n\
                       fatal: Could not read from remote repository.\n\
                       \n\
                       Please make sure you have the correct access rights\n\
                       and the repository exists.";
        assert_eq!(
            git_error_line(stderr),
            "fatal: Could not read from remote repository."
        );
    }

    #[test]
    fn falls_back_to_last_line_when_no_fatal_marker() {
        let stderr = "Cloning into 'repo'...\nsomething went sideways";
        assert_eq!(git_error_line(stderr), "something went sideways");
    }

    #[test]
    fn falls_back_to_unknown_error_when_empty() {
        assert_eq!(git_error_line(""), "unknown error");
    }

    #[test]
    fn explains_a_403_from_the_ci_server() {
        let auth = CiAuth::from_map(&HashMap::from([
            ("CI_JOB_TOKEN", "tok"),
            ("CI_SERVER_URL", "https://gitlab.example.com"),
        ]))
        .unwrap();
        let stderr = "fatal: unable to access \
                      'https://gitlab.example.com/acme/payments.git/': \
                      The requested URL returned error: 403";
        let message = git_failure(stderr, Some(&auth));
        assert!(message.starts_with("fatal: unable to access"));
        assert!(message.contains("Job token permissions"));
        // Without CI credentials there is nothing useful to add.
        assert!(!git_failure(stderr, None).contains("hint:"));
    }

    #[test]
    fn leaves_unrelated_failures_unannotated() {
        let auth = CiAuth::from_map(&HashMap::from([
            ("CI_JOB_TOKEN", "tok"),
            ("CI_SERVER_URL", "https://gitlab.example.com"),
        ]))
        .unwrap();
        let stderr = "fatal: repository 'https://gitlab.example.com/x.git/' not found";
        assert!(!git_failure(stderr, Some(&auth)).contains("hint:"));
    }
}
