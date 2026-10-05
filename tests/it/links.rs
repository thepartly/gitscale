//! Dependency links: a repository's dependency on another root checkout is a
//! symlink to it. Relinking a clone that replaced one, removing orphaned
//! links, and how `ls` shows both.

use crate::support;
use crate::support::resolution::*;
use crate::support::workspace::*;
use crate::support::{git_stdout, run_git_pub, strip_ansi, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_a_dependency_of_a_dependency_links_to_the_root_checkout() {
    let env = TestEnv::new("recursive_basic_symlink");

    // Create bare repo B
    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "hello from B")]);

    // Create bare repo A that declares B as a dependency
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

    // Root config declares both
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"repoB" = {{ url = "{}", revision = "main" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // repoA/libs/b should be a symlink pointing to ../../repoB
    let link = env.playground.join("repoA/libs/b");
    assert!(link.is_symlink(), "expected symlink at repoA/libs/b");
    let target = std::fs::read_link(&link).unwrap();
    assert_eq!(
        target,
        std::path::PathBuf::from("../../repoB"),
        "symlink should be relative"
    );

    // Content accessible through symlink
    assert!(link.join("b.txt").is_file());
    let content = std::fs::read_to_string(link.join("b.txt")).unwrap();
    assert_eq!(content, "hello from B");
}

#[test]
fn normal_002_sync_relinks_a_clean_clone() {
    let (env, link) = setup_unlinked_env("sync_relinks_clean");

    // Sync should auto-relink since the clone is clean
    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(link.is_symlink(), "expected symlink restored after sync");
    let target = std::fs::read_link(&link).unwrap();
    assert_eq!(
        target,
        std::path::PathBuf::from("../../repoB"),
        "symlink target mismatch: {:?}",
        target
    );
    // Verify content is accessible through the symlink
    assert!(
        env.playground.join("repoB/b.txt").is_file(),
        "repoB/b.txt should exist"
    );
    assert!(
        link.join("b.txt").is_file(),
        "b.txt not accessible through symlink; link={:?}, target={:?}, exists={}, is_dir={}",
        link,
        target,
        link.exists(),
        link.is_dir()
    );
}

#[test]
fn normal_003b_sync_force_relinks_a_modified_clone() {
    let (env, link) = setup_unlinked_env("sync_force_relinks_modified");

    // Make the clone dirty
    std::fs::write(link.join("dirty.txt"), "local change").unwrap();
    support::run_git_pub(&link, &["add", "."]);

    // Sync with --force should relink even though modified
    let out = env.run(&["sync", "--force"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(link.is_symlink(), "expected symlink restored with --force");
    let target = std::fs::read_link(&link).unwrap();
    assert_eq!(target, std::path::PathBuf::from("../../repoB"));
    // dirty.txt should be gone (it was in the clone that got removed)
    assert!(!link.join("dirty.txt").exists());
}

#[test]
fn normal_004b_sync_force_relinks_a_clone_with_unpushed_commits() {
    let (env, link) = setup_unlinked_env("sync_force_unpushed");

    // Make a commit that isn't pushed
    support::run_git_pub(&link, &["config", "user.email", "t@t.com"]);
    support::run_git_pub(&link, &["config", "user.name", "T"]);
    std::fs::write(link.join("new.txt"), "new file").unwrap();
    support::run_git_pub(&link, &["add", "."]);
    support::run_git_pub(&link, &["commit", "-m", "unpushed"]);

    // Sync with --force should relink
    let out = env.run(&["sync", "--force"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(link.is_symlink(), "expected symlink restored with --force");
}

/// Ignored files are what a build leaves behind, not work: they do not hold
/// a relink back.
#[test]
fn normal_007_sync_relinks_a_clone_holding_only_ignored_files() {
    let (env, link) = setup_unlinked_env("sync_relinks_ignored");

    std::fs::write(link.join(".git/info/exclude"), "target/\n").unwrap();
    std::fs::create_dir_all(link.join("target")).unwrap();
    std::fs::write(link.join("target/out.o"), "build output").unwrap();

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(link.is_symlink(), "expected the clone to be relinked");
}

#[test]
fn normal_009_ls_shows_an_unlinked_clone() {
    let (env, _link) = setup_unlinked_env("status_shows_unlinked");

    let out = env.run(&["ls"]);
    assert!(out.success, "stderr: {}", out.stderr);

    let plain = strip_ansi(&out.stdout);
    assert!(
        plain.contains("unlinked"),
        "expected 'unlinked' in status output: {}",
        plain
    );
}

#[test]
fn normal_010_ls_shows_an_unlinked_clone_as_modified() {
    let (env, link) = setup_unlinked_env("status_shows_unlinked_modified");

    // Make dirty
    std::fs::write(link.join("dirty.txt"), "change").unwrap();
    support::run_git_pub(&link, &["add", "."]);

    let out = env.run(&["ls"]);
    assert!(out.success, "stderr: {}", out.stderr);

    let plain = strip_ansi(&out.stdout);
    assert!(plain.contains("unlinked"), "expected 'unlinked': {}", plain);
    assert!(plain.contains("modified"), "expected 'modified': {}", plain);
}

#[test]
fn normal_013_ls_shows_an_orphan() {
    let (env, _orphan) = setup_orphan_env("status_shows_orphan", true);

    let out = env.run(&["ls"]);
    assert!(out.success, "stderr: {}", out.stderr);

    let plain = strip_ansi(&out.stdout);
    assert!(
        plain.contains("orphan"),
        "expected 'orphan' in status output: {}",
        plain
    );
}

#[test]
fn normal_014_sync_removes_a_broken_orphan_by_default() {
    let (env, orphan) = setup_orphan_env("sync_removes_broken_orphan", false);

    // A broken orphan (target missing) is removed automatically, no --force.
    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(
        std::fs::symlink_metadata(&orphan).is_err(),
        "expected broken orphan symlink to be removed"
    );
}

#[test]
fn normal_015b_sync_force_removes_an_orphan_with_a_valid_target() {
    let (env, orphan) = setup_orphan_env("sync_force_removes_orphan", true);

    let out = env.run(&["sync", "--force"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(
        std::fs::symlink_metadata(&orphan).is_err(),
        "expected orphan symlink to be removed with --force"
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn edge_003a_sync_skips_a_modified_clone_without_force() {
    let (env, link) = setup_unlinked_env("sync_skips_modified");

    // Make the clone dirty (uncommitted changes)
    std::fs::write(link.join("dirty.txt"), "local change").unwrap();
    support::run_git_pub(&link, &["add", "."]);

    // Sync without --force should fail (non-zero exit)
    let out = env.run(&["sync"]);
    assert!(
        !out.success,
        "expected sync to fail when skipping modified unlinked clone"
    );
    assert!(
        out.stdout.contains("skip") || out.stderr.contains("skip"),
        "expected skip message, got stdout: {}, stderr: {}",
        out.stdout,
        out.stderr
    );

    assert!(
        !link.is_symlink(),
        "should still be a real dir (not relinked)"
    );
    assert!(link.join("dirty.txt").is_file());
    // The test's own name holds "skip": say exactly what was skipped and why.
    assert!(
        out.stdout
            .contains("skip  repoA/libs/b (modified, use --force to relink)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stderr
            .contains("1 unlinked repo(s) with local modifications (use --force to override)"),
        "{}",
        out.stderr
    );
}

#[test]
fn edge_004a_sync_skips_a_clone_with_unpushed_commits() {
    let (env, link) = setup_unlinked_env("sync_skips_unpushed");

    // Make a commit that isn't pushed
    support::run_git_pub(&link, &["config", "user.email", "t@t.com"]);
    support::run_git_pub(&link, &["config", "user.name", "T"]);
    std::fs::write(link.join("new.txt"), "new file").unwrap();
    support::run_git_pub(&link, &["add", "."]);
    support::run_git_pub(&link, &["commit", "-m", "unpushed"]);

    // Sync without --force should fail (non-zero exit)
    let out = env.run(&["sync"]);
    assert!(
        !out.success,
        "expected sync to fail when skipping unlinked clone with unpushed commits"
    );

    assert!(
        !link.is_symlink(),
        "should still be a real dir (unpushed commits)"
    );
    assert!(link.join("new.txt").is_file());
}

/// A commit on a branch the remote has never seen is as much local work as an
/// unpushed commit on a tracked one. With no upstream there is nothing to be
/// ahead of, so it has to be found some other way — or `sync` deletes it.
#[test]
fn edge_005_sync_skips_a_clone_with_commits_on_a_local_only_branch() {
    let (env, link) = setup_unlinked_env("sync_skips_local_branch");

    support::run_git_pub(&link, &["config", "user.email", "t@t.com"]);
    support::run_git_pub(&link, &["config", "user.name", "T"]);
    support::run_git_pub(&link, &["checkout", "-q", "-b", "wip"]);
    std::fs::write(link.join("new.txt"), "new file").unwrap();
    support::run_git_pub(&link, &["add", "."]);
    support::run_git_pub(&link, &["commit", "-q", "-m", "local only"]);

    let out = env.run(&["sync"]);
    assert!(
        !out.success,
        "expected sync to refuse a clone with commits on a local-only branch"
    );
    assert!(!link.is_symlink(), "the clone was replaced by a link");
    assert!(link.join("new.txt").is_file(), "the local commit was lost");
}

/// A stash is work set aside, not thrown away: it lives in the clone's own
/// refs, so replacing the clone would delete it.
#[test]
fn edge_006_sync_skips_a_clone_with_a_stash() {
    let (env, link) = setup_unlinked_env("sync_skips_stash");

    support::run_git_pub(&link, &["config", "user.email", "t@t.com"]);
    support::run_git_pub(&link, &["config", "user.name", "T"]);
    std::fs::write(link.join("b.txt"), "changed").unwrap();
    support::run_git_pub(&link, &["stash", "-q"]);

    let out = env.run(&["sync"]);
    assert!(
        !out.success,
        "expected sync to refuse a clone holding a stash"
    );
    assert!(!link.is_symlink(), "the clone was replaced by a link");
}

/// Names narrow a sync to the repos named. A modified clone inside repoA is
/// repoA's business: syncing an unrelated repo, even with `--force`, must not
/// replace it.
#[test]
fn edge_008_sync_with_names_leaves_other_repos_links_alone() {
    let (env, link) = setup_unlinked_env("sync_names_relink");
    let bare_c = env.create_bare_repo("repoC", "main", &[("c.txt", "c")]);
    let config_path = env.playground.join(".gitscale.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap();
    config.push_str(&format!(
        "\"repoC\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare_c.display()
    ));
    std::fs::write(&config_path, config).unwrap();
    assert!(env.run(&["sync", "repoC"]).success);

    std::fs::write(link.join("dirty.txt"), "local change").unwrap();
    support::run_git_pub(&link, &["add", "."]);

    let out = env.run(&["sync", "--force", "repoC"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        !link.is_symlink(),
        "a clone in a repo that was not named was relinked"
    );
    assert!(
        link.join("dirty.txt").is_file(),
        "its local change was lost"
    );
}

/// The repo that owns a replaced link is the declared entry it sits under,
/// however many path components that entry spans.
#[test]
fn edge_011_ls_shows_unlinked_under_a_nested_entry() {
    let (env, _link) = setup_unlinked_env_at("status_unlinked_nested", "apps/a");

    let out = env.run(&["ls"]);
    assert!(out.success, "stderr: {}", out.stderr);

    let plain = strip_ansi(&out.stdout);
    let row = plain
        .lines()
        .find(|l| l.contains("apps/a"))
        .unwrap_or_else(|| panic!("no row for apps/a:\n{}", plain));
    assert!(row.contains("unlinked"), "expected 'unlinked': {}", row);
}

/// The links GitScale plants in a checkout are not its owner's work: they
/// are kept out of git's sight, in the checkout's `info/exclude`, so they make
/// nothing dirty and `git add -A` stages none. Should that block go, `ls`
/// flags them apart from dirty, and the next placement puts it back.
#[test]
fn edge_012_planted_links_are_kept_out_of_gits_sight() {
    let env = TestEnv::new("res_untracked_links");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["sync"]).success);
    let checkout = env.playground.join("imports/b");
    assert!(checkout.join("libs/d").is_symlink());
    assert_eq!(git_stdout(&checkout, &["status", "--porcelain"]), "");
    let row = status_row(&env, "imports/b");
    assert!(row.ends_with(" ok") || row.contains(" ok "), "{}", row);

    // Real work in the same checkout is still dirty.
    support::edit(&checkout.join("README.md"), "edited");
    let row = status_row(&env, "imports/b");
    assert!(
        row.contains("dirty") && !row.contains("untracked-links"),
        "{}",
        row
    );
    run_git_pub(&checkout, &["checkout", "--", "README.md"]);

    // The block removed by hand: flagged, with the fix; the next placement
    // puts it back.
    let exclude = git_stdout(&checkout, &["rev-parse", "--git-path", "info/exclude"]);
    let exclude = checkout.join(exclude);
    std::fs::write(&exclude, "").unwrap();
    let out = env.run(&["ls"]);
    let text = strip_ansi(&out.stdout);
    let row = text.lines().find(|l| l.contains("imports/b")).unwrap();
    assert!(
        row.contains("untracked-links") && !row.contains("dirty"),
        "{}",
        text
    );
    assert!(text.contains("hint: imports/b: "), "{}", text);
    assert!(env.run(&["sync"]).success);
    assert!(std::fs::read_to_string(&exclude)
        .unwrap()
        .contains("/libs/d"));
    let row = status_row(&env, "imports/b");
    assert!(!row.contains("untracked-links"), "{}", row);
}

#[test]
fn edge_015a_sync_skips_an_orphan_with_a_valid_target_without_force() {
    let (env, orphan) = setup_orphan_env("sync_skips_orphan_valid", true);

    // An orphan whose target still resolves requires --force.
    let out = env.run(&["sync"]);
    assert!(
        !out.success,
        "expected sync to fail when skipping orphan with valid target"
    );
    assert!(
        out.stdout.contains("orphan") || out.stderr.contains("orphan"),
        "expected orphan message, stdout: {}, stderr: {}",
        out.stdout,
        out.stderr
    );
    assert!(
        std::fs::symlink_metadata(&orphan)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false),
        "orphan symlink with valid target should remain without --force"
    );
    // The test's own path holds "orphan": say exactly what was kept and why.
    assert!(
        out.stdout
            .contains("skip  repoA/libs/c (orphan with valid target, use --force to remove)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stderr.contains("1 orphaned link(s) with valid targets"),
        "{}",
        out.stderr
    );
}

/// A child is never a workspace: `sync` typed inside one places the whole
/// workspace, whose root decides the dependency's revision — the link inside
/// the child stays, and the checkout it points at stays at the root's pin.
#[test]
fn edge_016_sync_inside_a_child_places_the_workspace_and_keeps_the_link() {
    let env = TestEnv::new("pull_child_outer_link");
    let (child, main_tip) = child_with_outer_link(&env);
    let link = child.join("libs/b");

    let out = run_in(&child, "sync", &[]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("ok    repoA"), "{}", out.stdout);
    assert!(out.stdout.contains("ok    repoB"), "{}", out.stdout);
    assert!(link.is_symlink(), "the dedup link must survive");
    assert_eq!(
        git_stdout(&env.playground.join("repoB"), &["rev-parse", "HEAD"]),
        main_tip,
        "the root's pin must win over the child's"
    );
}

/// A link path named from inside a child names the checkout it points at:
/// that checkout is placed, at the root's revision, and the link is not
/// replaced by a checkout of its own.
#[test]
fn edge_017_naming_a_link_inside_a_child_places_its_checkout_and_keeps_the_link() {
    let env = TestEnv::new("child_unlink_named");
    let (child, main_tip) = child_with_outer_link(&env);
    let link = child.join("libs/b");

    let out = run_in(&child, "sync", &["libs/b"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("ok    repoB"), "{}", out.stdout);
    assert!(!out.stdout.contains("repoA"), "{}", out.stdout);
    assert!(link.is_symlink(), "naming a link does not unlink it");
    assert_eq!(
        git_stdout(&env.playground.join("repoB"), &["rev-parse", "HEAD"]),
        main_tip
    );
}

/// Any other symlink at an entry path is replaced by a real checkout: placement
/// lands where a fresh one would, and the checkout the link pointed at is left
/// untouched.
#[test]
fn edge_018_placement_replaces_an_in_workspace_symlink_with_a_checkout() {
    let env = TestEnv::new("pull_inner_symlink");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "v1")]);
    let other = env.create_bare_repo("other", "main", &[("o.txt", "o")]);
    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"libs/alias" = {{ url = "{}", revision = "main" }}
"#,
        bare.display(),
        other.display(),
    ));
    let out = env.run(&["sync", "libs/mylib"]);
    assert!(out.success, "stderr: {}", out.stderr);
    std::os::unix::fs::symlink("mylib", env.playground.join("libs/alias")).unwrap();
    let before = git_stdout(&env.playground.join("libs/mylib"), &["rev-parse", "HEAD"]);

    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let alias = env.playground.join("libs/alias");
    assert!(!alias.is_symlink(), "the symlink should be replaced");
    assert!(
        alias.join("o.txt").is_file(),
        "with a clone of its own repo"
    );
    assert_eq!(
        git_stdout(&env.playground.join("libs/mylib"), &["rev-parse", "HEAD"]),
        before
    );
}

/// Untracked files are work too, even unstaged: a clone holding one is not
/// replaced by the link without `--force`.
#[test]
fn edge_019_sync_skips_a_clone_holding_untracked_files() {
    let (env, link) = setup_unlinked_env("links_untracked_clone");
    std::fs::write(link.join("notes.txt"), "mine").unwrap();

    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(!link.is_symlink(), "the clone was replaced by a link");
    assert_eq!(
        std::fs::read_to_string(link.join("notes.txt")).unwrap(),
        "mine"
    );
}

/// Where a link belongs, a plain directory with no repository of its own —
/// files copied in by hand, say — holds nobody knows what. `sync` must not
/// take it for a clean clone (git run inside it answers for the repository
/// around it) and delete it.
#[test]
#[ignore = "bug: is_tree_modified runs git in the plain directory, which answers for repoA; with libs/ ignored there the directory is deleted"]
fn edge_020_sync_never_replaces_a_plain_directory_at_a_link_path() {
    let (env, link) = setup_unlinked_env("links_plain_dir");
    std::fs::remove_dir_all(link.join(".git")).unwrap();
    std::fs::write(link.join("mine.txt"), "mine").unwrap();
    // repoA ignores where its links live, as docs/dependencies.md advises.
    let repo_a = env.playground.join("repoA");
    let exclude = repo_a.join(git_stdout(
        &repo_a,
        &["rev-parse", "--git-path", "info/exclude"],
    ));
    std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    std::fs::write(&exclude, "/libs/\n").unwrap();

    let out = env.run(&["sync"]);
    assert!(
        !link.is_symlink(),
        "the directory was replaced: {}",
        out.stdout
    );
    assert_eq!(
        std::fs::read_to_string(link.join("mine.txt")).unwrap(),
        "mine"
    );
    assert!(!out.success, "keeping it asks for --force: {}", out.stdout);
}

/// A symlink the repository itself tracks is its content, not a link
/// gitscale planted — even one that points at a direct child of the root,
/// broken or not. `sync` leaves it alone, `--force` included.
#[test]
#[ignore = "bug: orphan detection only looks at where a link points, so a tracked symlink to a root child is removed as an orphan"]
fn edge_021_sync_leaves_a_symlink_the_repository_tracks() {
    let env = TestEnv::new("links_tracked_symlink");
    let bare = env.create_bare_repo("repoA", "main", &[("a.txt", "a")]);
    let work = env.repos_remote.join("repoA-work");
    run_git_pub(
        &env.repos_remote,
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    std::os::unix::fs::symlink("../NOTES.md", work.join("notes")).unwrap();
    run_git_pub(&work, &["add", "notes"]);
    run_git_pub(
        &work,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "link",
        ],
    );
    run_git_pub(&work, &["push", "-q", "origin", "main"]);
    env.write_config(&format!(
        "[repos]\n\"repoA\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    assert!(env.run(&["sync"]).success);
    let tracked = env.playground.join("repoA/notes");
    let is_link = || std::fs::symlink_metadata(&tracked).is_ok_and(|m| m.file_type().is_symlink());
    assert!(is_link());

    // Broken: the root has no NOTES.md.
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(is_link(), "a tracked symlink was removed: {}", out.stdout);

    // Pointing at something, and with --force.
    std::fs::write(env.playground.join("NOTES.md"), "notes").unwrap();
    let out = env.run(&["sync", "--force"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(is_link(), "a tracked symlink was removed: {}", out.stdout);
}
