//! Helpers for the workspace tests.

use super::{git_stdout, run_git_pub, TestEnv};
/// A root workspace whose `repoA` declares `libs/b`, deduped to the root's
/// `repoB`. The root pins B at `main`; the child asks for `develop`, and the
/// root wins. Returns the `repoA` path and `main`'s tip.
pub fn child_with_outer_link(env: &TestEnv) -> (std::path::PathBuf, String) {
    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "B main")]);
    bare_git_stdout(&bare_b, &["branch", "develop", "main"]);
    commit_to_bare(&bare_b, "develop", "b.txt", "B develop");
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"develop\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"repoB" = {{ url = "{}", revision = "main" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));
    assert!(env.run(&["pull"]).success);
    assert!(
        env.playground.join("repoA/libs/b").is_symlink(),
        "clone should dedup repoA/libs/b"
    );
    (
        env.playground.join("repoA"),
        bare_git_stdout(&bare_b, &["rev-parse", "main"]),
    )
}

/// Run the CLI with `-C dir` after the subcommand, then `rest`.
pub fn run_in(dir: &std::path::Path, subcommand: &str, rest: &[&str]) -> super::CliOutput {
    let mut args = vec!["gitscale", subcommand, "-C", dir.to_str().unwrap()];
    args.extend_from_slice(rest);
    gitscale::run_cli_with(&args, false)
}

/// A workspace whose own `origin` gives the allowlist something to match on.
pub fn hook_env(name: &str, origin: &str) -> TestEnv {
    let env = TestEnv::new(name);
    env.init_playground_git();
    env.set_playground_origin(origin);
    env
}

/// Helper: set up a workspace with repoA (recursive) depending on repoB,
/// clone it (creating a symlink), then replace the symlink with a real clone.
/// Returns (env, path_to_link).
pub fn setup_unlinked_env(name: &str) -> (TestEnv, std::path::PathBuf) {
    setup_unlinked_env_at(name, "repoA")
}

/// The same, with repoA declared at `parent` — which may be more than one
/// path component deep.
pub fn setup_unlinked_env_at(name: &str, parent: &str) -> (TestEnv, std::path::PathBuf) {
    let env = TestEnv::new(name);

    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "hello from B")]);
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "hello from A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );

    env.write_config(&format!(
        r#"[repos]
"{}" = {{ url = "{}", revision = "main", recursive = true }}
"repoB" = {{ url = "{}", revision = "main" }}
"#,
        parent,
        bare_a.display(),
        bare_b.display(),
    ));

    // Clone creates the symlink
    let out = env.run(&["pull"]);
    assert!(out.success, "clone failed: {}", out.stderr);

    let link = env.playground.join(parent).join("libs/b");
    assert!(link.is_symlink(), "expected symlink after clone");

    // Replace symlink with a real clone (simulating `gitscale sync` from inside repoA)
    std::fs::remove_file(&link).unwrap();
    super::run_git_pub(
        &env.playground,
        &[
            "clone",
            "--branch",
            "main",
            bare_b.to_str().unwrap(),
            link.to_str().unwrap(),
        ],
    );
    assert!(!link.is_symlink(), "should be a real dir now");
    assert!(link.join("b.txt").is_file());

    (env, link)
}

/// Helper: set up a workspace with repoA (recursive) depending on repoB, clone
/// it, then plant an orphaned gitscale-style symlink `repoA/libs/c -> ../../repoC`
/// for a dependency that is not declared anywhere (simulating a removed dep).
/// When `valid_target` is true the target `repoC` dir is created so the orphan
/// resolves; otherwise the orphan is left broken. Returns (env, orphan_link).
pub fn setup_orphan_env(name: &str, valid_target: bool) -> (TestEnv, std::path::PathBuf) {
    let env = TestEnv::new(name);

    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "hello from B")]);
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "hello from A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );

    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main", recursive = true }}
"repoB" = {{ url = "{}", revision = "main" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));

    let out = env.run(&["pull"]);
    assert!(out.success, "clone failed: {}", out.stderr);

    // Plant an orphaned gitscale-style symlink for an undeclared dependency.
    let orphan = env.playground.join("repoA/libs/c");
    std::os::unix::fs::symlink("../../repoC", &orphan).unwrap();
    if valid_target {
        std::fs::create_dir_all(env.playground.join("repoC")).unwrap();
        std::fs::write(env.playground.join("repoC/c.txt"), "hello from C").unwrap();
    }

    (env, orphan)
}

