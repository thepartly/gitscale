//! `gitscale commit`: one message across the checkouts that have changes.

use crate::support;
use crate::support::resolution::*;
use crate::support::workspace::*;
use crate::support::{git_stdout, run_git_pub, strip_ansi, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_commits_every_change_in_a_dirty_checkout() {
    let (env, _bare, clone) = setup_commit_env("commit_dirty");

    // Make it dirty: modify a tracked file and add an untracked one.
    std::fs::write(clone.join("README.md"), "# v2\n").unwrap();
    std::fs::write(clone.join("new.txt"), "new").unwrap();

    let out = env.run(&["commit", "-m", "test commit"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // Working tree should be clean afterwards.
    assert!(
        git_stdout(&clone, &["status", "--porcelain"]).is_empty(),
        "expected clean working tree after commit"
    );
    // The commit has our message and includes the untracked file (git add -A).
    assert_eq!(
        git_stdout(&clone, &["log", "-1", "--pretty=%s"]),
        "test commit"
    );
    let files = git_stdout(&clone, &["show", "--name-only", "--pretty=format:", "HEAD"]);
    assert!(
        files.contains("new.txt"),
        "expected new.txt in commit: {}",
        files
    );
}

#[test]
fn normal_002_skips_a_clean_checkout() {
    let (env, _bare, clone) = setup_commit_env("commit_clean");
    let before = git_stdout(&clone, &["rev-parse", "HEAD"]);

    let out = env.run(&["commit", "-m", "nothing to do"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    assert!(
        plain.contains("clean") || plain.contains("skip"),
        "expected a clean/skip message: {}",
        plain
    );
    assert!(plain.contains("skip  libs/mylib (clean)"), "{}", plain);
    assert_eq!(
        git_stdout(&clone, &["rev-parse", "HEAD"]),
        before,
        "no empty commit"
    );
}

/// Push sends each checkout's topic branch to its remote under the topic's
/// name, as its upstream — the root's included.
#[test]
fn normal_003_then_push_reaches_the_remote() {
    let (env, bare, clone) = setup_commit_env("commit_then_push");

    std::fs::write(clone.join("README.md"), "# v2\n").unwrap();
    let out = env.run(&["commit", "-m", "propagated"]);
    assert!(out.success, "commit stderr: {}", out.stderr);

    let out = env.run(&["push"]);
    assert!(out.success, "push stderr: {}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("libs/mylib → feat/x"), "{}", out.stdout);

    // Name the git dir explicitly: discovery-based access to a bare repo is
    // refused when the developer has `safe.bareRepository = explicit` set.
    let bare_str = bare.to_str().unwrap();
    assert_eq!(
        git_stdout(
            &bare,
            &["--git-dir", bare_str, "log", "-1", "--pretty=%s", "feat/x"]
        ),
        "propagated"
    );
    assert_eq!(
        git_stdout(&clone, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/feat/x"
    );
    // The root's topic branch went to the root's own remote too.
    let root_remote = env.repos_remote.join("root.git");
    assert_eq!(
        git_stdout(
            &root_remote,
            &[
                "--git-dir",
                root_remote.to_str().unwrap(),
                "rev-parse",
                "feat/x"
            ]
        ),
        git_stdout(&env.playground, &["rev-parse", "HEAD"])
    );
}

/// A checkout off the topic is at its pin: commit never makes a commit on a
/// detached HEAD, and says how to bring it in.
#[test]
fn normal_004_skips_a_checkout_off_the_topic_and_says_how_to_bring_it_in() {
    let env = TestEnv::new("commit_off_topic");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# v1\n")]);
    env.write_config(&format!(
        "[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    env.init_playground_git();
    assert!(env.run(&["pull"]).success);
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let clone = env.playground.join("libs/mylib");
    support::edit(&clone.join("README.md"), "# edited\n");

    let out = env.run(&["commit", "-m", "nope"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains("libs/mylib (not on topic feat/x; run gitscale develop libs/mylib)"),
        "{}",
        out.stdout
    );
    assert!(!git_stdout(&clone, &["status", "--porcelain"]).is_empty());
}

/// With no names the root is committed too, reported as `.`; given names,
/// only those checkouts are, and the root is left as it was.
#[test]
fn normal_007_commits_the_root_only_when_no_names_are_given() {
    let (env, _bare, clone) = setup_commit_env("commit_root");
    std::fs::write(env.playground.join(".gitignore"), "/libs/\n").unwrap();
    std::fs::write(env.playground.join("root.txt"), "root change").unwrap();
    std::fs::write(clone.join("child.txt"), "child change").unwrap();
    let root_before = git_stdout(&env.playground, &["rev-parse", "HEAD"]);

    let out = env.run(&["commit", "-m", "child only", "libs/mylib"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("workspace root"), "{}", out.stdout);
    assert_eq!(
        git_stdout(&clone, &["log", "-1", "--pretty=%s"]),
        "child only"
    );
    assert_eq!(
        git_stdout(&env.playground, &["rev-parse", "HEAD"]),
        root_before
    );

    let out = env.run(&["commit", "-m", "everything"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("ok    . (workspace root)"),
        "{}",
        out.stdout
    );
    assert_eq!(
        git_stdout(&env.playground, &["log", "-1", "--pretty=%s"]),
        "everything"
    );
    let files = git_stdout(
        &env.playground,
        &["show", "--name-only", "--pretty=format:", "HEAD"],
    );
    assert!(files.contains("root.txt"), "{}", files);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn edge_005_does_not_reach_the_workspace_through_an_empty_directory() {
    let env = workspace_with_stray_directory("commit_empty_entry_dir", |_| {});
    std::fs::write(env.playground.join("staged.txt"), "x").unwrap();
    git_out(&env.playground, &["add", "staged.txt"]);
    let before = git_out(&env.playground, &["rev-parse", "HEAD"]);

    let out = env.run(&["commit", "libs/core", "-m", "meant for core"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("not cloned"), "stdout: {}", out.stdout);
    assert_eq!(
        git_out(&env.playground, &["rev-parse", "HEAD"]),
        before,
        "a commit meant for the entry must not land in the workspace"
    );
}

/// The dependency links gitscale plants in a checkout are not anyone's work —
/// status does not call them dirty — so committing a developed checkout
/// commits its real changes and never the planted links.
#[test]
#[ignore = "bug: commit runs `git add -A` on a topic checkout, committing the dependency links gitscale planted in it"]
fn edge_008_does_not_commit_the_links_gitscale_planted() {
    let env = TestEnv::new("commit_planted_links");
    let (b, d) = diamond(&env);
    let root_remote = env.create_bare_repo("root", "main", &[("README.md", "root")]);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    env.init_playground_git();
    env.set_playground_origin(root_remote.to_str().unwrap());
    assert!(env.run(&["pull"]).success);
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let out = env.run(&["develop", "imports/b"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let checkout = env.playground.join("imports/b");
    assert!(checkout.join("libs/d").is_symlink(), "the link is planted");
    run_git_pub(&checkout, &["config", "user.email", "t@t.com"]);
    run_git_pub(&checkout, &["config", "user.name", "T"]);
    support::edit(&checkout.join("README.md"), "real work");

    let out = env.run(&["commit", "-m", "work", "imports/b"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let files = git_stdout(
        &checkout,
        &["show", "--name-only", "--pretty=format:", "HEAD"],
    );
    assert!(files.contains("README.md"), "{}", files);
    assert!(
        !files.contains("libs/d"),
        "the planted link was committed: {}",
        files
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_006_refuses_an_empty_message() {
    let (env, _bare, clone) = setup_commit_env("commit_empty_msg");
    std::fs::write(clone.join("new.txt"), "new").unwrap();

    let before = git_stdout(&clone, &["rev-parse", "HEAD"]);
    let root_before = git_stdout(&env.playground, &["rev-parse", "HEAD"]);

    for message in ["", "   ", "\n\t"] {
        let out = env.run(&["commit", "-m", message]);
        assert!(!out.success, "expected failure on empty commit message");
        assert!(
            out.stderr.contains("commit message must not be empty"),
            "{:?}: {}",
            message,
            out.stderr
        );
    }
    assert_eq!(git_stdout(&clone, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        git_stdout(&env.playground, &["rev-parse", "HEAD"]),
        root_before
    );
    assert!(clone.join("new.txt").is_file());
}

/// A checkout whose commit fails (here a pre-commit hook refuses it) fails
/// the command, after everything else is done: the root is still committed,
/// and the exit status says how many failed. Pinned as it is: the docs do not
/// say whether one failure should hold the root back.
#[test]
fn error_009_a_failing_checkout_commit_fails_the_command_after_the_rest() {
    use std::os::unix::fs::PermissionsExt;
    let (env, _bare, clone) = setup_commit_env("commit_child_fails");
    std::fs::write(env.playground.join(".gitignore"), "/libs/\n").unwrap();
    let hooks = env.repos_remote.join("refusing-hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho refused by hook >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    run_git_pub(
        &clone,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
    std::fs::write(clone.join("child.txt"), "child change").unwrap();
    let child_before = git_stdout(&clone, &["rev-parse", "HEAD"]);

    let out = env.run(&["commit", "-m", "both"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("FAIL  libs/mylib"), "{}", out.stderr);
    assert!(
        out.stderr.contains("1 repo(s) failed to commit"),
        "{}",
        out.stderr
    );
    assert_eq!(git_stdout(&clone, &["rev-parse", "HEAD"]), child_before);
    assert!(
        clone.join("child.txt").is_file(),
        "the change is still there"
    );
    assert!(
        out.stdout.contains("ok    . (workspace root)"),
        "{}",
        out.stdout
    );
    assert_eq!(
        git_stdout(&env.playground, &["log", "-1", "--pretty=%s"]),
        "both"
    );
}
