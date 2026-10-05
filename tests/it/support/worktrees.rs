//! Helpers for the worktrees tests.

use super::{git_stdout, run_git_pub, strip_ansi, TestEnv};
use std::path::{Path, PathBuf};
/// gitscale run against the workspace at `dir`.
pub fn gs(dir: &Path, args: &[&str]) -> super::CliOutput {
    let mut full = vec!["gitscale", "-C", dir.to_str().unwrap()];
    full.extend_from_slice(args);
    gitscale::run_cli_with(&full, false)
}

pub fn ok(out: &super::CliOutput) {
    assert!(out.success, "{}{}", out.stdout, out.stderr);
}

/// `git` in `dir`, which must succeed.
pub fn git(dir: &Path, args: &[&str]) -> String {
    git_stdout(dir, args)
}

pub fn git_ok(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A dependency `core` tagged v1.0.0, and a root repository whose config pins
/// it there, with `extra` added to that config.
pub struct Fixture {
    pub env: TestEnv,
    pub core: PathBuf,
    pub root: PathBuf,
}

pub fn fixture(name: &str, extra: &str) -> Fixture {
    let env = TestEnv::new(name);
    let core = env.create_bare_repo("core", "main", &[("lib.txt", "v1")]);
    run_git_pub(&core, &["tag", "v1.0.0", "main"]);
    let config = format!(
        "{}[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        extra,
        core.display()
    );
    let root = env.create_bare_repo(
        "root",
        "main",
        &[("README.md", "root"), (".gitscale.toml", &config)],
    );
    Fixture { env, core, root }
}

impl Fixture {
    /// A plain clone of the root at `name`, synced.
    pub fn clone_root(&self, name: &str) -> PathBuf {
        let dest = self.env.repos_remote.join(name);
        run_git_pub(
            &self.env.repos_remote,
            &[
                "clone",
                "-q",
                self.root.to_str().unwrap(),
                dest.to_str().unwrap(),
            ],
        );
        identity(&dest);
        ok(&gs(&dest, &["sync"]));
        dest
    }

    /// Commit `content` to `file` on `branch` of `core`, creating the branch
    /// from main if it is not there. Returns the commit.
    pub fn core_commit(&self, branch: &str, file: &str, content: &str) -> String {
        if !git_ok(&self.core, &["rev-parse", "--verify", "-q", branch]) {
            run_git_pub(&self.core, &["branch", branch, "main"]);
        }
        self.env.push_commit(&self.core, branch, file, content)
    }
}

pub fn identity(repo: &Path) {
    run_git_pub(repo, &["config", "user.email", "t@t.com"]);
    run_git_pub(repo, &["config", "user.name", "T"]);
}

pub fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

pub fn branch(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["symbolic-ref", "-q", "--short", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn writable(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(file).unwrap().permissions().mode() & 0o200 != 0
}

pub fn common_dir(dir: &Path) -> PathBuf {
    PathBuf::from(git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .canonicalize()
    .unwrap()
}

pub fn status_row(dir: &Path, repo: &str) -> String {
    let out = gs(dir, &["ls"]);
    ok(&out);
    strip_ansi(&out.stdout)
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(repo))
        .unwrap_or_default()
        .to_string()
}

/// gitscale as a GitLab job on the root's branch `branch` would run it.
pub fn ci_job(dir: &Path, cache: &Path, branch: &str, args: &[&str]) -> super::CliOutput {
    let commit = git(dir, &["rev-parse", "HEAD"]);
    let mut full: Vec<&str> = vec!["-C", dir.to_str().unwrap()];
    full.extend_from_slice(args);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(&full)
        .env("CI", "true")
        .env("GITLAB_CI", "true")
        .env("CI_COMMIT_SHA", &commit)
        .env("CI_COMMIT_BRANCH", branch)
        .env("CI_DEFAULT_BRANCH", "main")
        .env("GITSCALE_CACHE_DIR", cache)
        .output()
        .unwrap();
    super::CliOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        success: output.status.success(),
    }
}

/// A root whose `meta/app` uses `app`'s artefact as `artefact_use`.
pub fn artefact_workspace(env: &TestEnv, app: &Path, artefact_use: &str) -> PathBuf {
    let config = format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"main\", artefact = \"{}\" }}\n",
        env.registries(),
        app.display(),
        artefact_use
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    identity(&ws);
    ws
}

/// Commit `content` to `file` in the checkout at `dir`, read-only or not.
/// Returns the new commit.
pub fn commit_in(dir: &Path, file: &str, content: &str) -> String {
    identity(dir);
    super::edit(&dir.join(file), content);
    run_git_pub(dir, &["add", file]);
    run_git_pub(dir, &["commit", "-q", "-m", content]);
    head(dir)
}

/// Whether `name` resolves to a commit in the repository at `dir`.
pub fn has_ref(dir: &Path, name: &str) -> bool {
    git_ok(
        dir,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{}^{{commit}}", name),
        ],
    )
}

/// Whether some local branch of `dir`'s repository holds `commit`.
pub fn on_some_branch(dir: &Path, commit: &str) -> bool {
    !git(
        dir,
        &[
            "for-each-ref",
            "--contains",
            commit,
            "--format=%(refname)",
            "refs/heads/",
        ],
    )
    .is_empty()
}

/// The gitscale binary run against the workspace at `dir`, with extra
/// environment: for what reads the process environment, such as `HOME` or
/// `GIT_TRACE2_EVENT`.
pub fn gs_bin(dir: &Path, args: &[&str], vars: &[(&str, &str)]) -> super::CliOutput {
    let mut full: Vec<&str> = vec!["-C", dir.to_str().unwrap()];
    full.extend_from_slice(args);
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"));
    cmd.args(&full);
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let output = cmd.output().unwrap();
    super::CliOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        success: output.status.success(),
    }
}

/// The root's store for `url`, in the common git dir of the root at `ws`.
pub fn store_for(ws: &Path, url: &Path) -> PathBuf {
    common_dir(ws)
        .join("gitscale/repos")
        .join(gitscale::store::entry_name(&url.display().to_string()))
}

/// Whether the bare repository `bare` has the ref `name`. Names the git dir
/// explicitly: discovery-based access to a bare repository is refused under
/// `safe.bareRepository = explicit`.
pub fn bare_has(bare: &Path, name: &str) -> bool {
    git_ok(
        bare,
        &[
            "--git-dir",
            bare.to_str().unwrap(),
            "rev-parse",
            "--verify",
            "-q",
            name,
        ],
    )
}

/// `git` against the bare repository `bare`, which must succeed.
pub fn bare_git(bare: &Path, args: &[&str]) -> String {
    let mut full = vec!["--git-dir", bare.to_str().unwrap()];
    full.extend_from_slice(args);
    git(bare, &full)
}

/// Whether `store` lists `checkout` among its worktrees.
pub fn is_worktree_listed(store: &Path, checkout: &Path) -> bool {
    let wanted = checkout.canonicalize().unwrap();
    git(store, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .any(|p| Path::new(p).canonicalize().ok().as_deref() == Some(wanted.as_path()))
}
