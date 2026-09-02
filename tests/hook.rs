// Each integration test binary compiles `helpers` separately; this one uses
// only part of it.
#[allow(dead_code)]
mod helpers;

use helpers::TestEnv;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// `gitscale hook` — install / uninstall / shim behaviour
//
// Only `--local` is exercised. `--global` and `--system` write to the user's
// (or the machine's) git config, which a test must never touch.
// ---------------------------------------------------------------------------

/// Run the CLI directly: `TestEnv::run` injects `-C` after the first
/// subcommand, which lands in the wrong place for a nested one like `hook`.
fn cli(args: &[&str]) -> helpers::CliOutput {
    let mut full = vec!["gitscale"];
    full.extend_from_slice(args);
    gitscale::run_cli(&full)
}

/// A git repo with an initial commit, optionally carrying a gitscale config.
fn repo_with(env: &TestEnv, config: Option<&str>) -> PathBuf {
    let repo = env.playground.clone();
    helpers::run_git_pub(&repo, &["init", "-q", "-b", "main"]);
    helpers::run_git_pub(&repo, &["config", "user.email", "t@t.com"]);
    helpers::run_git_pub(&repo, &["config", "user.name", "T"]);
    if let Some(text) = config {
        std::fs::write(repo.join(".gitscale.toml"), text).unwrap();
    }
    std::fs::write(repo.join("f.txt"), "v1").unwrap();
    helpers::run_git_pub(&repo, &["add", "-A"]);
    helpers::run_git_pub(&repo, &["commit", "-q", "-m", "init"]);
    repo
}

fn hooks_dir(repo: &Path) -> PathBuf {
    repo.join(".git/hooks")
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

/// Execute an installed hook script the way git would.
fn run_hook(script: &Path, cwd: &Path, sentinel: Option<&str>) -> (i32, String) {
    let mut cmd = Command::new("sh");
    cmd.arg(script).current_dir(cwd);
    match sentinel {
        Some(v) => cmd.env("GITSCALE_HOOK", v),
        None => cmd.env_remove("GITSCALE_HOOK"),
    };
    let out = cmd.output().expect("failed to run hook");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

#[test]
fn install_local_writes_both_hooks() {
    let env = TestEnv::new("hook_install_local");
    let repo = repo_with(&env, Some("[repos]\n"));

    let out = cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]);
    assert!(out.success, "stderr: {}", out.stderr);

    for hook in ["post-checkout", "post-merge"] {
        let path = hooks_dir(&repo).join(hook);
        assert!(path.is_file(), "{} was not installed", hook);
        assert!(is_executable(&path), "{} is not executable", hook);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("installed by gitscale"),
            "{} lacks the gitscale marker",
            hook
        );
    }
}

#[test]
fn install_displaces_and_chains_an_existing_hook() {
    let env = TestEnv::new("hook_install_chains");
    let repo = repo_with(&env, Some("[repos]\n"));
    let own = hooks_dir(&repo).join("post-checkout");
    std::fs::write(&own, "#!/bin/sh\necho MINE\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let out = cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]);
    assert!(out.success, "stderr: {}", out.stderr);

    let displaced = hooks_dir(&repo).join("post-checkout.local");
    assert!(
        displaced.is_file(),
        "the repo's own hook should have been moved aside, not destroyed"
    );
    assert_eq!(
        std::fs::read_to_string(&displaced).unwrap(),
        "#!/bin/sh\necho MINE\n"
    );

    let shim = std::fs::read_to_string(hooks_dir(&repo).join("post-checkout")).unwrap();
    assert!(
        shim.contains(displaced.to_str().unwrap()),
        "the shim should chain to the displaced hook"
    );
}

#[test]
fn uninstall_restores_the_displaced_hook() {
    let env = TestEnv::new("hook_uninstall_restores");
    let repo = repo_with(&env, Some("[repos]\n"));
    let own = hooks_dir(&repo).join("post-checkout");
    std::fs::write(&own, "#!/bin/sh\necho MINE\n").unwrap();

    let root = repo.to_str().unwrap();
    assert!(cli(&["hook", "install", "--local", "-C", root]).success);
    assert!(cli(&["hook", "uninstall", "--local", "-C", root]).success);

    assert_eq!(
        std::fs::read_to_string(&own).unwrap(),
        "#!/bin/sh\necho MINE\n",
        "the repo's own hook should be back where git expects it"
    );
    assert!(
        !hooks_dir(&repo).join("post-checkout.local").exists(),
        "the displaced copy should be gone"
    );
    assert!(!hooks_dir(&repo).join("post-merge").exists());
}

#[test]
fn shim_is_silent_in_a_repo_without_a_gitscale_config() {
    let env = TestEnv::new("hook_shim_silent");
    let repo = repo_with(&env, None); // no .gitscale.toml
    assert!(cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]).success);

    let (code, output) = run_hook(&hooks_dir(&repo).join("post-checkout"), &repo, None);
    assert_eq!(code, 0, "must not disturb repos that never opted in");
    assert!(
        output.trim().is_empty(),
        "expected no output in a non-gitscale repo, got: {}",
        output
    );
}

#[test]
fn shim_runs_the_chained_hook_and_honours_the_recursion_guard() {
    let env = TestEnv::new("hook_shim_chain");
    // A config is present, so without the guard the shim would invoke gitscale.
    let repo = repo_with(&env, Some("[repos]\n"));
    let own = hooks_dir(&repo).join("post-checkout");
    std::fs::write(&own, "#!/bin/sh\necho CHAINED\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]).success);

    // GITSCALE_HOOK set == this git call came from gitscale itself.
    let (code, output) = run_hook(&hooks_dir(&repo).join("post-checkout"), &repo, Some("1"));
    assert_eq!(code, 0);
    assert!(
        output.contains("CHAINED"),
        "the repo's own hook must still run: {}",
        output
    );
    assert!(
        !output.contains("Pulling"),
        "the recursion guard should have stopped gitscale from running: {}",
        output
    );
}

#[test]
fn hook_run_rejects_an_unknown_hook_name() {
    let env = TestEnv::new("hook_run_unknown");
    let repo = repo_with(&env, Some("[repos]\n"));
    let out = cli(&["hook", "run", "pre-rebase", "-C", repo.to_str().unwrap()]);
    assert!(
        !out.success,
        "expected failure for a hook gitscale never installs"
    );
    assert!(
        out.stderr.contains("unknown hook"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn hook_status_reports_installed_hooks() {
    let env = TestEnv::new("hook_status");
    let repo = repo_with(&env, Some("[repos]\n"));
    let root = repo.to_str().unwrap();
    assert!(cli(&["hook", "install", "--local", "-C", root]).success);

    let out = cli(&["hook", "status", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("post-checkout"), "{}", out.stdout);
    assert!(out.stdout.contains("gitscale"), "{}", out.stdout);
}
