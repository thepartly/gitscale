// A CI runner that keeps its build directory between jobs — a GitLab shell
// runner — updates the checkouts it already has instead of cloning them again,
// which is what keeps unchanged files' mtimes and so the build cache valid.
// These tests hold that path to the one promise it makes: each job starts from
// what a fresh clone would have given it — the pinned commit, nothing
// untracked — and a pipeline that would delete the checkouts after the hook
// populated them fails at the checkout, not somewhere downstream.
#[allow(dead_code)]
mod helpers;

use helpers::TestEnv;
use std::path::Path;

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {:?} in {} failed:\n{}",
        args,
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A bare repo whose `main` has commits `c1`..`c3`, tagged `v1`..`v3`, plus a
/// `feature` branch off `v1`.
fn tagged_remote(env: &TestEnv) -> std::path::PathBuf {
    let bare = env.create_bare_repo("lib", "main", &[("f.txt", "0")]);
    let work = env.repos_remote.join("lib-work");
    git(
        &env.repos_remote,
        &[
            "clone",
            "-q",
            "-b",
            "main",
            bare.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    git(&work, &["config", "user.email", "t@t"]);
    git(&work, &["config", "user.name", "T"]);
    for i in 1..=3 {
        std::fs::write(work.join("f.txt"), i.to_string()).unwrap();
        git(&work, &["commit", "-qam", &format!("c{}", i)]);
        git(&work, &["tag", &format!("v{}", i)]);
    }
    git(&work, &["checkout", "-q", "-b", "feature", "v1"]);
    std::fs::write(work.join("f.txt"), "feature").unwrap();
    git(&work, &["commit", "-qam", "feat"]);
    git(
        &work,
        &["push", "-q", "origin", "main", "feature", "--tags"],
    );
    bare
}

fn pin(env: &TestEnv, bare: &Path, revision: &str, cache: bool) {
    let cache = if cache {
        format!("[cache]\ndir = \"{}\"\n\n", env.cache.display())
    } else {
        "[cache]\nenabled = false\n\n".to_string()
    };
    env.write_config(&format!(
        // file:// — git ignores `--depth` for a plain-path remote, and these
        // tests are about the shallow checkout CI makes.
        "{}[repos]\n\"libs/lib\" = {{ url = \"file://{}\", revision = \"{}\" }}\n",
        cache,
        bare.display(),
        revision
    ));
}

fn subject(dir: &Path) -> String {
    git(dir, &["log", "-1", "--format=%s"])
}

/// Without a cache a shallow checkout's own refspec is the one branch it was
/// cloned at, so updating it by `fetch` + `reset @{upstream}` never reached a
/// tag, or another branch, the config had moved to — and reported `ok`.
#[test]
fn a_kept_checkout_follows_every_pin_change_without_the_cache() {
    let env = TestEnv::new("ci_pin_moves");
    let bare = tagged_remote(&env);
    let lib = env.playground.join("libs/lib");

    for (revision, expected) in [
        ("v1", "c1"),
        ("v2", "c2"),
        ("v1", "c1"),
        ("main", "c3"),
        ("v2", "c2"),
        ("feature", "feat"),
    ] {
        pin(&env, &bare, revision, false);
        let out = env.run_with_env(&[("CI", "true")], &["pull"]);
        assert!(
            out.success,
            "pin {}: {}{}",
            revision, out.stdout, out.stderr
        );
        assert_eq!(subject(&lib), expected, "pin {}", revision);
        // Detached at the commit, branch or tag alike: what the workspace
        // gets, and what the pipeline builds.
        let head = git(&lib, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(head, "HEAD", "pin {}: detached", revision);
    }
}

#[test]
fn a_pin_the_remote_does_not_have_fails_instead_of_staying_put() {
    let env = TestEnv::new("ci_pin_missing");
    let bare = tagged_remote(&env);
    pin(&env, &bare, "v1", false);
    assert!(env.run_with_env(&[("CI", "true")], &["pull"]).success);

    pin(&env, &bare, "v9", false);
    let out = env.run_with_env(&[("CI", "true")], &["pull"]);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(!out.success, "{}", text);
    // Refused while resolving, before any checkout is touched.
    assert!(
        text.contains("'v9' is not a branch, tag or commit"),
        "{}",
        text
    );
}

/// In place is what keeps the build cache valid, and what lets the last job's
/// leftovers through: in CI the pull finishes the job a fresh clone would do.
#[test]
fn a_ci_pull_leaves_each_checkout_as_a_fresh_clone_would() {
    let env = TestEnv::new("ci_scrub");
    env.init_playground_git();
    let bare = tagged_remote(&env);
    for cache in [false, true] {
        let _ = std::fs::remove_dir_all(env.playground.join("libs"));
        pin(&env, &bare, "v1", cache);
        assert!(env.run_with_env(&[("CI", "true")], &["pull"]).success);

        let lib = env.playground.join("libs/lib");
        std::fs::write(lib.join("stray.txt"), "left behind").unwrap();
        std::fs::create_dir_all(lib.join("target")).unwrap();
        std::fs::write(lib.join("target/out.o"), "build output").unwrap();
        std::fs::write(lib.join(".git/info/exclude"), "target/\n").unwrap();
        std::fs::write(env.playground.join("root-stray.txt"), "not ours").unwrap();

        pin(&env, &bare, "v2", cache);
        let out = env.run_with_env(&[("CI", "true")], &["pull"]);
        assert!(out.success, "{}{}", out.stdout, out.stderr);
        assert_eq!(subject(&lib), "c2");
        assert_eq!(
            git(&lib, &["status", "--porcelain", "--ignored"]),
            "",
            "cache {}: untracked and ignored files survived",
            cache
        );
        assert!(
            env.playground.join("root-stray.txt").exists(),
            "the workspace root is the runner's to clean, not gitscale's"
        );
    }
}

/// The CI pull ends with exactly `gitscale clean -f <each checkout>`: the same
/// rules — a checkout's own keep-list included — and the same report.
#[test]
fn a_ci_pull_cleans_each_checkout_as_clean_f_does() {
    let env = TestEnv::new("ci_scrub_keep_list");
    let bare = env.create_bare_repo(
        "lib",
        "main",
        &[
            ("f.txt", "0"),
            (".gitscale.toml", "[clean]\nexclude = [\"envs/\"]\n"),
        ],
    );
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"file://{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    assert!(env.run_with_env(&[("CI", "true")], &["pull"]).success);

    let lib = env.playground.join("libs/lib");
    let kept = lib.join("envs/dev");
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, "kept by its own config").unwrap();
    std::fs::write(lib.join("stray.txt"), "left behind").unwrap();

    let out = env.run_with_env(&[("CI", "true")], &["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(kept.exists(), "the checkout's own keep-list applies");
    assert!(!lib.join("stray.txt").exists());
    assert!(
        out.stdout.contains("Removing untracked files...")
            && out.stdout.contains("libs/lib (1 path)"),
        "the report is clean -f's: {}",
        out.stdout
    );
}

#[test]
fn outside_ci_a_pull_leaves_untracked_files_alone() {
    let env = TestEnv::new("ci_scrub_local");
    let bare = tagged_remote(&env);
    pin(&env, &bare, "v1", false);
    assert!(env.run_with_env(&[("CI", "false")], &["pull"]).success);
    let stray = env.playground.join("libs/lib/notes.txt");
    std::fs::write(&stray, "mine").unwrap();
    assert!(env.run_with_env(&[("CI", "false")], &["pull"]).success);
    assert!(
        stray.exists(),
        "a developer's untracked work is not a leftover"
    );
}

// ---------------------------------------------------------------------------
// GitLab Runner cleans after its checkout
// ---------------------------------------------------------------------------

fn gitlab_workspace(env: &TestEnv, extra_config: &str) {
    env.init_playground_git();
    // As the docs recommend — and what makes `-ffd`, without `-x`, keep them.
    std::fs::write(env.playground.join(".gitignore"), "/imports/\n").unwrap();
    let bare = env.create_bare_repo("lib", "main", &[("f.txt", "0")]);
    env.write_config(&format!(
        "{}[repos]\n\"imports/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        extra_config,
        bare.display()
    ));
}

#[test]
fn a_gitlab_hook_pull_fails_when_the_runner_would_delete_the_checkouts() {
    let env = TestEnv::new("gitlab_clean_flags_missing");
    // `warn` does not soften it: this is not a pull that failed but a pipeline
    // that cannot work.
    gitlab_workspace(&env, "[hooks]\non_pull_error = \"warn\"\n\n");

    let out = env.run_hook_run(
        "post-checkout",
        "*",
        &[
            ("GITLAB_CI", "true"),
            ("CI", "true"),
            ("GIT_CLEAN_FLAGS", "-ffdx"),
        ],
    );
    assert!(!out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stderr.contains("GIT_CLEAN_FLAGS: -ffdx -e /imports/"),
        "the error must carry the line to add: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("would delete imports/lib"),
        "{}",
        out.stderr
    );
}

#[test]
fn the_runner_default_clean_flags_are_checked_too() {
    let env = TestEnv::new("gitlab_clean_flags_default");
    gitlab_workspace(&env, "");
    let out = env.run_hook_run(
        "post-checkout",
        "*",
        &[("GITLAB_CI", "true"), ("CI", "true")],
    );
    assert!(
        !out.success,
        "unset means the runner's -ffdx: {}",
        out.stderr
    );
}

#[test]
fn an_exclude_that_keeps_the_checkouts_passes() {
    for flags in ["-ffdx -e /imports/", "-ffdx -e imports", "-ffd", "none"] {
        let env = TestEnv::new("gitlab_clean_flags_ok");
        gitlab_workspace(&env, "");
        let out = env.run_hook_run(
            "post-checkout",
            "*",
            &[
                ("GITLAB_CI", "true"),
                ("CI", "true"),
                ("GIT_CLEAN_FLAGS", flags),
            ],
        );
        assert!(
            out.success,
            "GIT_CLEAN_FLAGS={}: {}{}",
            flags, out.stdout, out.stderr
        );
        assert!(env.playground.join("imports/lib/f.txt").exists());
    }
}

#[test]
fn an_exclude_covering_only_some_checkouts_names_the_rest() {
    let env = TestEnv::new("gitlab_clean_flags_partial");
    env.init_playground_git();
    let a = env.create_bare_repo("a", "main", &[("a.txt", "a")]);
    let b = env.create_bare_repo("b", "main", &[("b.txt", "b")]);
    env.write_config(&format!(
        "[repos]\n\"imports/a\" = {{ url = \"{}\", revision = \"main\" }}\n\"vendor/b\" = {{ url = \"{}\", revision = \"main\" }}\n",
        a.display(),
        b.display()
    ));
    let out = env.run_hook_run(
        "post-checkout",
        "*",
        &[
            ("GITLAB_CI", "true"),
            ("CI", "true"),
            ("GIT_CLEAN_FLAGS", "-ffdx -e /imports/"),
        ],
    );
    assert!(!out.success);
    assert!(
        out.stderr.contains("would delete vendor/b"),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains("imports/a,"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("GIT_CLEAN_FLAGS: -ffdx -e /imports/ -e /vendor/"),
        "{}",
        out.stderr
    );
}

#[test]
fn outside_gitlab_the_clean_flags_are_not_consulted() {
    let env = TestEnv::new("gitlab_clean_flags_elsewhere");
    gitlab_workspace(&env, "");
    let out = env.run_hook_run(
        "post-checkout",
        "*",
        &[("GITLAB_CI", "false"), ("GIT_CLEAN_FLAGS", "-ffdx")],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
}
