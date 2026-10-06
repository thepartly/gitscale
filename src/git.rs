use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::ci;
use crate::config::RepoEntry;

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

/// Whether a person ran this command, at a terminal, and may answer a
/// credential prompt — set once, as it starts: see [`set_watched`].
static WATCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record whether someone is watching this run. Watched, every git call
/// gitscale makes keeps the user's askpass helpers, so it authenticates as
/// their own `git` in that terminal would — an editor's credential prompt
/// included. Unwatched — a git hook, CI, a run with no terminal — no helper
/// may ask, and what needs a credential git does not have fails rather than
/// waits.
pub fn set_watched(watched: bool) {
    WATCHED.store(watched, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn run_git(
    args: &[&str],
    cwd: Option<&Path>,
    check: bool,
) -> Result<std::process::Output> {
    run(
        args,
        cwd,
        check,
        WATCHED.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// [`run_git`] for a remote write the user's own command makes — `upgrade`
/// deleting a promoted branch: the user's askpass helpers are kept, watched
/// or not, so an HTTPS remote can be written as their own `git push` would.
/// Never from a hook: nobody may be there to answer.
pub(crate) fn run_git_as_user(
    args: &[&str],
    cwd: Option<&Path>,
    check: bool,
) -> Result<std::process::Output> {
    run(args, cwd, check, true)
}

fn run(
    args: &[&str],
    cwd: Option<&Path>,
    check: bool,
    askpass: bool,
) -> Result<std::process::Output> {
    let mut cmd = prepared(args, cwd, askpass);
    let output = cmd
        .output()
        .with_context(|| format!("failed to run: git {}", args.join(" ")))?;
    if check && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{}", git_failure(&stderr, ci::active()));
    }
    Ok(output)
}

/// The git command [`run`] runs. Without `askpass`, no helper may ask for a
/// credential either: a run nobody watches must fail rather than wait.
fn prepared(args: &[&str], cwd: Option<&Path>, askpass: bool) -> Command {
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
    // it, a hook that places the workspace recurses without bound.
    cmd.env("GITSCALE_HOOK", "1");
    if !askpass {
        cmd.env("GIT_ASKPASS", "");
        cmd.env("SSH_ASKPASS", "");
        cmd.env("SSH_ASKPASS_REQUIRE", "never");
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd
}

/// [`run_git`] with `input` on stdin, never checked: the caller reads the
/// exit status, since for some commands a non-zero one is an answer.
pub(crate) fn run_git_input(
    args: &[&str],
    cwd: Option<&Path>,
    input: &[u8],
) -> Result<std::process::Output> {
    use std::io::Write as _;
    let mut cmd = git_command();
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GITSCALE_HOOK", "1");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to run: git {}", args.join(" ")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input)?;
    }
    Ok(child.wait_with_output()?)
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
/// the history a shallow clone or an artefact never downloads. `git clone
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

/// `(commit, full ref name)` for each branch and tag of a remote.
pub type RemoteRefs = Vec<(String, String)>;

/// Every branch and tag of the remote at `url`, as `(commit, full ref name)`,
/// a tag peeled to the commit it points at, and the branch its `HEAD` names —
/// its default branch. One ref advertisement, no objects.
pub fn ls_remote_full(url: &str) -> Result<(RemoteRefs, Option<String>)> {
    let listed = run_git(
        &[
            "ls-remote",
            "--symref",
            url,
            "HEAD",
            "refs/heads/*",
            "refs/tags/*",
        ],
        None,
        true,
    )?;
    let text = String::from_utf8_lossy(&listed.stdout);
    let default = text.lines().find_map(|line| {
        line.strip_prefix("ref: refs/heads/")?
            .split_once('\t')
            .filter(|(_, name)| *name == "HEAD")
            .map(|(branch, _)| branch.to_string())
    });
    let lines: Vec<(&str, &str)> = text
        .lines()
        .filter(|l| !l.starts_with("ref: "))
        .filter_map(|l| l.split_once('\t'))
        .filter(|(_, name)| name.starts_with("refs/"))
        .collect();
    let peeled: std::collections::HashMap<&str, &str> = lines
        .iter()
        .filter_map(|(sha, name)| name.strip_suffix("^{}").map(|n| (n, *sha)))
        .collect();
    let refs = lines
        .iter()
        .filter(|(_, name)| !name.ends_with("^{}"))
        .map(|(sha, name)| {
            let commit = peeled.get(name).copied().unwrap_or(sha);
            (commit.to_string(), name.to_string())
        })
        .collect();
    Ok((refs, default))
}

/// A commit as people read it: its first 7 hex digits, git's (and GitHub's)
/// default abbreviation. For display only — never resolve one of these.
pub fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
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

/// Whether a failed fetch or `ls-remote`, by what git said, was refused —
/// the repository is not there for us, or not ours to read — rather than
/// unable to reach the server at all.
pub fn is_access_error(said: &str) -> bool {
    let said = said.to_lowercase();
    let unreachable = [
        "could not resolve host",
        "connection refused",
        "connection timed out",
        "timed out",
        "network is unreachable",
        "failed to connect",
        "couldn't connect",
        "no route to host",
    ];
    if unreachable.iter().any(|s| said.contains(s)) {
        return false;
    }
    let refused = [
        "repository not found",
        "does not appear to be a git repository",
        "authentication failed",
        "permission denied",
        "access denied",
        "could not read username",
        "terminal prompts disabled",
        "the requested url returned error: 401",
        "the requested url returned error: 403",
        "the requested url returned error: 404",
    ];
    refused.iter().any(|s| said.contains(s))
}

/// Ensure an existing clone's `origin` remote URL matches the configured URL.
/// Returns `true` if the remote was updated.
pub fn reconcile_remote(entry: &RepoEntry, root: &Path) -> Result<bool> {
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
pub(crate) fn ensure_ci_remote(entry: &RepoEntry, dest: &Path) -> Result<()> {
    let Some(auth) = ci::active() else {
        return Ok(());
    };
    let Some(url) = auth.remote_url(&entry.repo_url) else {
        return Ok(());
    };
    set_origin(dest, &url)?;
    Ok(())
}

pub fn is_shallow(dest: &Path) -> bool {
    run_git(&["rev-parse", "--is-shallow-repository"], Some(dest), false)
        .map(|o| stdout_str(&o) == "true")
        .unwrap_or(false)
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
pub(crate) fn fast_forward(dest: &Path) -> Result<()> {
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

/// Refuse to move the checkout at `dest` when that could lose anything — the
/// one condition on which a pull moves somebody's checkout. A switch of
/// branch is a move even when both branches sit at the same commit.
///
/// `carry` is a move onto a topic branch: uncommitted changes go along with
/// it rather than stopping it, and git refuses the move itself should one of
/// them conflict.
pub(crate) fn ensure_switchable(dest: &Path, resets: Option<&str>, carry: bool) -> Result<()> {
    // A CI checkout holds no one's work: what the last job left behind is
    // what the pull's own scrub exists to remove, so it must not stop the
    // move the scrub follows.
    if is_ci() {
        return Ok(());
    }
    // Changes to tracked files are what a move could lose. Untracked files
    // are not: a checkout keeps them, and refuses rather than overwrite one —
    // which is as well, since the links gitscale plants for a repository's own
    // dependencies are untracked files in every repository that does not
    // ignore its import directory.
    let dirty = run_git(
        &["status", "--porcelain", "--untracked-files=no"],
        Some(dest),
        true,
    )?;
    if !carry && !stdout_str(&dirty).is_empty() {
        bail!("not moved: uncommitted changes; commit or stash, then pull again");
    }
    let lost = commits_a_move_would_lose(dest, resets)?;
    let Some(first) = lost.first() else {
        return Ok(());
    };
    match head_ref(dest) {
        Some((branch, false)) => bail!(
            "not moved: {} on {} no remote has would be lost ({}); push them, then pull \
             again",
            if lost.len() == 1 {
                "a commit"
            } else {
                "commits"
            },
            branch,
            short_sha(first)
        ),
        _ => {
            let head = resolve_ref(dest, "HEAD").unwrap_or_default();
            bail!(
                "not moved: HEAD {} is on no branch; keep it with `git branch <name> {}`, then \
                 pull again",
                short_sha(&head),
                head
            )
        }
    }
}

/// Commits HEAD, and the branch a move resets, hold that nothing else would
/// once the move is made: no remote, no tag, no other local branch. A commit
/// a shallow fetch brought — listed in `.git/shallow`, its parents never
/// fetched — is the remote's, not somebody's work, though no ref holds it.
fn commits_a_move_would_lose(dest: &Path, resets: Option<&str>) -> Result<Vec<String>> {
    let reset_ref = resets.map(|branch| format!("refs/heads/{}", branch));
    let mut args: Vec<String> = vec!["rev-list".into(), "HEAD".into()];
    if let Some(reset) = &reset_ref {
        if ref_exists(dest, reset) {
            args.push(reset.clone());
        }
    }
    args.extend(["--not", "--tags", "--remotes"].map(String::from));
    // Every other local branch keeps its commits through the move.
    let branches = query(
        dest,
        &["for-each-ref", "--format=%(refname)", "refs/heads/"],
    )
    .unwrap_or_default();
    for branch in branches.lines() {
        if Some(branch) != reset_ref.as_deref() {
            args.push(branch.to_string());
        }
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let listed = run_git(&refs, Some(dest), false)?;
    if !listed.status.success() {
        // Unknown is not safe: say HEAD itself would be lost.
        return Ok(resolve_ref(dest, "HEAD").into_iter().collect());
    }
    let fetched: std::collections::HashSet<String> = git_path(dest, "shallow")
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default();
    Ok(stdout_str(&listed)
        .lines()
        .filter(|commit| !fetched.contains(*commit))
        .map(str::to_string)
        .collect())
}

/// `git checkout` with `args`, after [`ensure_switchable`] has passed: no
/// tracked file has changes to lose, so a plain checkout moves it — and
/// refuses, rather than overwrite, an untracked file in the way. In CI, where
/// a checkout holds nobody's work and the scrub after the pull removes what
/// the last job left, it is forced, as it always was.
pub(crate) fn move_checkout(dest: &Path, args: &[&str]) -> Result<()> {
    let mut full = vec!["checkout", "--quiet"];
    if is_ci() {
        full.push("-f");
    }
    full.extend_from_slice(args);
    run_git(&full, Some(dest), true)?;
    Ok(())
}

/// What `name` resolves to in `dir` (a ref, `HEAD`, `<rev>^{commit}`…), or
/// `None` when it does not.
pub(crate) fn resolve_ref(dir: &Path, name: &str) -> Option<String> {
    query(dir, &["rev-parse", "--verify", "--quiet", name])
}

pub(crate) fn ref_exists(dir: &Path, name: &str) -> bool {
    resolve_ref(dir, name).is_some()
}

/// What in the checkout at `dir` a move could lose, as a reason to give —
/// `None` when nothing: uncommitted changes (see [`uncommitted`]), or commits
/// on HEAD no remote or tag has.
pub fn local_work(dir: &Path, planted: &[String]) -> Option<String> {
    if let Some(changed) = uncommitted(dir, planted) {
        return Some(format!(
            "{} uncommitted",
            crate::cache::plural(changed, "change", "changes")
        ));
    }
    let unpushed = unpushed(dir);
    (unpushed > 0).then(|| {
        format!(
            "{} not pushed",
            crate::cache::plural(unpushed, "commit", "commits")
        )
    })
}

/// How many paths of the checkout at `dir` have uncommitted changes —
/// tracked files changed, or untracked files other than the `planted` links
/// (relative to `dir`) — or `None` when none do.
pub fn uncommitted(dir: &Path, planted: &[String]) -> Option<usize> {
    let listed = porcelain(dir)?;
    let changed = listed
        .iter()
        .filter(|(untracked, path)| !(*untracked && planted.iter().any(|p| p == path)))
        .count();
    (changed > 0).then_some(changed)
}

/// How many commits on HEAD of `dir` no remote or tag has. One a shallow
/// fetch brought is the remote's, not somebody's work.
pub fn unpushed(dir: &Path) -> usize {
    let Some(listed) = query(dir, &["rev-list", "HEAD", "--not", "--remotes", "--tags"]) else {
        return 0;
    };
    let fetched: std::collections::HashSet<String> = git_path(dir, "shallow")
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default();
    listed
        .lines()
        .filter(|c| !c.is_empty() && !fetched.contains(*c))
        .count()
}

/// Remove untracked files from `dir`'s working tree as `flags` say,
/// returning the paths removed (or, when not deleting, the paths that would
/// be).
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
pub fn clean_repo(
    dir: &Path,
    excludes: &[String],
    flags: &crate::commands::clean::Flags,
) -> Result<Vec<String>> {
    use crate::commands::clean::Ignored;
    let listing = |ignored: &str, keep: bool| -> Result<Vec<String>> {
        let mut args: Vec<&str> = vec!["clean", "-n"];
        if flags.directories {
            args.push("-d");
        }
        args.push(ignored);
        if keep {
            for pattern in excludes {
                args.push("-e");
                args.push(pattern);
            }
        }
        Ok(clean_report(&run_git(&args, Some(dir), true)?))
    };
    if flags.ignored != Ignored::Only {
        let mut args: Vec<&str> = vec!["clean", if flags.delete { "-f" } else { "-n" }];
        if flags.directories {
            args.push("-d");
        }
        if flags.ignored == Ignored::Too {
            args.push("-x");
        }
        for pattern in excludes {
            args.push("-e");
            args.push(pattern);
        }
        let output = run_git(&args, Some(dir), true)?;
        return Ok(clean_report(&output));
    }
    // `-X` takes `-e` patterns for more ignored files, to remove rather than
    // keep — every checkout among them. So what goes is what `-X` alone
    // would remove that `-x` keeping them would remove too.
    let keeping = listing("-x", true)?;
    let paths: Vec<String> = listing("-X", false)?
        .into_iter()
        .filter(|p| keeping.contains(p))
        .collect();
    if flags.delete && !paths.is_empty() {
        for chunk in paths.chunks(200) {
            let specs: Vec<String> = chunk.iter().map(|p| format!(":(literal){}", p)).collect();
            let mut args: Vec<&str> = vec!["clean", "-f", "-X"];
            if flags.directories {
                args.push("-d");
            }
            args.push("--");
            args.extend(specs.iter().map(String::as_str));
            run_git(&args, Some(dir), true)?;
        }
    }
    Ok(paths)
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

/// Returns true when `dir` is the top level of its own git repository (not
/// merely nested inside some ancestor git repo).
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

/// The git directory every worktree of `repo`'s repository shares: `.git` of
/// a plain clone, the bare repository of a set of worktrees.
pub fn common_dir(repo: &Path) -> Option<PathBuf> {
    query(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(PathBuf::from)
}

/// The branch `dir` is on, `None` when detached or not a repository.
pub fn current_branch(dir: &Path) -> Option<String> {
    query(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]).filter(|b| !b.is_empty())
}

/// Whether this git writes worktree links relative to each other
/// (`worktree.useRelativePaths`, git 2.48 and later).
pub fn supports_relative_worktrees() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let Ok(output) = run_git(&["--version"], None, false) else {
            return false;
        };
        version_at_least(&stdout_str(&output), (2, 48))
    })
}

/// Whether `git version 2.43.0`-style text names at least `wanted`.
fn version_at_least(text: &str, wanted: (u64, u64)) -> bool {
    let numbers: Vec<u64> = text
        .split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or_default()
        .split('.')
        .map_while(|part| part.parse().ok())
        .collect();
    match numbers.as_slice() {
        [major, minor, ..] => (*major, *minor) >= wanted,
        _ => false,
    }
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
    /// The form the checkout has on disk, `None` when it is not there.
    /// Filled in by the caller.
    pub on_disk: Option<crate::prefer::Form>,
    pub is_stale: bool,
    pub is_symlink: bool,
    pub symlink_target: String,
    pub has_unlinked: bool,
    pub has_unlinked_modified: bool,
    /// The links gitscale planted in this checkout that git sees as untracked
    /// files, because the repository does not ignore where they live. Filled
    /// in by the caller, which knows the links.
    pub untracked_links: Vec<String>,
    /// The checkout is where resolution puts it: detached at the commit it
    /// selected, or on the topic branch.
    ///
    /// Separate from comparing `current_ref` with `expected_ref` as text,
    /// because a detached HEAD is spelled as an abbreviated commit — a
    /// spelling the revision can never match, however right the checkout is.
    pub at_expected: bool,
    /// Not a worktree of the root's store for its repository: a clone of its
    /// own, or one whose store is gone. Filled in by the caller, which knows
    /// the stores.
    pub foreign: bool,
    /// What an artefact has installed and what the last fetch saw —
    /// `None` for a source checkout.
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
            on_disk: None,
            is_stale: false,
            is_symlink: false,
            symlink_target: String::new(),
            has_unlinked: false,
            has_unlinked_modified: false,
            untracked_links: Vec::new(),
            at_expected: true,
            foreign: false,
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
pub(crate) fn head_ref(dir: &Path) -> Option<(String, bool)> {
    let branch = run_git(&["symbolic-ref", "--short", "HEAD"], Some(dir), false).ok()?;
    if branch.status.success() {
        return Some((stdout_str(&branch), false));
    }
    let commit = run_git(&["rev-parse", "--short", "HEAD"], Some(dir), false).ok()?;
    commit.status.success().then(|| (stdout_str(&commit), true))
}

/// Commits `dir`'s HEAD has that its upstream does not, and the reverse. A
/// branch with no upstream — a topic branch not pushed yet — is ahead by the
/// commits no remote branch or tag has: work only this machine holds.
/// `(0, 0)` detached.
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
        _ if head_ref(dir).is_some_and(|(_, detached)| !detached) => {
            let unpushed = query(
                dir,
                &[
                    "rev-list",
                    "--count",
                    "HEAD",
                    "--not",
                    "--remotes",
                    "--tags",
                ],
            );
            (unpushed.and_then(|n| n.parse().ok()).unwrap_or(0), 0)
        }
        _ => (0, 0),
    }
}

/// What `git status` lists in `dir`, every untracked file named on its own
/// line rather than folded into its directory: `(untracked, path)` per entry.
/// `None` when git cannot say.
pub fn porcelain(dir: &Path) -> Option<Vec<(bool, String)>> {
    let output = run_git(
        &["status", "--porcelain", "--untracked-files=all"],
        Some(dir),
        false,
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.len() > 3)
            .map(|line| {
                (
                    line.starts_with("??"),
                    line[3..].trim_matches('"').to_string(),
                )
            })
            .collect(),
    )
}

/// Whether any commit reachable from HEAD or a local branch is missing from
/// every remote-tracking ref. Unlike `ahead_behind`, this needs no upstream: a
/// branch that was never pushed is all unpushed. A git failure counts as yes,
/// since the answer decides whether a directory is deleted.
pub(crate) fn has_unpushed_commits(path: &Path) -> bool {
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
/// Ignored files are not work: they are what a build leaves behind. Nor is an
/// untracked symlink, which holds a path and no content — the links gitscale
/// plants for a repository's own dependencies are exactly that.
pub fn is_tree_modified(path: &Path) -> bool {
    let dirty = porcelain(path)
        .map(|listed| {
            listed
                .iter()
                .any(|(untracked, file)| !(*untracked && path.join(file).is_symlink()))
        })
        .unwrap_or(false);
    if dirty || has_unpushed_commits(path) || has_stash(path) {
        return true;
    }
    let Some(config) =
        crate::config::load_config_optional(&path.join(crate::config::CONFIG_FILENAME))
    else {
        return false;
    };
    config.repos.iter().any(|entry| {
        let child = path.join(&entry.directory);
        is_checkout(&child) && !child.is_symlink() && is_tree_modified(&child)
    })
}

/// Where resolution puts a checkout: on a topic branch, or detached at a
/// commit. Neither, when it could not tell — a slot not resolved offline.
#[derive(Debug, Clone, Default)]
pub struct Expected {
    pub branch: Option<String>,
    pub commit: Option<String>,
}

impl Expected {
    /// Whether the checkout at `dest` is there. Unknown answers `true`:
    /// status should not raise a complaint it cannot substantiate.
    fn met_by(&self, dest: &Path) -> bool {
        if let Some(branch) = &self.branch {
            return current_branch(dest).as_deref() == Some(branch.as_str());
        }
        match (&self.commit, resolve_ref(dest, "HEAD")) {
            (Some(wanted), Some(head)) => wanted.eq_ignore_ascii_case(&head),
            _ => true,
        }
    }
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

pub fn get_repo_status(entry: &RepoEntry, root: &Path, expected: &Expected) -> RepoStatus {
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
        at_expected: expected.met_by(&dest),
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
            .map(|i| i.tag.clone())
            .unwrap_or_default(),
        artefact: Some(state),
        ..RepoStatus::new(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::{git_error_line, git_failure, prepared, version_at_least};
    use crate::ci::CiAuth;
    use std::collections::HashMap;

    /// A run nobody may be watching blanks every askpass helper; a remote
    /// write the user's own command makes keeps theirs.
    #[test]
    fn only_the_users_own_writes_keep_their_askpass_helpers() {
        let envs = |askpass| -> Vec<(String, Option<String>)> {
            prepared(&["push"], None, askpass)
                .get_envs()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.map(|v| v.to_string_lossy().into_owned()),
                    )
                })
                .collect()
        };
        let blank = |var: &str| (var.to_string(), Some(String::new()));
        let quiet = envs(false);
        assert!(quiet.contains(&blank("GIT_ASKPASS")));
        assert!(quiet.contains(&blank("SSH_ASKPASS")));
        let user = envs(true);
        assert!(!user
            .iter()
            .any(|(k, _)| k == "GIT_ASKPASS" || k == "SSH_ASKPASS"));
        // Never a terminal prompt either way: stdin is not the user's.
        for set in [&quiet, &user] {
            assert!(set.contains(&("GIT_TERMINAL_PROMPT".to_string(), Some("0".to_string()))));
        }
    }

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
    fn git_versions_compare_by_major_and_minor() {
        assert!(version_at_least("git version 2.48.0", (2, 48)));
        assert!(version_at_least(
            "git version 2.50.1 (Apple Git-155)",
            (2, 48)
        ));
        assert!(!version_at_least("git version 2.43.0", (2, 48)));
        assert!(version_at_least("git version 3.0", (2, 48)));
        assert!(!version_at_least("nonsense", (2, 48)));
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
