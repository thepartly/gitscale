//! `gitscale fetch`: filling the stores without moving any checkout.

use crate::support::resolution::*;
use crate::support::workspace::*;
use crate::support::{git_stdout, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// A fetch fills the root's store whether or not the checkout exists yet:
/// the next pull has nothing left to download.
#[test]
fn normal_001_fills_the_store_whether_or_not_the_checkout_exists() {
    let env = TestEnv::new("fetch_git_not_cloned");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run(&["fetch"]);
    assert!(out.success);
    insta::assert_snapshot!("fetch_git_not_cloned_stdout", out.stdout);
    assert!(env
        .store(&bare.display().to_string())
        .join("HEAD")
        .is_file());
    assert!(!env.playground.join("libs/mylib").exists());
}

/// With the checkout already there, a fetch brings its store up to date and
/// leaves the checkout exactly where it is: only `pull` moves checkouts.
#[test]
fn normal_002_refreshes_the_store_of_a_checkout_and_leaves_the_checkout() {
    let env = TestEnv::new("fetch_git_cloned");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    env.run(&["pull"]);
    let out = env.run(&["fetch"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("fetch_git_cloned_stdout", out.stdout);

    let checkout = env.playground.join("libs/mylib");
    let head = git_stdout(&checkout, &["rev-parse", "HEAD"]);
    let tip = env.push_commit(&bare, "main", "a.txt", "b");
    let out = env.run(&["fetch"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let store = env.store(&bare.display().to_string());
    assert_eq!(
        git_stdout(
            &store,
            &[
                "--git-dir",
                store.to_str().unwrap(),
                "rev-parse",
                "refs/remotes/origin/main"
            ]
        ),
        tip,
        "the store has the remote's new commit"
    );
    assert_eq!(git_stdout(&checkout, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "a"
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A graph that does not resolve still gets its declared entries fetched,
/// and the fetch fails afterwards with the reason.
#[test]
fn edge_003_fetches_the_declared_entries_when_resolution_fails() {
    let env = TestEnv::new("res_fetch_fallback");
    let d = tagged(&env, "d", &[("v1.5.0", "")]);
    // main moves on past the tag, so the two requests name different commits.
    env.push_commit(&d, "main", "VERSION", "next");
    let b = tagged(
        &env,
        "b",
        &[("v1.0.0", &repos(&[("libs/d", &d, ", revision = \"main\"")]))],
    );
    let c = tagged(
        &env,
        "c",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.5.0\"")]),
        )],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")])
    ));
    assert!(env.run(&["pull"]).success);
    let fresh = env.push_commit(&b, "main", "news.txt", "new");

    // c's request for d cannot be ordered against b's: neither is above.
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ""),
    ]));
    let out = env.run(&["fetch"]);
    assert!(!out.success);
    assert!(out.stderr.contains("cannot order"), "{}", out.stderr);
    assert_eq!(
        git_stdout(
            &env.playground.join("imports/b"),
            &["rev-parse", "origin/main"]
        ),
        fresh,
        "the declared entry was fetched anyway"
    );
}

/// Inside a child repository a dependency is the enclosing workspace's link;
/// that checkout is the outer root's to fetch, and is skipped here.
#[test]
fn edge_005_skips_an_enclosing_workspaces_link() {
    let env = TestEnv::new("fetch_outer_link");
    let (child, _) = child_with_outer_link(&env);
    let out = run_in(&child, "fetch", &[]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("skip  libs/b (symlink)"),
        "{}",
        out.stdout
    );
    assert!(child.join("libs/b").is_symlink());
}

/// CI keeps no history to fetch into: git entries are skipped with that
/// reason, and the next pull takes the commit it needs.
#[test]
fn edge_006_in_ci_git_entries_are_skipped() {
    let env = TestEnv::new("fetch_in_ci");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    let out = env.run_with_env(&[("CI", "true")], &["fetch"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("skip  libs/mylib (no history in CI)"),
        "{}",
        out.stdout
    );
    assert!(!env.playground.join("libs/mylib").exists());
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// When an entry fails to fetch and resolution fails too, both are said.
#[test]
fn error_004_reports_both_failures() {
    let env = TestEnv::new("res_fetch_both");
    let d = tagged(&env, "d", &[("v1.5.0", "")]);
    env.push_commit(&d, "main", "VERSION", "next");
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"main\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    let gone = tagged(&env, "gone", &[("v1.0.0", "")]);
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/gone", &gone, ", revision = \"v1.0.0\""),
        ])
    ));
    assert!(env.run(&["pull"]).success);
    std::fs::remove_dir_all(&gone).unwrap();
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ""),
        ("imports/gone", &gone, ", revision = \"v1.0.0\""),
    ]));
    let out = env.run(&["fetch"]);
    assert!(!out.success);
    assert!(out.stderr.contains("failed to fetch"), "{}", out.stderr);
    assert!(out.stderr.contains("cannot resolve"), "{}", out.stderr);
}
