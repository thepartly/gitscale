//! `gitscale push`: which checkouts are pushed, and which are left alone.

use crate::support;
use crate::support::{git_stdout, run_git_pub, TestEnv};

/// A workspace on topic `feat/x` with `libs/a` and `libs/b` both developed on
/// it and pushed once, so each has `origin/feat/x` as its upstream. Returns
/// the bare remotes of a and b.
fn two_developed_children(name: &str) -> (TestEnv, std::path::PathBuf, std::path::PathBuf) {
    let env = TestEnv::new(name);
    let a = env.create_bare_repo("a", "main", &[("a.txt", "a")]);
    let b = env.create_bare_repo("b", "main", &[("b.txt", "b")]);
    let root_remote = env.create_bare_repo("root", "main", &[("README.md", "root")]);
    env.write_config(&format!(
        "[repos]\n\"libs/a\" = {{ url = \"{}\", revision = \"main\" }}\n\
         \"libs/b\" = {{ url = \"{}\", revision = \"main\" }}\n",
        a.display(),
        b.display()
    ));
    env.init_playground_git();
    env.set_playground_origin(root_remote.to_str().unwrap());
    assert!(env.run(&["pull"]).success);
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let out = env.run(&["develop", "libs/a", "libs/b"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    for dir in ["libs/a", "libs/b"] {
        commit_in(&env.playground.join(dir), "first.txt");
    }
    let out = env.run(&["push"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    (env, a, b)
}

/// A commit in the checkout at `dir`, adding `file`.
fn commit_in(dir: &std::path::Path, file: &str) {
    std::fs::write(dir.join(file), file).unwrap();
    run_git_pub(dir, &["add", file]);
    run_git_pub(
        dir,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            file,
        ],
    );
}

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// Off a topic every checkout sits at its pin: nothing of gitscale's to push.
#[test]
fn normal_001_skips_checkouts_off_the_topic() {
    let env = TestEnv::new("push_off_topic");
    let bare = env.create_bare_repo("rolib", "main", &[("data.txt", "hello\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/rolib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    env.run(&["pull"]);
    let out = env.run(&["push"]);
    assert!(out.success);
    insta::assert_snapshot!("push_off_topic_stdout", out.stdout);
}

#[test]
fn normal_002_skips_an_artefact() {
    let env = TestEnv::new("push_skip_artefact");
    let bare = env.artefact_repo("app", &[("app.bin", "content")]);

    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "main", artefact = "replace" }}
"#,
        env.registries(),
        bare.display(),
    ));

    assert!(env.run(&["pull"]).success);
    let out = env.run(&["push"]);
    assert!(out.success);
    insta::assert_snapshot!("push_skip_artefact_stdout", out.stdout);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A checkout pinned to a tag is detached. There is no branch to push, and
/// `git push` failing on that used to fail every sync.
#[test]
fn edge_003_skips_a_detached_tag_pin() {
    let env = TestEnv::new("push_skip_detached");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    support::run_git_pub(&bare, &["tag", "demo-v1", "main"]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "demo-v1" }}
"#,
        bare.display()
    ));
    assert!(env.run(&["pull"]).success);

    let out = env.run(&["sync"]);
    assert!(
        out.success,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    assert!(
        out.stdout.contains("libs/mylib (not on the topic)"),
        "{}",
        out.stdout
    );
}

/// A checkout whose remote topic branch has moved on while it has nothing new
/// of its own has nothing to push: it is skipped as up to date, not pushed
/// backwards and refused.
#[test]
#[ignore = "bug: push_topic counts only HEAD == origin/<topic> as up to date, so a checkout merely behind is pushed and rejected"]
fn edge_004_skips_a_topic_checkout_that_is_only_behind_its_remote() {
    let (env, a, _b) = two_developed_children("push_only_behind");
    env.push_commit(&a, "feat/x", "theirs.txt", "a teammate's change");
    assert!(env.run(&["fetch"]).success);

    let out = env.run(&["push"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("skip  libs/a (up to date)"),
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// A push the remote refuses — someone else pushed to the topic first — fails
/// that checkout, while every other checkout is still pushed, and the command
/// exits non-zero with the count.
#[test]
fn error_005a_a_rejected_push_fails_that_checkout_and_the_rest_are_pushed() {
    let (env, a, b) = two_developed_children("push_refused");
    env.push_commit(&a, "feat/x", "theirs.txt", "a teammate's change");
    commit_in(&env.playground.join("libs/a"), "mine.txt");
    commit_in(&env.playground.join("libs/b"), "second.txt");
    let b_head = git_stdout(&env.playground.join("libs/b"), &["rev-parse", "HEAD"]);

    let out = env.run(&["push"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("FAIL  libs/a"), "{}", out.stderr);
    assert!(
        out.stdout.contains("ok    libs/b → feat/x"),
        "{}",
        out.stdout
    );
    assert!(
        out.stderr.contains("1 repo(s) failed to push"),
        "{}",
        out.stderr
    );
    let b_remote = git_stdout(
        &b,
        &["--git-dir", b.to_str().unwrap(), "rev-parse", "feat/x"],
    );
    assert_eq!(b_remote, b_head);
}

/// The same refusal says why: the remote rejected it, because it has commits
/// the checkout does not — not just that "some refs" failed.
#[test]
#[ignore = "bug: the FAIL line is git's 'error: failed to push some refs to ...', with the rejection and its reason dropped"]
fn error_005b_a_rejected_push_says_it_was_rejected() {
    let (env, a, _b) = two_developed_children("push_refused_reason");
    env.push_commit(&a, "feat/x", "theirs.txt", "a teammate's change");
    commit_in(&env.playground.join("libs/a"), "mine.txt");

    let out = env.run(&["push"]);
    assert!(!out.success, "{}", out.stdout);
    let line = out
        .stderr
        .lines()
        .find(|l| l.contains("FAIL  libs/a"))
        .unwrap_or_default()
        .to_string();
    assert!(line.contains("rejected"), "{}", out.stderr);
}
