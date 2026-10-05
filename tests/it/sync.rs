//! `git scale sync`: placement with its tidying — relinking, and removing
//! checkouts nothing asks for any more, but never one holding work. It never
//! pushes.

use crate::support::resolution::*;
use crate::support::workspace::*;
use crate::support::{git_stdout, run_git_pub, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_places_a_fresh_workspace() {
    let env = TestEnv::new("sync_full");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# mylib\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("sync_full_stdout", out.stdout);

    assert!(env.playground.join("libs/mylib/README.md").is_file());
}

/// An implicit checkout nothing asks for any more is removed by `sync` when
/// that loses nothing; a clone made by hand under the hoist directory never.
#[test]
fn normal_002_removes_an_implicit_checkout_left_behind() {
    let env = TestEnv::new("res_left_behind");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = tagged(
        &env,
        "b",
        &[
            (
                "v1.0.0",
                &repos(&[("libs/d", &d, ", revision = \"v1.0.0\"")]),
            ),
            ("v1.1.0", "[repos]\n"),
        ],
    );
    let config = |rev: &str| {
        format!(
            "{}{}",
            allow(&env),
            repos(&[("imports/b", &b, &format!(", revision = \"{}\"", rev))])
        )
    };
    env.write_config(&config("v1.0.0"));
    assert!(env.run(&["sync"]).success);
    assert!(env.playground.join("imports/d").is_dir());
    // Someone's own clone, which gitscale did not make.
    run_git_pub(
        &env.playground,
        &["clone", "--quiet", d.to_str().unwrap(), "imports/mine"],
    );

    env.write_config(&config("v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("remove  imports/d (no longer needed)"),
        "{}",
        out.stdout
    );
    assert!(!env.playground.join("imports/d").exists());
    assert!(env.playground.join("imports/mine").is_dir());
}

/// An implicit artefact nothing asks for any more is removed too, with what
/// gitscale recorded about installing it.
#[test]
fn normal_003_removes_a_left_behind_implicit_artefact() {
    let env = TestEnv::new("res_left_behind_artefact");
    let art = env.artefact_repo("art", &[("app.bin", "x")]);
    run_git_pub(&art, &["tag", "v1.0.0", "main"]);
    let b = tagged(
        &env,
        "b",
        &[
            (
                "v1.0.0",
                &repos(&[(
                    "libs/art",
                    &art,
                    ", revision = \"v1.0.0\", artefact = \"replace\"",
                )]),
            ),
            ("v1.1.0", "[repos]\n"),
        ],
    );
    env.init_playground_git();
    let config = |rev: &str| {
        format!(
            "{}{}{}",
            env.registries(),
            allow(&env),
            repos(&[("imports/b", &b, &format!(", revision = \"{}\"", rev))])
        )
    };
    env.write_config(&config("v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/art/dist/app.bin").is_file());
    let records = env.playground.join(".git/gitscale/artefacts");
    assert!(std::fs::read_dir(&records).unwrap().count() > 0);

    env.write_config(&config("v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("remove  imports/art (no longer needed)"),
        "{}",
        out.stdout
    );
    assert!(!env.playground.join("imports/art").exists());
    assert_eq!(std::fs::read_dir(&records).unwrap().count(), 0);
}

/// An entry removed from the root config goes with the next sync, as long as
/// that loses nothing; renamed, the old checkout goes and the new one comes.
#[test]
fn normal_004_removes_a_checkout_whose_entry_was_removed() {
    let env = TestEnv::new("res_removed_entry");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let e = tagged(&env, "e", &[("v1.0.0", "")]);
    env.write_config(&repos(&[
        ("imports/d", &d, ", revision = \"v1.0.0\""),
        ("imports/e", &e, ", revision = \"v1.0.0\""),
    ]));
    assert!(env.run(&["sync"]).success);

    // Removed, and renamed.
    env.write_config(&repos(&[("libs/e", &e, ", revision = \"v1.0.0\"")]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("remove  imports/d (no longer needed)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("remove  imports/e (no longer needed)"),
        "{}",
        out.stdout
    );
    assert!(!env.playground.join("imports/d").exists());
    assert!(!env.playground.join("imports/e").exists());
    assert_eq!(head(&env, "libs/e"), tag_commit(&e, "v1.0.0"));
}

/// `sync` takes the name of an implicit checkout, as it does a declared one.
#[test]
fn normal_005_takes_an_implicit_checkout_by_name() {
    let env = TestEnv::new("res_sync_implicit_name");
    implicit_d(&env);
    assert!(env.run(&["sync"]).success);
    let out = env.run(&["sync", "imports/d"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A checkout nothing needs, whose only change is the links gitscale planted
/// in it, holds nothing to lose and goes.
#[test]
fn edge_006_removes_a_checkout_whose_only_change_is_its_links() {
    let env = TestEnv::new("res_remove_with_links");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["sync"]).success);
    assert!(env.playground.join("imports/b/libs/d").is_symlink());

    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.5.0\"")]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!env.playground.join("imports/b").exists(), "{}", out.stdout);
}

/// A left-behind implicit checkout holding work is kept, and the sync fails
/// at the end saying so; `--force` removes it.
#[test]
fn edge_007_keeps_a_left_behind_checkout_with_work_and_fails() {
    let env = TestEnv::new("res_left_behind_work");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = tagged(
        &env,
        "b",
        &[
            (
                "v1.0.0",
                &repos(&[("libs/d", &d, ", revision = \"v1.0.0\"")]),
            ),
            ("v1.1.0", "[repos]\n"),
        ],
    );
    let config = |rev: &str| {
        format!(
            "{}{}",
            allow(&env),
            repos(&[("imports/b", &b, &format!(", revision = \"{}\"", rev))])
        )
    };
    env.write_config(&config("v1.0.0"));
    assert!(env.run(&["sync"]).success);
    std::fs::write(env.playground.join("imports/d/notes.txt"), "mine").unwrap();

    env.write_config(&config("v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("skip  imports/d (no longer needed, but modified"),
        "{}",
        out.stdout
    );
    assert!(out.stderr.contains("use --force"), "{}", out.stderr);
    assert!(env.playground.join("imports/d/notes.txt").is_file());

    let out = env.run(&["sync", "--force"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!env.playground.join("imports/d").exists());
}

/// A recorded checkout nothing needs is still kept while it holds a
/// checkout that is wanted.
#[test]
fn edge_008_never_removes_a_directory_holding_a_wanted_checkout() {
    let env = TestEnv::new("res_ledger_nested");
    let outer = tagged(&env, "outer", &[("v1.0.0", "")]);
    let inner = tagged(&env, "inner", &[("v1.0.0", "")]);
    env.write_config(&repos(&[
        ("deps", &outer, ", revision = \"v1.0.0\""),
        ("deps/inner", &inner, ", revision = \"v1.0.0\""),
    ]));
    assert!(env.run(&["sync"]).success);

    env.write_config(&repos(&[("deps/inner", &inner, ", revision = \"v1.0.0\"")]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("remove  deps "), "{}", out.stdout);
    assert!(env.playground.join("deps/inner/VERSION").is_file());
}

/// `sync` removes only checkouts gitscale made. A clone the user made at an
/// entry's path — which `sync` refused to touch — is not gitscale's, and
/// stays when the entry is removed.
#[test]
#[ignore = "bug: sync records any directory with a .git at an entry path as its own, so a later sync deletes the user's clone"]
fn edge_009_never_removes_a_clone_the_user_made_at_a_removed_entry() {
    let env = TestEnv::new("sync_foreign_clone");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "a")]);
    let clone = env.playground.join("libs/lib");
    run_git_pub(
        &env.playground,
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("not a gitscale worktree"),
        "{}",
        out.stderr
    );

    env.write_config("[repos]\n");
    let out = env.run(&["sync"]);
    assert!(
        clone.join(".git").is_dir() && clone.join("a.txt").is_file(),
        "the user's clone was deleted: {}",
        out.stdout
    );
    assert!(!out.stdout.contains("remove  libs/lib"), "{}", out.stdout);
}

/// However a config names the workspace itself — an entry at `.` — no later
/// `sync` deletes the workspace. Here the root is clean and pushed, the case
/// in which a checkout nothing needs would be removed.
#[test]
#[ignore = "bug: an entry '.' is accepted and placement records the root as a checkout; once the entry goes, sync deletes the whole workspace, .git included"]
fn edge_010_never_deletes_the_workspace_after_a_dot_entry_is_removed() {
    let env = TestEnv::new("sync_dot_entry");
    let other = env.create_bare_repo("other", "main", &[("o.txt", "o")]);
    run_git_pub(&env.repos_remote, &["init", "-q", "--bare", "root.git"]);
    let root_remote = env.repos_remote.join("root.git");
    std::fs::write(env.playground.join(".gitignore"), "/libs/\n").unwrap();
    env.write_config(&format!(
        "[repos]\n\".\" = {{ url = \"{}\", revision = \"main\" }}\n",
        other.display()
    ));
    env.init_playground_git();
    let commit_all = |message: &str| {
        run_git_pub(&env.playground, &["add", "-A"]);
        run_git_pub(&env.playground, &["commit", "-q", "-m", message]);
        run_git_pub(&env.playground, &["push", "-q", "-u", "origin", "main"]);
    };
    env.set_playground_origin(root_remote.to_str().unwrap());
    commit_all("dot entry");
    let _ = env.run(&["sync"]);

    env.write_config("[repos]\n");
    commit_all("no entries");
    let out = env.run(&["sync"]);
    assert!(
        env.playground.join(".git").is_dir() && env.playground.join(".gitscale.toml").is_file(),
        "the workspace was deleted: {}{}",
        out.stdout,
        out.stderr
    );
    assert!(!out.stdout.contains("remove  ."), "{}", out.stdout);
}

/// Relink refusing an unlinked checkout with work fails the sync with its
/// reason, after the checkouts are placed — and a sync pushes nothing, then
/// or ever.
#[test]
fn edge_011_a_relink_refusal_fails_the_sync_after_placing_and_nothing_is_pushed() {
    let (env, link) = setup_unlinked_env("sync_push_after_refusal");
    std::fs::write(link.join("dirty.txt"), "local change").unwrap();

    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    let placed = out.stdout.find("ok    repoB").expect(&out.stdout);
    let relinked = out
        .stdout
        .find("skip  repoA/libs/b (modified")
        .expect(&out.stdout);
    assert!(placed < relinked, "{}", out.stdout);
    assert!(!out.stdout.contains("Pushing"), "{}", out.stdout);
    assert!(
        out.stderr.contains("use --force to override"),
        "{}",
        out.stderr
    );
}

/// A checkout nothing needs any more is kept while it holds commits no remote
/// has, even with a clean working tree.
#[test]
fn edge_012_keeps_a_left_behind_checkout_with_unpushed_commits() {
    let env = TestEnv::new("sync_left_behind_commit");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let e = tagged(&env, "e", &[("v1.0.0", "")]);
    env.write_config(&repos(&[
        ("imports/d", &d, ", revision = \"v1.0.0\""),
        ("imports/e", &e, ", revision = \"v1.0.0\""),
    ]));
    assert!(env.run(&["sync"]).success);
    let dest = env.playground.join("imports/d");
    std::fs::write(dest.join("mine.txt"), "mine").unwrap();
    run_git_pub(&dest, &["add", "mine.txt"]);
    run_git_pub(
        &dest,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "mine",
        ],
    );
    let mine = git_stdout(&dest, &["rev-parse", "HEAD"]);

    env.write_config(&repos(&[("imports/e", &e, ", revision = \"v1.0.0\"")]));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("skip  imports/d (no longer needed, but modified"),
        "{}",
        out.stdout
    );
    assert_eq!(git_stdout(&dest, &["rev-parse", "HEAD"]), mine);
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

/// However often a sync's steps ask for a store, each store is fetched once
/// per sync.
#[test]
fn perf_013_fetches_each_store_once_per_sync() {
    let env = TestEnv::new("sync_fetch_count");
    let mut config = String::from("[repos]\n");
    for name in ["a", "b", "c", "d", "e"] {
        let bare = env.create_bare_repo(name, "main", &[("f.txt", name)]);
        config.push_str(&format!(
            "\"libs/{}\" = {{ url = \"{}\", revision = \"main\" }}\n",
            name,
            bare.display()
        ));
    }
    env.write_config(&config);
    let (path, log) = counting_git(&env);
    for round in ["first", "second"] {
        let out = env.run_with_env(&[("PATH", &path)], &["sync"]);
        assert!(out.success, "{}: {}{}", round, out.stdout, out.stderr);
        assert_eq!(take_git_count(&log, "fetch --prune"), 5, "{} sync", round);
        assert_eq!(
            take_git_count(&log, "push"),
            0,
            "{} sync: nothing to push",
            round
        );
    }
}
