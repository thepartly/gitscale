//! Stores: every dependency is a worktree of the root's own store for it —
//! where the stores live in each layout, and keeping their worktrees sound.

use crate::support::worktrees::*;
use crate::support::{git_stdout, run_git_pub, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// A plain clone: the stores live in its own `.git/gitscale`, and the child
/// is a worktree of one, detached at the pin.
#[test]
fn normal_001_a_plain_clone_keeps_its_stores_in_its_own_git_directory() {
    let f = fixture("wt_plain", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert_eq!(branch(&child), None);
    assert!(!writable(&child.join("lib.txt")));
    assert!(common_dir(&child).starts_with(ws.join(".git/gitscale/repos").canonicalize().unwrap()));
}

/// A bare root with worktrees, Angel's layout: every root worktree's children
/// are worktrees of the one store in the bare repository, so a second root
/// worktree downloads nothing and sees the first one's branches.
#[test]
fn normal_002_a_bare_root_shares_one_store_across_its_worktrees() {
    let f = fixture("wt_bare", "");
    let repo = f.env.repos_remote.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    run_git_pub(
        &repo,
        &["clone", "-q", "--bare", f.root.to_str().unwrap(), ".bare"],
    );
    std::fs::write(repo.join(".git"), "gitdir: ./.bare\n").unwrap();
    run_git_pub(
        &repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    run_git_pub(&repo, &["fetch", "-q"]);
    run_git_pub(&repo, &["worktree", "add", "-q", "main", "main"]);
    let main = repo.join("main");
    identity(&main);
    ok(&gs(&main, &["sync"]));

    let store_root = repo.join(".bare/gitscale/repos");
    assert!(
        store_root.is_dir(),
        "the stores live in the bare repository"
    );
    let child = main.join("imports/core");
    assert!(common_dir(&child).starts_with(store_root.canonicalize().unwrap()));

    // A second root worktree: its child is a worktree of the same store.
    run_git_pub(&repo, &["worktree", "add", "-q", "-b", "feat/x", "feature"]);
    let feature = repo.join("feature");
    ok(&gs(&feature, &["sync"]));
    let other = feature.join("imports/core");
    assert_eq!(common_dir(&other), common_dir(&child));
    assert_eq!(std::fs::read_dir(&store_root).unwrap().count(), 1);

    // A topic branch joined in one worktree is visible from the other's child.
    ok(&gs(&feature, &["topic", "join", "imports/core"]));
    identity(&other);
    std::fs::write(other.join("lib.txt"), "work").unwrap();
    run_git_pub(&other, &["commit", "-q", "-am", "work"]);
    assert_eq!(git(&child, &["rev-parse", "feat/x"]), head(&other));
}

/// The store keeps the remote's branches apart from its own: a fetch never
/// touches the branches joined checkouts are on, and no other refs come.
#[test]
fn normal_003_a_store_maps_the_remote_branches_to_remote_tracking_refs() {
    let env = TestEnv::new("store_refspec");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    let url = bare.display().to_string();
    git_stdout(&bare, &["update-ref", "refs/merge-requests/1/head", "main"]);
    env.write_config(&format!(
        "[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        url
    ));
    assert!(env.run(&["sync"]).success);

    let store = env.store(&url);
    let refs = git_stdout(&store, &["for-each-ref", "--format=%(refname)"]);
    assert!(refs.contains("refs/remotes/origin/main"), "{}", refs);
    assert!(!refs.contains("refs/heads/main"), "{}", refs);
    assert!(!refs.contains("merge-requests"), "{}", refs);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// `git clone --bare` sets no fetch refspec; `ls` says what that costs.
#[test]
fn edge_004_ls_warns_about_a_root_without_a_fetch_refspec() {
    let f = fixture("wt_refspec", "");
    let repo = f.env.repos_remote.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    run_git_pub(
        &repo,
        &["clone", "-q", "--bare", f.root.to_str().unwrap(), ".bare"],
    );
    std::fs::write(repo.join(".git"), "gitdir: ./.bare\n").unwrap();
    run_git_pub(&repo, &["worktree", "add", "-q", "main", "main"]);
    let main = repo.join("main");
    ok(&gs(&main, &["sync"]));
    let out = gs(&main, &["ls"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("warning: the root repository has no fetch refspec"),
        "{}",
        out.stdout
    );
    run_git_pub(
        &repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    let out = gs(&main, &["ls"]);
    assert!(!out.stdout.contains("warning:"), "{}", out.stdout);
}

/// A root worktree deleted with its children in it leaves entries in the
/// stores that keep its branches locked; the next command prunes them.
#[test]
fn edge_005_a_deleted_root_worktree_does_not_lock_its_topic() {
    let f = fixture("wt_prune", "");
    let ws = f.clone_root("ws");
    identity(&ws);
    run_git_pub(&ws, &["add", "."]);
    let other = f.env.repos_remote.join("other");
    run_git_pub(
        &ws,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/x",
            other.to_str().unwrap(),
        ],
    );
    ok(&gs(&other, &["sync"]));
    ok(&gs(&other, &["topic", "join", "imports/core"]));
    let work = commit_in(&other.join("imports/core"), "lib.txt", "work");
    std::fs::remove_dir_all(&other).unwrap();
    run_git_pub(&ws, &["worktree", "prune"]);

    // The same topic, in the main worktree: its branch must not be held by
    // the deleted one.
    run_git_pub(&ws, &["switch", "-q", "feat/x"]);
    let out = gs(&ws, &["sync"]);
    ok(&out);
    assert_eq!(branch(&ws.join("imports/core")).as_deref(), Some("feat/x"));
    // The work committed in the deleted worktree's checkout is in the store,
    // and on the branch.
    assert_eq!(head(&ws.join("imports/core")), work);
}

/// Moving a plain-clone root breaks its children's links to the store, which
/// is inside it; sync repairs them.
#[test]
fn edge_006_sync_repairs_children_after_the_root_moves() {
    let f = fixture("wt_moved", "");
    let ws = f.clone_root("ws");
    // Absolute links whatever git this is: git 2.48 and later write relative
    // ones, which a move does not break, and the repair would go untested.
    let store = store_for(&ws, &f.core);
    run_git_pub(&store, &["config", "worktree.useRelativePaths", "false"]);
    let child_before = ws.join("imports/core");
    run_git_pub(
        &store,
        &["worktree", "repair", child_before.to_str().unwrap()],
    );
    let link = std::fs::read_to_string(child_before.join(".git")).unwrap();
    assert!(link.starts_with("gitdir: /"), "{}", link);

    let moved = f.env.repos_remote.join("moved");
    std::fs::rename(&ws, &moved).unwrap();
    let out = gs(&moved, &["sync"]);
    ok(&out);
    let child = moved.join("imports/core");
    let link = std::fs::read_to_string(child.join(".git")).unwrap();
    assert!(
        link.contains(&format!("{}/.git/gitscale/repos/", moved.display())),
        "the child's link names the moved store: {}",
        link
    );
    assert!(is_worktree_listed(&store_for(&moved, &f.core), &child));
    assert!(
        git_ok(&child, &["status"]),
        "the child reads its history again"
    );
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
}

/// A checkout gitscale did not make — a clone of its own, from an older
/// gitscale or by hand — is reported and left alone.
#[test]
fn edge_007_a_foreign_checkout_is_reported_and_left_alone() {
    let f = fixture("wt_foreign", "");
    let ws = f.env.repos_remote.join("ws");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            ws.to_str().unwrap(),
        ],
    );
    run_git_pub(
        &ws,
        &["clone", "-q", f.core.to_str().unwrap(), "imports/core"],
    );
    let out = gs(&ws, &["sync"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "not a gitscale worktree; move your changes out, delete it and run git scale sync"
        ),
        "{}",
        out.stderr
    );
    assert!(ws.join("imports/core/.git").is_dir());
    assert!(
        status_row(&ws, "imports/core").contains("foreign"),
        "{}",
        status_row(&ws, "imports/core")
    );
}

/// A store deleted while its checkouts still use it: each checkout is
/// reported as not gitscale's, and left as it is — its files, and whatever
/// work they hold, are not gitscale's to remove.
#[test]
fn edge_008_a_deleted_store_leaves_its_checkouts_alone_and_says_so() {
    let f = fixture("stores_deleted_store", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    crate::support::edit(&child.join("lib.txt"), "unsaved work");
    std::fs::remove_dir_all(store_for(&ws, &f.core)).unwrap();

    let out = gs(&ws, &["sync"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("imports/core: not a gitscale worktree"),
        "{}",
        out.stderr
    );
    assert_eq!(
        std::fs::read_to_string(child.join("lib.txt")).unwrap(),
        "unsaved work"
    );
}

/// A joined checkout deleted by hand comes back, on its topic branch with
/// its commits, on the next sync: the work was in the store all along.
#[test]
fn edge_009_a_deleted_joined_checkout_comes_back_on_its_topic_branch() {
    let f = fixture("stores_deleted_checkout", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "work");
    std::fs::remove_dir_all(&child).unwrap();

    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), work);
    assert!(writable(&child.join("lib.txt")));
}

/// A store whose creation was interrupted leaves a staging directory, never
/// a half-made store: the next sync builds the store afresh.
#[test]
fn edge_010_an_interrupted_store_creation_is_redone() {
    let f = fixture("stores_staging", "");
    let ws = f.env.repos_remote.join("ws");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            ws.to_str().unwrap(),
        ],
    );
    let store = store_for(&ws, &f.core);
    let staging = store.with_extension("staging");
    std::fs::create_dir_all(staging.join("objects")).unwrap();
    std::fs::write(staging.join("HEAD"), "garbage").unwrap();

    ok(&gs(&ws, &["sync"]));
    assert!(!staging.exists());
    assert!(store.join("HEAD").is_file());
    assert_eq!(
        head(&ws.join("imports/core")),
        git(&f.core, &["rev-parse", "v1.0.0"])
    );
}

/// An unreadable checkout record is rebuilt by the next sync from what is on
/// disk, so `sync` can still remove a checkout nothing needs any more.
#[test]
fn edge_011_a_corrupt_checkout_record_is_rebuilt_by_sync() {
    let f = fixture("stores_ledger_corrupt", "");
    let ws = f.clone_root("ws");
    let ledger = ws.join(".git/gitscale/checkouts.json");
    assert!(ledger.is_file());
    std::fs::write(&ledger, "{ not json").unwrap();

    ok(&gs(&ws, &["sync"]));
    let text = std::fs::read_to_string(&ledger).unwrap();
    let record: serde_json::Value = serde_json::from_str(&text).expect("valid JSON again");
    assert_eq!(record["checkouts"]["imports/core"], "git", "{}", text);

    // Renamed in the config: the old checkout is recorded, so it goes.
    let config = std::fs::read_to_string(ws.join(".gitscale.toml"))
        .unwrap()
        .replace("imports/core", "imports/core2");
    std::fs::write(ws.join(".gitscale.toml"), config).unwrap();
    let out = gs(&ws, &["sync"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("remove  imports/core (no longer needed)"),
        "{}",
        out.stdout
    );
    assert!(!ws.join("imports/core").exists());
}

/// Each root worktree keeps its own checkout record: a `sync` in one removes
/// only the checkouts it made, never another worktree's.
#[test]
fn edge_012_a_root_worktrees_sync_leaves_another_worktrees_checkouts_alone() {
    let f = fixture("stores_ledger_worktrees", "");
    let ws = f.clone_root("ws");
    let other = f.env.repos_remote.join("other");
    run_git_pub(
        &ws,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/y",
            other.to_str().unwrap(),
        ],
    );
    ok(&gs(&other, &["sync"]));
    let config = std::fs::read_to_string(other.join(".gitscale.toml"))
        .unwrap()
        .replace("imports/core", "imports/core2");
    std::fs::write(other.join(".gitscale.toml"), config).unwrap();

    let out = gs(&other, &["sync"]);
    ok(&out);
    assert!(!other.join("imports/core").exists(), "{}", out.stdout);
    assert!(other.join("imports/core2/lib.txt").is_file());
    let mine = ws.join("imports/core");
    assert!(mine.join("lib.txt").is_file());
    assert!(git_ok(&mine, &["status"]), "still a working checkout");
}

/// Two root worktrees synced at the same moment share one store: each waits
/// for the other's fetch, and both end up with a worktree of it.
#[test]
fn edge_013_concurrent_syncs_in_two_root_worktrees_both_succeed() {
    let f = fixture("stores_concurrent", "");
    let ws = f.env.repos_remote.join("ws");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            ws.to_str().unwrap(),
        ],
    );
    let other = f.env.repos_remote.join("other");
    run_git_pub(
        &ws,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/y",
            other.to_str().unwrap(),
        ],
    );

    let pulls: Vec<_> = [ws.clone(), other.clone()]
        .into_iter()
        .map(|dir| std::thread::spawn(move || gs_bin(&dir, &["sync"], &[])))
        .collect();
    for pull in pulls {
        ok(&pull.join().unwrap());
    }
    let store = store_for(&ws, &f.core);
    assert_eq!(
        std::fs::read_dir(store.parent().unwrap()).unwrap().count(),
        1
    );
    for dir in [&ws, &other] {
        let child = dir.join("imports/core");
        assert_eq!(common_dir(&child), store.canonicalize().unwrap());
        assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    }
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// A store directory that lost its `HEAD` is no store: the sync fails that
/// entry, names the directory, and deletes nothing — the directory may still
/// hold the records of other checkouts.
#[test]
fn error_014_a_half_deleted_store_is_reported_and_left_alone() {
    let f = fixture("stores_half_deleted", "");
    let ws = f.clone_root("ws");
    let store = store_for(&ws, &f.core);
    std::fs::remove_file(store.join("HEAD")).unwrap();

    let out = gs(&ws, &["sync"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("cannot move the new store into"),
        "{}",
        out.stderr
    );
    assert!(store.join("objects").is_dir());
    assert!(ws.join("imports/core/lib.txt").is_file());
}

/// A store keeps its `origin` pointing at the entry's URL: one whose remote
/// was removed by hand gets it back on the next sync instead of failing every
/// fetch from then on.
#[test]
#[ignore = "bug: a store whose origin was removed is never given one back"]
fn error_015_a_store_whose_origin_was_removed_gets_it_back() {
    let f = fixture("stores_no_origin", "");
    let ws = f.clone_root("ws");
    let store = store_for(&ws, &f.core);
    run_git_pub(&store, &["remote", "remove", "origin"]);

    let out = gs(&ws, &["sync"]);
    ok(&out);
    assert_eq!(
        git(&store, &["remote", "get-url", "origin"]),
        f.core.display().to_string()
    );
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

/// Two checkouts of one repository — two majors — share its store, and a
/// sync fetches that store once, not once per checkout.
#[test]
fn perf_016_a_sync_fetches_a_store_shared_by_two_majors_once() {
    let env = TestEnv::new("stores_fetch_once");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    let v2 = env.push_commit(&d, "main", "d.txt", "v2");
    run_git_pub(&d, &["tag", "v2.0.0", &v2]);
    let config = format!(
        "[repos]\n\"imports/d\" = {{ url = \"{0}\", revision = \"v2.0.0\" }}\n\
         \"imports/d_v1\" = {{ url = \"{0}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    let trace = env.repos_remote.join("trace2.json");

    let out = gs_bin(
        &ws,
        &["sync"],
        &[("GIT_TRACE2_EVENT", trace.to_str().unwrap())],
    );
    ok(&out);
    assert!(ws.join("imports/d_v1/d.txt").is_file());
    let events = std::fs::read_to_string(&trace).unwrap();
    let fetches = events
        .lines()
        .filter(|l| l.contains(r#""event":"cmd_name""#) && l.contains(r#""name":"fetch""#))
        .count();
    assert_eq!(fetches, 1, "{}", out.stdout);
}
