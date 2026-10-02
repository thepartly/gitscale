//! Worktree workspaces end to end: every dependency a worktree of the root's
//! own store for it, the root's branch the topic, and `develop` putting a
//! checkout on it.
#[allow(dead_code)]
mod helpers;

use helpers::{git_stdout, run_git_pub, strip_ansi, TestEnv};
use std::path::{Path, PathBuf};

/// gitscale run against the workspace at `dir`.
fn gs(dir: &Path, args: &[&str]) -> helpers::CliOutput {
    let mut full = vec!["gitscale"];
    full.extend_from_slice(&args[..1]);
    full.extend_from_slice(&["-C", dir.to_str().unwrap()]);
    full.extend_from_slice(&args[1..]);
    gitscale::run_cli_with(&full, false)
}

fn ok(out: &helpers::CliOutput) {
    assert!(out.success, "{}{}", out.stdout, out.stderr);
}

/// `git` in `dir`, which must succeed.
fn git(dir: &Path, args: &[&str]) -> String {
    git_stdout(dir, args)
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A dependency `core` tagged v1.0.0, and a root repository whose config pins
/// it there, with `extra` added to that config.
struct Fixture {
    env: TestEnv,
    core: PathBuf,
    root: PathBuf,
}

fn fixture(name: &str, extra: &str) -> Fixture {
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
    /// A plain clone of the root at `name`, pulled.
    fn clone_root(&self, name: &str) -> PathBuf {
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
        ok(&gs(&dest, &["pull"]));
        dest
    }

    /// Commit `content` to `file` on `branch` of `core`, creating the branch
    /// from main if it is not there. Returns the commit.
    fn core_commit(&self, branch: &str, file: &str, content: &str) -> String {
        if !git_ok(&self.core, &["rev-parse", "--verify", "-q", branch]) {
            run_git_pub(&self.core, &["branch", branch, "main"]);
        }
        self.env.push_commit(&self.core, branch, file, content)
    }
}

fn identity(repo: &Path) {
    run_git_pub(repo, &["config", "user.email", "t@t.com"]);
    run_git_pub(repo, &["config", "user.name", "T"]);
}

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

fn branch(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["symbolic-ref", "-q", "--short", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn writable(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(file).unwrap().permissions().mode() & 0o200 != 0
}

fn common_dir(dir: &Path) -> PathBuf {
    PathBuf::from(git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .canonicalize()
    .unwrap()
}

fn status_row(dir: &Path, repo: &str) -> String {
    let out = gs(dir, &["status"]);
    ok(&out);
    strip_ansi(&out.stdout)
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(repo))
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------------------------
// Layouts
// ---------------------------------------------------------------------------

/// A plain clone: the stores live in its own `.git/gitscale`, and the child
/// is a worktree of one, detached at the pin.
#[test]
fn a_plain_clone_keeps_its_stores_in_its_own_git_directory() {
    let f = fixture("wt_plain", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert_eq!(branch(&child), None);
    assert!(!writable(&child.join("lib.txt")));
    assert!(common_dir(&child).starts_with(ws.join(".git/gitscale/repos").canonicalize().unwrap()));
}

/// A bare root with worktrees, Angel's layout: every root worktree's children
/// are worktrees of the one store in the bare repository, so a second root
/// worktree downloads nothing and sees the first one's branches.
#[test]
fn a_bare_root_shares_one_store_across_its_worktrees() {
    let f = fixture("wt_bare", "");
    let repo = f.env.repos_remote.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    run_git_pub(
        &repo,
        &["clone", "-q", "--bare", f.root.to_str().unwrap(), ".bare"],
    );
    std::fs::write(repo.join(".git"), "gitdir: ./.bare\n").unwrap();
    run_git_pub(
        &repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    run_git_pub(&repo, &["fetch", "-q"]);
    run_git_pub(&repo, &["worktree", "add", "-q", "main", "main"]);
    let main = repo.join("main");
    identity(&main);
    ok(&gs(&main, &["pull"]));

    let store_root = repo.join(".bare/gitscale/repos");
    assert!(
        store_root.is_dir(),
        "the stores live in the bare repository"
    );
    let child = main.join("imports/core");
    assert!(common_dir(&child).starts_with(store_root.canonicalize().unwrap()));

    // A second root worktree: its child is a worktree of the same store.
    run_git_pub(&repo, &["worktree", "add", "-q", "-b", "feat/x", "feature"]);
    let feature = repo.join("feature");
    ok(&gs(&feature, &["pull"]));
    let other = feature.join("imports/core");
    assert_eq!(common_dir(&other), common_dir(&child));
    assert_eq!(std::fs::read_dir(&store_root).unwrap().count(), 1);

    // A branch developed in one worktree is visible from the other's child.
    ok(&gs(&feature, &["develop", "imports/core"]));
    identity(&other);
    std::fs::write(other.join("lib.txt"), "work").unwrap();
    run_git_pub(&other, &["commit", "-q", "-am", "work"]);
    assert_eq!(git(&child, &["rev-parse", "feat/x"]), head(&other));
}

/// `git clone --bare` sets no fetch refspec; status says what that costs.
#[test]
fn status_warns_about_a_root_without_a_fetch_refspec() {
    let f = fixture("wt_refspec", "");
    let repo = f.env.repos_remote.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    run_git_pub(
        &repo,
        &["clone", "-q", "--bare", f.root.to_str().unwrap(), ".bare"],
    );
    std::fs::write(repo.join(".git"), "gitdir: ./.bare\n").unwrap();
    run_git_pub(&repo, &["worktree", "add", "-q", "main", "main"]);
    let main = repo.join("main");
    ok(&gs(&main, &["pull"]));
    let out = gs(&main, &["status"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("warning: the root repository has no fetch refspec"),
        "{}",
        out.stdout
    );
    run_git_pub(
        &repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    let out = gs(&main, &["status"]);
    assert!(!out.stdout.contains("warning:"), "{}", out.stdout);
}

// ---------------------------------------------------------------------------
// Developing
// ---------------------------------------------------------------------------

#[test]
fn develop_puts_a_child_on_the_topic_from_its_pin() {
    let f = fixture("wt_develop", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let pin = head(&child);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    let out = gs(&ws, &["develop", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core on feat/x, from v1.0.0"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), pin);
    assert!(writable(&child.join("lib.txt")));

    let row = status_row(&ws, "imports/core");
    assert!(row.contains("feat/x"), "{}", row);
    assert!(row.contains(" ok "), "{}", row);
    assert!(row.ends_with("topic, no change yet"), "{}", row);
    let table = strip_ansi(&gs(&ws, &["status"]).stdout);
    assert!(table.starts_with("topic feat/x"), "{}", table);

    // A pull keeps it there.
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

#[test]
fn develop_refuses_off_a_topic() {
    let f = fixture("wt_develop_refusals", "");
    let ws = f.clone_root("ws");
    // On main, which the root pins.
    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("git switch -c <topic>"),
        "{}",
        out.stderr
    );
    // Detached.
    run_git_pub(&ws, &["checkout", "-q", "--detach"]);
    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stderr.contains("on no branch"), "{}", out.stderr);
}

#[test]
fn develop_refuses_an_entry_the_root_overrides() {
    let f = fixture("wt_develop_override", "");
    let ws = f.clone_root("ws");
    let config = std::fs::read_to_string(ws.join(".gitscale.toml"))
        .unwrap()
        .replace(
            "revision = \"v1.0.0\"",
            "revision = \"v1.0.0\", override = true",
        );
    std::fs::write(ws.join(".gitscale.toml"), config).unwrap();
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stdout.contains("held by override"), "{}", out.stdout);
}

/// The remote has the topic branch: a pull puts the child on it, writable,
/// tracking the remote's.
#[test]
fn a_child_follows_its_remote_branch_of_the_topic() {
    let f = fixture("wt_follow", "");
    let ws = f.clone_root("ws");
    let tip = f.core_commit("feat/x", "lib.txt", "remote work");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    let child = ws.join("imports/core");
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), tip);
    assert_eq!(
        git(&child, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/feat/x"
    );
    assert!(writable(&child.join("lib.txt")));

    // Back on main: detached at the pin, read-only again.
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert!(!writable(&child.join("lib.txt")));
}

/// A child with uncommitted changes does not move when the root changes
/// branch: its entry fails, the rest goes on.
#[test]
fn a_dirty_child_stays_when_the_root_leaves_the_topic() {
    let f = fixture("wt_dirty_leave", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    std::fs::write(child.join("lib.txt"), "unfinished").unwrap();

    run_git_pub(&ws, &["switch", "-q", "main"]);
    let out = gs(&ws, &["pull"]);
    assert!(!out.success);
    assert!(out.stderr.contains("uncommitted changes"), "{}", out.stderr);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

/// `git switch -c feat/y` from topic feat/x carries the developed children to
/// feat/y, at the commits they are at, edits included.
#[test]
fn a_new_branch_from_a_topic_carries_its_children() {
    let f = fixture("wt_carry", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "committed").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "x work"]);
    std::fs::write(child.join("lib.txt"), "uncommitted").unwrap();
    let at = head(&child);

    run_git_pub(&ws, &["switch", "-q", "-c", "feat/y"]);
    let out = gs(&ws, &["pull"]);
    ok(&out);
    assert!(
        out.stdout.contains("carry  imports/core → feat/y"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&child).as_deref(), Some("feat/y"));
    assert_eq!(head(&child), at);
    assert_eq!(
        std::fs::read_to_string(child.join("lib.txt")).unwrap(),
        "uncommitted"
    );

    // Switching back to an existing topic is no new branch: nothing carried.
    run_git_pub(&child, &["checkout", "--", "lib.txt"]);
    run_git_pub(&ws, &["switch", "-q", "feat/x"]);
    let out = gs(&ws, &["pull"]);
    ok(&out);
    assert!(!out.stdout.contains("carry"), "{}", out.stdout);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

#[test]
fn develop_stop_takes_a_child_back_to_its_pin() {
    let f = fixture("wt_stop", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "work").unwrap();

    // Uncommitted work: refused.
    let out = gs(&ws, &["develop", "--stop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stdout.contains("uncommitted"), "{}", out.stdout);

    run_git_pub(&child, &["checkout", "--", "lib.txt"]);
    let out = gs(&ws, &["develop", "--stop", "imports/core"]);
    ok(&out);
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert!(!git_ok(
        &child,
        &["rev-parse", "--verify", "-q", "refs/heads/feat/x"]
    ));
    // And it stays off the topic.
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child), None);
}

/// A topic branch the remote has keeps being followed: stopping refuses until
/// it is deleted there.
#[test]
fn develop_stop_refuses_while_the_remote_has_the_branch() {
    let f = fixture("wt_stop_pushed", "");
    let ws = f.clone_root("ws");
    f.core_commit("feat/x", "lib.txt", "pushed");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    let out = gs(&ws, &["develop", "--stop", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stdout.contains("Delete it there first"),
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Pinned branches
// ---------------------------------------------------------------------------

/// A branch the root pins builds every child from pins, even one whose remote
/// has a branch of that name.
#[test]
fn a_pinned_branch_follows_no_topic() {
    let f = fixture(
        "wt_pinned",
        "[develop]\npinned = [\"main\", \"staging\"]\n\n",
    );
    let ws = f.clone_root("ws");
    f.core_commit("staging", "lib.txt", "staging");
    run_git_pub(&ws, &["switch", "-q", "-c", "staging"]);
    ok(&gs(&ws, &["pull"]));
    let child = ws.join("imports/core");
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stderr.contains("which it pins"), "{}", out.stderr);
}

/// `pinned = []` pins nothing: the root's main follows every child's main.
#[test]
fn an_empty_pinned_list_makes_the_default_branch_a_topic() {
    let f = fixture("wt_unpinned", "[develop]\npinned = []\n\n");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    assert_eq!(branch(&child).as_deref(), Some("main"));
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "main"]));
}

/// A dependency that pins the topic branch keeps its own dependencies at
/// their pins: the root builds what that dependency's pinned branch built.
#[test]
fn a_dependency_that_pins_the_topic_keeps_what_it_asks_for_at_its_pins() {
    let env = TestEnv::new("wt_pinned_below");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "d v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    run_git_pub(&d, &["branch", "staging", "main"]);
    let staging_d = env.push_commit(&d, "staging", "d.txt", "d staging");
    let b_config = format!(
        "[develop]\npinned = [\"staging\"]\n\n[repos]\n\"libs/d\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let b = env.create_bare_repo("b", "main", &[(".gitscale.toml", &b_config)]);
    run_git_pub(&b, &["tag", "v1.0.0", "main"]);
    run_git_pub(&b, &["branch", "staging", "main"]);
    let root_config = format!(
        "[resolve]\nallow = [\"{}/*\"]\n\n[repos]\n\"imports/b\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        env.repos_remote.display(),
        b.display()
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &root_config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    run_git_pub(&ws, &["switch", "-q", "-c", "staging"]);
    ok(&gs(&ws, &["pull"]));

    // B follows staging; D, below B, stays at B's pin.
    assert_eq!(branch(&ws.join("imports/b")).as_deref(), Some("staging"));
    let child_d = ws.join("imports/d");
    assert_ne!(head(&child_d), staging_d);
    assert_eq!(head(&child_d), git(&d, &["rev-parse", "v1.0.0"]));
    assert!(
        status_row(&ws, "imports/d").contains("pinned by imports/b"),
        "{}",
        status_row(&ws, "imports/d")
    );
    let out = gs(&ws, &["develop", "imports/d"]);
    assert!(!out.success);
    assert!(
        out.stdout
            .contains("imports/b pins staging for its dependencies"),
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Two majors
// ---------------------------------------------------------------------------

/// Two checkouts of one repository share a store, so their topic branches
/// need names of their own: the newest major takes the topic's, the older
/// `<topic>@v<major>`.
#[test]
fn an_older_major_develops_on_a_suffixed_branch() {
    let env = TestEnv::new("wt_majors");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    env.push_commit(&d, "main", "d.txt", "v2");
    let v2 = git(&d, &["rev-parse", "main"]);
    run_git_pub(&d, &["tag", "v2.0.0", &v2]);
    let config = format!(
        "[repos]\n\"imports/d\" = {{ url = \"{0}\", revision = \"v2.0.0\" }}\n\
         \"imports/d_v1\" = {{ url = \"{0}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    ok(&gs(&ws, &["pull"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    ok(&gs(&ws, &["develop", "imports/d", "imports/d_v1"]));
    assert_eq!(branch(&ws.join("imports/d")).as_deref(), Some("feat/x"));
    assert_eq!(
        branch(&ws.join("imports/d_v1")).as_deref(),
        Some("feat/x@v1")
    );
}

// ---------------------------------------------------------------------------
// Housekeeping
// ---------------------------------------------------------------------------

/// A root worktree deleted with its children in it leaves entries in the
/// stores that keep its branches locked; the next command prunes them.
#[test]
fn a_deleted_root_worktree_does_not_lock_its_topic() {
    let f = fixture("wt_prune", "");
    let ws = f.clone_root("ws");
    identity(&ws);
    run_git_pub(&ws, &["add", "."]);
    let other = f.env.repos_remote.join("other");
    run_git_pub(
        &ws,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/x",
            other.to_str().unwrap(),
        ],
    );
    ok(&gs(&other, &["pull"]));
    ok(&gs(&other, &["develop", "imports/core"]));
    std::fs::remove_dir_all(&other).unwrap();
    run_git_pub(&ws, &["worktree", "prune"]);

    // The same topic, in the main worktree: its branch must not be held by
    // the deleted one.
    run_git_pub(&ws, &["switch", "-q", "feat/x"]);
    let out = gs(&ws, &["pull"]);
    ok(&out);
    assert_eq!(branch(&ws.join("imports/core")).as_deref(), Some("feat/x"));
}

/// Moving a plain-clone root breaks its children's links to the store, which
/// is inside it; pull repairs them.
#[test]
fn pull_repairs_children_after_the_root_moves() {
    let f = fixture("wt_moved", "");
    let ws = f.clone_root("ws");
    let moved = f.env.repos_remote.join("moved");
    std::fs::rename(&ws, &moved).unwrap();
    let out = gs(&moved, &["pull"]);
    ok(&out);
    let child = moved.join("imports/core");
    assert!(
        git_ok(&child, &["status"]),
        "the child reads its history again"
    );
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
}

/// A checkout gitscale did not make — a clone of its own, from an older
/// gitscale or by hand — is reported and left alone.
#[test]
fn a_foreign_checkout_is_reported_and_left_alone() {
    let f = fixture("wt_foreign", "");
    let ws = f.env.repos_remote.join("ws");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            ws.to_str().unwrap(),
        ],
    );
    run_git_pub(
        &ws,
        &["clone", "-q", f.core.to_str().unwrap(), "imports/core"],
    );
    let out = gs(&ws, &["pull"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("not a gitscale worktree; move your changes out, delete it and run pull"),
        "{}",
        out.stderr
    );
    assert!(ws.join("imports/core/.git").is_dir());
    assert!(
        status_row(&ws, "imports/core").contains("foreign"),
        "{}",
        status_row(&ws, "imports/core")
    );
}

// ---------------------------------------------------------------------------
// CI
// ---------------------------------------------------------------------------

/// gitscale as a GitLab job on the root's branch `branch` would run it.
fn ci_job(dir: &Path, cache: &Path, branch: &str, args: &[&str]) -> helpers::CliOutput {
    let commit = git(dir, &["rev-parse", "HEAD"]);
    let mut full: Vec<&str> = args[..1].to_vec();
    full.extend_from_slice(&["-C", dir.to_str().unwrap()]);
    full.extend_from_slice(&args[1..]);
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
    helpers::CliOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        success: output.status.success(),
    }
}

/// A pipeline on the root's topic branch takes each child whose remote has
/// that branch from it — detached at its tip — and `check` fails until the
/// pins say so.
#[test]
fn a_pipeline_on_a_topic_takes_children_from_their_branches() {
    let f = fixture("wt_ci_topic", "");
    let tip = f.core_commit("feat/x", "lib.txt", "remote work");
    // The runner's own checkout of the root, detached at the pipeline's
    // commit.
    let job = f.env.repos_remote.join("job");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            job.to_str().unwrap(),
        ],
    );
    run_git_pub(&job, &["checkout", "-q", "--detach"]);
    let out = ci_job(&job, &f.env.cache, "feat/x", &["pull"]);
    ok(&out);
    let child = job.join("imports/core");
    assert_eq!(head(&child), tip);
    assert_eq!(branch(&child), None);

    let out = ci_job(&job, &f.env.cache, "feat/x", &["check"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "imports/core was taken from branch feat/x, not from v1.0.0 pinned in .gitscale.toml"
        ),
        "{}",
        out.stderr
    );

    // On main nothing is followed, and the gate passes.
    let out = ci_job(&job, &f.env.cache, "main", &["pull"]);
    ok(&out);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    ok(&ci_job(&job, &f.env.cache, "main", &["check"]));
}

/// A merge request into a branch the root does not pin is not gated.
#[test]
fn check_gates_only_merges_into_pinned_branches() {
    let f = fixture("wt_ci_gate", "");
    f.core_commit("feat/x", "lib.txt", "remote work");
    let job = f.env.repos_remote.join("job");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            job.to_str().unwrap(),
        ],
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(["check", "-C", job.to_str().unwrap()])
        .env("CI", "true")
        .env("GITLAB_CI", "true")
        .env("CI_COMMIT_SHA", git(&job, &["rev-parse", "HEAD"]))
        .env("CI_MERGE_REQUEST_SOURCE_BRANCH_NAME", "feat/x")
        .env("CI_MERGE_REQUEST_TARGET_BRANCH_NAME", "integration")
        .env("CI_DEFAULT_BRANCH", "main")
        .env("GITSCALE_CACHE_DIR", &f.env.cache)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("not a pinned branch"));
}

// ---------------------------------------------------------------------------
// Promotion and raising pins
// ---------------------------------------------------------------------------

/// Once a release tag holds a developed child's change, `upgrade` writes the
/// tag into the root's config, deletes the child's topic branch and detaches
/// it at the tag.
#[test]
fn upgrade_promotes_a_child_whose_change_is_released() {
    let f = fixture("wt_promote", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    let out = gs(&ws, &["push"]);
    ok(&out);

    // Merged upstream, squashed, and released.
    let released = f.core_commit("main", "lib.txt", "v2");
    run_git_pub(&f.core, &["tag", "v1.1.0", &released]);
    run_git_pub(&f.core, &["branch", "-D", "feat/x"]);

    let out = gs(&ws, &["upgrade", "--commit"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core  promoted → v1.1.0"),
        "{}",
        out.stdout
    );
    let config = std::fs::read_to_string(ws.join(".gitscale.toml")).unwrap();
    assert!(config.contains("revision = \"v1.1.0\""), "{}", config);
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%s"]),
        "pin imports/core v1.1.0"
    );
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), released);
    assert!(!git_ok(
        &child,
        &["rev-parse", "--verify", "-q", "refs/heads/feat/x"]
    ));
}

/// `upgrade <dir>` raises a dependency to its newest release in the root's
/// config.
#[test]
fn upgrade_raises_a_named_dependency_to_its_newest_release() {
    let f = fixture("wt_raise", "");
    let ws = f.clone_root("ws");
    let newer = f.core_commit("main", "lib.txt", "v1.2");
    run_git_pub(&f.core, &["tag", "v1.2.0", &newer]);
    let out = gs(&ws, &["upgrade", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core   v1.0.0 → v1.2.0"),
        "{}",
        out.stdout
    );
    let config = std::fs::read_to_string(ws.join(".gitscale.toml")).unwrap();
    assert!(config.contains("revision = \"v1.2.0\""), "{}", config);
}

// ---------------------------------------------------------------------------
// Artefacts on a topic, and overlays
// ---------------------------------------------------------------------------

/// A root whose `meta/app` uses `app`'s artefact as `artefact_use`.
fn artefact_workspace(env: &TestEnv, app: &Path, artefact_use: &str) -> PathBuf {
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

/// On the topic, an artefact that replaces its checkout is the image of the
/// branch tip when one is published, and that tip's source when not.
#[test]
fn an_artefact_on_a_topic_is_the_tips_image_or_its_source() {
    let env = TestEnv::new("wt_artefact_topic");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, "replace");
    ok(&gs(&ws, &["pull"]));
    let dest = ws.join("meta/app");
    assert!(dest.join("dist/app.bin").is_file());

    // A branch of the topic, published.
    run_git_pub(&app, &["branch", "feat/x", "main"]);
    env.push_commit(&app, "feat/x", "README.md", "feature");
    env.publish(&app, "feat/x", &[("app.bin", "feature build")]);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "feature build"
    );
    assert!(!dest.join(".git").exists());

    // A newer tip with no image yet: its source, detached.
    let unpublished = env.push_commit(&app, "feat/x", "README.md", "newer");
    let out = gs(&ws, &["pull"]);
    ok(&out);
    assert!(dest.join(".git").is_file(), "{}", out.stdout);
    assert_eq!(head(&dest), unpublished);
    assert_eq!(branch(&dest), None);

    // Developed: the source, on its branch.
    ok(&gs(&ws, &["develop", "meta/app"]));
    assert_eq!(branch(&dest).as_deref(), Some("feat/x"));

    // Back on main: the image again.
    run_git_pub(&dest, &["checkout", "-q", "--detach"]);
    run_git_pub(&dest, &["branch", "-q", "-D", "feat/x"]);
    run_git_pub(&ws, &["switch", "-q", "main"]);
    run_git_pub(&app, &["branch", "-D", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    assert!(!dest.join(".git").exists());
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "main build"
    );
}

/// An overlay is the source checkout with the image's untracked files laid
/// over it: tracked files are the checkout's own, and the next overlay
/// removes what this one wrote before laying its own. `clean` keeps them.
#[test]
fn an_overlay_lays_the_build_over_the_source() {
    let env = TestEnv::new("wt_overlay");
    // Build output is ignored, as the artefact policy has it.
    let app = env.create_bare_repo(
        "app",
        "main",
        &[
            ("README.md", "app"),
            ("src.txt", "source"),
            (".gitignore", "/dist/\n"),
        ],
    );
    let producer_files = [("app.bin", "v1 build"), ("old.bin", "goes away")];
    env.publish(&app, "main", &producer_files);
    let ws = artefact_workspace(&env, &app, "overlay");
    ok(&gs(&ws, &["pull"]));
    let dest = ws.join("meta/app");
    assert!(dest.join(".git").is_file(), "a source worktree");
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "v1 build"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("src.txt")).unwrap(),
        "source"
    );
    assert!(
        git(&dest, &["status", "--porcelain"]).is_empty(),
        "the overlay is ignored output"
    );

    // A clean keeps the overlay's files.
    ok(&gs(&ws, &["clean", "-f"]));
    assert!(dest.join("dist/app.bin").is_file());

    // A new commit with a smaller build: the old file goes.
    env.push_commit(&app, "main", "README.md", "v2");
    env.publish(&app, "main", &[("app.bin", "v2 build")]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "v2 build"
    );
    assert!(!dest.join("dist/old.bin").exists());
}