/// A workspace on topic `feat/x` with `libs/mylib` developed on it: the root
/// on that branch with a remote of its own, the child on its `feat/x`.
pub fn setup_commit_env(name: &str) -> (TestEnv, std::path::PathBuf, std::path::PathBuf) {
    let env = TestEnv::new(name);
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# v1\n")]);
    let root_remote = env.create_bare_repo("root", "main", &[("README.md", "root")]);
    env.write_config(&format!(
        "[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    env.init_playground_git();
    env.set_playground_origin(root_remote.to_str().unwrap());
    let out = env.run(&["pull"]);
    assert!(out.success, "pull failed: {}", out.stderr);
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let out = env.run(&["develop", "libs/mylib"]);
    assert!(out.success, "develop failed: {}{}", out.stdout, out.stderr);

    let clone = env.playground.join("libs/mylib");
    super::run_git_pub(&clone, &["config", "user.email", "t@t.com"]);
    super::run_git_pub(&clone, &["config", "user.name", "T"]);
    (env, bare, clone)
}

/// Run git against a bare repo and return trimmed stdout. Addresses the git
/// dir explicitly, since discovery-based access to a bare repo is refused when
/// the developer has `safe.bareRepository = explicit` set.
#[cfg(test)]
pub fn bare_git_stdout(bare: &std::path::Path, args: &[&str]) -> String {
    let mut full = vec!["--git-dir", bare.to_str().unwrap()];
    full.extend_from_slice(args);
    let output = std::process::Command::new("git")
        .args(&full)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        full,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Add a commit to `branch` of a bare repo via a throwaway clone.
/// Returns the new commit SHA.
#[cfg(test)]
pub fn commit_to_bare(bare: &std::path::Path, branch: &str, file: &str, content: &str) -> String {
    let tmp = bare.with_extension("commit-tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    super::run_git_pub(
        bare.parent().unwrap(),
        &[
            "clone",
            bare.to_str().unwrap(),
            tmp.to_str().unwrap(),
            "--branch",
            branch,
        ],
    );
    super::run_git_pub(&tmp, &["config", "user.email", "test@test.com"]);
    super::run_git_pub(&tmp, &["config", "user.name", "Test"]);
    std::fs::write(tmp.join(file), content).unwrap();
    super::run_git_pub(&tmp, &["add", "-A"]);
    super::run_git_pub(&tmp, &["commit", "-m", "update"]);
    super::run_git_pub(&tmp, &["push", "origin", &format!("{}:{}", branch, branch)]);
    let sha = git_stdout(&tmp, &["rev-parse", "HEAD"]);
    let _ = std::fs::remove_dir_all(&tmp);
    sha
}

/// A workspace that is itself a git repo, with one readwrite repo cloned in.
pub fn clean_env(name: &str) -> TestEnv {
    let env = TestEnv::new(name);
    let core = env.create_bare_repo("core", "main", &[("README.md", "core")]);
    env.write_config(&format!(
        "[repos]\n\"core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        core.to_str().unwrap()
    ));
    env.init_playground_git();
    let out = env.run(&["pull"]);
    assert!(out.success, "clone failed: {}", out.stderr);
    env
}

pub fn git_out(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run git");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A workspace that is a git repository on a branch of its own, declaring
/// `libs/core` pinned to `main` — a name the workspace also has — with the
/// entry's directory created by `setup` rather than by a clone.
pub fn workspace_with_stray_directory(name: &str, setup: impl FnOnce(&std::path::Path)) -> TestEnv {
    let env = TestEnv::new(name);
    let core = env.create_bare_repo("core", "main", &[("README.md", "core")]);
    env.write_config(&format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        core.to_str().unwrap()
    ));
    env.init_playground_git();
    git_out(&env.playground, &["checkout", "-q", "-B", "main"]);
    git_out(&env.playground, &["checkout", "-q", "-b", "feature"]);
    let dir = env.playground.join("libs/core");
    std::fs::create_dir_all(&dir).unwrap();
    setup(&dir);
    env
}

/// A `git` on `PATH` that logs every invocation's arguments, one line each,
/// then runs the real git: how a perf test counts what gitscale asks git to
/// do. Returns the `PATH` to run the binary with and the log file.
pub fn counting_git(env: &TestEnv) -> (String, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var("PATH").unwrap_or_default();
    let real = std::env::split_paths(&path)
        .map(|dir| dir.join("git"))
        .find(|candidate| candidate.is_file())
        .expect("git is on PATH");
    let dir = env.repos_remote.join("counting-git");
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("git.log");
    let shim = dir.join("git");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    (format!("{}:{}", dir.display(), path), log)
}

/// How many logged git invocations contain `needle`, and empty the log.
pub fn take_git_count(log: &std::path::Path, needle: &str) -> usize {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let _ = std::fs::remove_file(log);
    text.lines().filter(|line| line.contains(needle)).count()
}
