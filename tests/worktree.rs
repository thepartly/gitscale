// A workspace laid out as a bare repository plus linked worktrees: the git
// directory is not named `.git`, and each worktree's own `.git` is a file
// pointing into it. Every other test here uses a plain `git init` playground,
// where git behaves differently in one way that matters — a checkout in a
// linked worktree exports GIT_DIR to its hooks, and in an ordinary repository
// it does not.
#[allow(dead_code)]
mod helpers;

use helpers::TestEnv;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Git with the developer's own config and the system hook path kept out of it,
/// so a gitscale installed on this machine cannot fire during the test.
fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@t.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@t.com")
        .output()
        .expect("failed to run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Run the gitscale binary against `root`, with extra environment variables.
fn gitscale(root: &Path, args: &[&str], vars: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gitscale"));
    cmd.arg(args[0])
        .arg("-C")
        .arg(root)
        .args(&args[1..])
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GITSCALE_HOOK_ALLOW");
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to run the gitscale binary");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

/// A bare repository at `<playground>/.bare` with a linked worktree at
/// `<playground>/ws`, carrying `config` as its gitscale config. Returns the
/// worktree, which is the workspace root gitscale operates on.
fn bare_worktree_workspace(env: &TestEnv, config: &str) -> PathBuf {
    let pg = &env.playground;
    let upstream = pg.join("upstream");
    std::fs::create_dir_all(&upstream).unwrap();
    git(
        pg,
        &["init", "-q", "-b", "main", upstream.to_str().unwrap()],
    );
    std::fs::write(upstream.join(".gitscale.toml"), config).unwrap();
    std::fs::write(upstream.join(".gitignore"), "imports/\n").unwrap();
    git(&upstream, &["add", "-A"]);
    git(&upstream, &["commit", "-qm", "init"]);

    let bare = pg.join(".bare");
    git(
        pg,
        &[
            "clone",
            "-q",
            "--bare",
            upstream.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let ws = pg.join("ws");
    git(
        &bare,
        &["worktree", "add", "-q", ws.to_str().unwrap(), "main"],
    );
    ws
}

fn head_of(repo: &Path) -> String {
    git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
}

fn config_for(url: &str) -> String {
    format!(
        "[repos]\n\"imports/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        url
    )
}

#[test]
fn clone_and_pull_work_in_a_linked_worktree() {
    let env = TestEnv::new("worktree_clone_pull");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# v1\n")]);
    let ws = bare_worktree_workspace(&env, &config_for(&bare.display().to_string()));

    // the layout is the point: a git dir not named `.git`, reached by a file
    assert!(env.playground.join(".bare").is_dir());
    assert!(ws.join(".git").is_file());

    let (ok, out) = gitscale(&ws, &["clone"], &[]);
    assert!(ok, "clone failed: {out}");
    let (ok, out) = gitscale(&ws, &["pull"], &[]);
    assert!(ok, "pull failed: {out}");

    assert_eq!(head_of(&ws), "main", "the worktree moved off its branch");
    assert_eq!(
        head_of(&ws.join("imports/mylib")),
        "main",
        "the import is not on its pinned revision"
    );
}

#[test]
fn a_pull_leaves_the_worktree_alone_when_an_import_is_not_a_repo() {
    let env = TestEnv::new("worktree_import_not_a_repo");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# v1\n")]);
    let ws = bare_worktree_workspace(&env, &config_for(&bare.display().to_string()));
    git(&ws, &["checkout", "-q", "-b", "feature"]);

    // exists, holds no repository: discovery would otherwise climb to the worktree
    std::fs::create_dir_all(ws.join("imports/mylib")).unwrap();

    let (ok, _) = gitscale(&ws, &["pull"], &[]);
    assert!(!ok, "a pull into a non-repository should fail");
    assert_eq!(head_of(&ws), "feature", "the pull moved the worktree");
}

#[test]
fn a_pull_carrying_the_worktrees_git_dir_acts_on_the_import() {
    let env = TestEnv::new("worktree_git_dir_inherited");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# v1\n")]);
    let ws = bare_worktree_workspace(&env, &config_for(&bare.display().to_string()));
    assert!(gitscale(&ws, &["clone"], &[]).0);
    git(&ws, &["checkout", "-q", "-b", "feature"]);

    // what git exports to a hook it runs from a linked worktree
    let git_dir = git(&ws, &["rev-parse", "--absolute-git-dir"]);
    let (ok, out) = gitscale(&ws, &["pull"], &[("GIT_DIR", git_dir.as_str())]);
    assert!(ok, "pull failed: {out}");

    assert_eq!(head_of(&ws), "feature", "the pull moved the worktree");
    let import = ws.join("imports/mylib");
    assert_eq!(
        git(&import, &["status", "--porcelain"]),
        "",
        "the worktree's files were written into the import"
    );
}
