//! Placement — what `git scale sync`, `git scale pull`, the hook and CI all
//! do: putting every checkout where resolution says, checking out what is
//! missing, and never losing work to a move.

use crate::support;
use crate::support::resolution::*;
use crate::support::workspace::*;
use crate::support::{git_stdout, redact_shas, run_git_pub, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// Every checkout is a worktree of the root's own store for its repository,
/// detached at the commit its revision names, every file read-only.
#[test]
fn normal_001_makes_each_checkout_a_detached_readonly_worktree_of_the_root_store() {
    let env = TestEnv::new("pull_fresh");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# mylib\n")]);
    let url = bare.display().to_string();

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        url
    ));

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("sync_fresh_stdout", out.stdout);

    let checkout = env.playground.join("libs/mylib");
    let file = checkout.join("README.md");
    assert!(file.is_file());
    assert!(
        checkout.join(".git").is_file(),
        "a worktree has a .git file"
    );
    let common = git_stdout(
        &checkout,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    assert_eq!(
        std::path::PathBuf::from(common).canonicalize().unwrap(),
        env.store(&url).canonicalize().unwrap()
    );
    assert_eq!(
        git_stdout(&checkout, &["rev-parse", "HEAD"]),
        git_stdout(&bare, &["rev-parse", "main"])
    );
    let branch = std::process::Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .current_dir(&checkout)
        .output()
        .unwrap();
    assert!(!branch.status.success(), "detached, not on a branch");

    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::metadata(&file).unwrap().permissions();
    assert_eq!(perms.mode() & 0o222, 0, "file should be readonly");
}

#[test]
fn normal_002_a_second_placement_leaves_a_checkout_where_it_is() {
    let env = TestEnv::new("pull_twice");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# mylib\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    assert!(env.run(&["sync"]).success);
    let checkout = env.playground.join("libs/mylib");
    let head = git_stdout(&checkout, &["rev-parse", "HEAD"]);
    let link = std::fs::read_to_string(checkout.join(".git")).unwrap();
    let out = env.run(&["sync"]);
    assert!(out.success);
    insta::assert_snapshot!("sync_twice_stdout", out.stdout);
    // Where it was, and the same worktree — not one made again.
    assert_eq!(git_stdout(&checkout, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read_to_string(checkout.join(".git")).unwrap(),
        link
    );
}

#[test]
fn normal_003_directories_select_which_entries_to_place() {
    let env = TestEnv::new("pull_selective");
    let bare1 = env.create_bare_repo("lib1", "main", &[("a.txt", "a")]);
    let bare2 = env.create_bare_repo("lib2", "main", &[("b.txt", "b")]);

    env.write_config(&format!(
        r#"[repos]
"libs/lib1" = {{ url = "{}", revision = "main" }}
"libs/lib2" = {{ url = "{}", revision = "main" }}
"#,
        bare1.display(),
        bare2.display(),
    ));

    let out = env.run(&["sync", "libs/lib1"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(env.playground.join("libs/lib1/a.txt").is_file());
    assert!(!env.playground.join("libs/lib2").exists());
}

#[test]
fn normal_004_places_checkouts_and_artefacts_together() {
    let env = TestEnv::new("multiple_repos_mixed");
    let bare_rw = env.create_bare_repo("rw-lib", "main", &[("rw.txt", "readwrite")]);
    let bare_ro = env.create_bare_repo("ro-lib", "main", &[("ro.txt", "readonly")]);
    let bare_art = env.create_bare_repo("art", "main", &[("README.md", "art")]);
    run_git_pub(&bare_art, &["tag", "v1.0.0", "main"]);
    env.publish(&bare_art, "v1.0.0", &[("art.bin", "artefact-data")]);

    env.prefer(&bare_art, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        r#"{}[repos]
"libs/ro-lib" = {{ url = "{}", revision = "main" }}
"libs/rw-lib" = {{ url = "{}", revision = "main" }}
"meta/art" = {{ url = "{}", revision = "v1.0.0" }}
"#,
        env.registries(),
        bare_ro.display(),
        bare_rw.display(),
        bare_art.display(),
    ));

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("multiple_repos_mixed_stdout", redact_shas(&out.stdout));

    assert!(env.playground.join("libs/rw-lib/rw.txt").is_file());
    assert!(env.playground.join("libs/ro-lib/ro.txt").is_file());
    assert!(env.playground.join("meta/art/dist/art.bin").is_file());
}

/// The refs come first, then the move: a checkout pinned to a branch that did
/// not exist when it was made finds it, and lands detached at its tip.
#[test]
fn normal_005_moves_a_checkout_to_a_branch_made_after_it() {
    let env = TestEnv::new("pull_new_branch");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "v1")]);
    let config = |revision: &str| {
        format!(
            "[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"{}\" }}\n",
            bare.display(),
            revision
        )
    };
    env.write_config(&config("main"));
    assert!(env.run(&["sync"]).success);

    bare_git_stdout(&bare, &["branch", "feature", "main"]);
    let tip = commit_to_bare(&bare, "feature", "a.txt", "feature");
    env.write_config(&config("feature"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let dest = env.playground.join("libs/mylib");
    assert_eq!(git_stdout(&dest, &["rev-parse", "HEAD"]), tip);
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).unwrap(),
        "feature"
    );
}

/// No revision means the remote's default branch: sync moves to its new head.
#[test]
fn normal_006_without_a_revision_follows_the_default_branch() {
    let env = TestEnv::new("pull_no_rev");
    // Not `main` or `master`: nothing may assume the default's name.
    let bare = env.create_bare_repo("mylib", "trunk", &[("a.txt", "v1")]);
    env.write_config(&format!(
        "[repos]\n\"libs/mylib\" = {{ url = \"{}\" }}\n",
        bare.display()
    ));
    assert!(env.run(&["sync"]).success);

    let second = commit_to_bare(&bare, "trunk", "a.txt", "v2");
    let out = env.run(&["sync"]);
    assert!(out.success, "pull failed: {}{}", out.stdout, out.stderr);
    let dest = env.playground.join("libs/mylib");
    assert_eq!(git_stdout(&dest, &["rev-parse", "HEAD"]), second);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A tag whose name is all hex digits is a tag, not a commit.
#[test]
fn edge_007_an_all_hex_tag_name_is_a_tag() {
    let env = TestEnv::new("hex_tag_git");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "release")]);
    let tagged = git_stdout(&bare, &["rev-parse", "main"]);
    run_git_pub(&bare, &["tag", "20241001", "main"]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"20241001\" }}\n",
        bare.display()
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}", out.stderr);
    let checkout = env.playground.join("libs/lib");
    assert_eq!(git_stdout(&checkout, &["rev-parse", "HEAD"]), tagged);
    assert_eq!(
        git_stdout(&checkout, &["describe", "--tags", "--exact-match"]),
        "20241001"
    );
}

/// git exports `GIT_DIR` to hooks — during `git clone`, the new root's `.git`.
/// A hook-triggered placement that let its git calls inherit it ran them
/// against the root: the pinned tag "did not exist", and with an existing
/// store the fetch rewrote the root's refs and checked the sub-repo's tag out
/// over it.
#[test]
fn edge_008_ignores_an_inherited_git_dir() {
    let env = TestEnv::new("pull_inherited_git_dir");
    env.init_playground_git();
    env.set_playground_origin("https://github.com/acme/root.git");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    support::run_git_pub(&bare, &["tag", "demo-v1", "main"]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "demo-v1" }}
"#,
        bare.display()
    ));
    let root_git = env.playground.join(".git");
    let rev = |dir: &std::path::Path, spec: &str| {
        let out = std::process::Command::new("git")
            .args(["rev-parse", spec])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let root_head = rev(&env.playground, "HEAD");
    let tag = rev(&bare, "demo-v1");

    // No store first, then an existing one: the second is the one that
    // damaged the root.
    for pass in ["cold", "warm"] {
        let _ = std::fs::remove_dir_all(env.playground.join("libs"));
        let out = env.run_with_env(&[("GIT_DIR", root_git.to_str().unwrap())], &["sync"]);
        assert!(
            out.success,
            "{} pass\nstdout: {}\nstderr: {}",
            pass, out.stdout, out.stderr
        );
        let lib = env.playground.join("libs/mylib");
        assert_eq!(
            rev(&lib, "HEAD"),
            tag,
            "{} pass: sub-repo not on the tag",
            pass
        );
        assert_eq!(
            rev(&env.playground, "HEAD"),
            root_head,
            "{} pass: root moved",
            pass
        );
        let leaked = std::process::Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", "refs/tags/demo-v1"])
            .current_dir(&env.playground)
            .output()
            .unwrap();
        assert!(
            !leaked.status.success(),
            "{} pass: the sub-repo's tag was fetched into the root",
            pass
        );
        // Creating the store ran `init --bare` on the root; updating one
        // repointed the root's origin at the sub-repo.
        assert_eq!(
            rev(&env.playground, "--is-bare-repository"),
            "false",
            "{} pass: the root was reinitialised as bare",
            pass
        );
        let origin = std::process::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(&env.playground)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&origin.stdout).trim(),
            "https://github.com/acme/root.git",
            "{} pass: the root's origin was repointed",
            pass
        );
    }
}

/// Commits made on a checkout's detached HEAD are somebody's work no branch
/// holds: sync will not move away from them.
#[test]
fn edge_009_refuses_to_lose_commits_made_at_a_pin() {
    let env = TestEnv::new("pull_detached_commits");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "v1")]);
    env.write_config(&format!(
        "[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    assert!(env.run(&["sync"]).success);
    let dest = env.playground.join("libs/mylib");
    support::run_git_pub(&dest, &["config", "user.email", "t@t.com"]);
    support::run_git_pub(&dest, &["config", "user.name", "T"]);
    std::fs::write(dest.join("local.txt"), "mine").unwrap();
    support::run_git_pub(&dest, &["add", "-A"]);
    support::run_git_pub(&dest, &["commit", "-q", "-m", "local"]);
    let local = git_stdout(&dest, &["rev-parse", "HEAD"]);
    commit_to_bare(&bare, "main", "a.txt", "v2");

    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(text.contains("is on no branch"), "{}", text);
    assert_eq!(git_stdout(&dest, &["rev-parse", "HEAD"]), local);
}

#[test]
fn edge_010_clones_into_an_empty_directory_and_leaves_the_workspace_alone() {
    // What a failed clone, an interrupted delete or an outside cleaner leaves.
    // Git run in there walks up to the workspace, so treating it as a checkout
    // checked out `main` over the workspace's own branch.
    let env = workspace_with_stray_directory("pull_empty_entry_dir", |_| {});

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert_eq!(
        git_out(&env.playground, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature",
        "the workspace's own branch must not move"
    );
    assert!(
        env.playground.join("libs/core/README.md").is_file(),
        "the entry should be cloned into the empty directory"
    );
}

#[test]
fn edge_011_moves_a_checkout_only_when_nothing_can_be_lost() {
    let env = TestEnv::new("res_safe_move");
    let d = tagged(&env, "d", &[("v1.2.0", ""), ("v1.5.0", "")]);
    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.2.0\"")]));
    assert!(env.run(&["sync"]).success);
    let dest = env.playground.join("imports/d");
    support::edit(&dest.join("README.md"), "local edit");

    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.5.0\"")]));
    let out = env.run(&["sync"]);
    assert!(!out.success);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(text.contains("not moved: uncommitted changes"), "{}", text);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.2.0"));
    assert_eq!(
        std::fs::read_to_string(dest.join("README.md")).unwrap(),
        "local edit"
    );

    run_git_pub(&dest, &["checkout", "--", "README.md"]);
    assert!(env.run(&["sync"]).success);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
}

/// A checkout moves with a plain checkout, so an untracked file in the way of
/// the new revision stops the move rather than being overwritten.
#[test]
fn edge_012_a_move_never_overwrites_an_untracked_file() {
    let env = TestEnv::new("res_untracked_in_the_way");
    let d = env.create_bare_repo("d", "main", &[("README.md", "d")]);
    let v1 = env.push_commit(&d, "main", "VERSION", "1");
    run_git_pub(&d, &["tag", "v1", &v1]);
    let v2 = env.push_commit(&d, "main", "new.txt", "from v2");
    run_git_pub(&d, &["tag", "v2", &v2]);
    let url = format!("file://{}", d.display());
    let entry = |rev: &str| {
        format!(
            "[repos]\n\"imports/d\" = {{ url = \"{}\", revision = \"{}\" }}\n",
            url, rev
        )
    };
    env.write_config(&entry("v1"));
    assert!(env.run(&["sync"]).success);
    let dest = env.playground.join("imports/d");
    std::fs::write(dest.join("new.txt"), "mine").unwrap();

    env.write_config(&entry("v2"));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        std::fs::read_to_string(dest.join("new.txt")).unwrap(),
        "mine"
    );
    assert_eq!(head(&env, "imports/d"), v1);
}

/// A detached HEAD with a commit no branch holds is not moved: switching away
/// would leave that commit reachable only through the reflog.
#[test]
fn edge_013_does_not_move_a_detached_head_with_commits_on_no_branch() {
    let env = TestEnv::new("res_detached_work");
    let d = tagged(&env, "d", &[("v1.2.0", ""), ("v1.5.0", "")]);
    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.2.0\"")]));
    assert!(env.run(&["sync"]).success);
    let dest = env.playground.join("imports/d");
    run_git_pub(
        &dest,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "--allow-empty",
            "-m",
            "work on no branch",
        ],
    );
    let work = head(&env, "imports/d");

    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.5.0\"")]));
    let out = env.run(&["sync"]);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(!out.success, "{}", text);
    assert!(text.contains("is on no branch"), "{}", text);
    assert_eq!(head(&env, "imports/d"), work);
}

/// An annotated tag is an object of its own; the checkout lands on the
/// commit it points at, as git itself would check it out.
#[test]
fn edge_020_pins_an_annotated_tag_at_its_commit() {
    let env = TestEnv::new("pull_annotated_tag");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "release")]);
    let commit = git_stdout(&bare, &["rev-parse", "main"]);
    run_git_pub(
        &bare,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "tag",
            "-a",
            "v1.0.0",
            "-m",
            "release",
            "main",
        ],
    );
    env.push_commit(&bare, "main", "a.txt", "later");
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        bare.display()
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let checkout = env.playground.join("libs/lib");
    assert_eq!(git_stdout(&checkout, &["rev-parse", "HEAD"]), commit);
    assert_eq!(
        std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "release"
    );
}

/// A full SHA is the commit itself, however it is spelt: a commit behind the
/// tip is checked out exactly, in lower or upper case.
#[test]
fn edge_021_a_full_sha_pins_that_commit() {
    let env = TestEnv::new("pull_full_sha");
    let mut config = String::from("[repos]\n");
    let mut wanted = Vec::new();
    for (name, upper) in [("lower", false), ("upper", true)] {
        let bare = env.create_bare_repo(name, "main", &[("a.txt", "first")]);
        let first = git_stdout(&bare, &["rev-parse", "main"]);
        env.push_commit(&bare, "main", "a.txt", "second");
        let spelt = if upper {
            first.to_uppercase()
        } else {
            first.clone()
        };
        config.push_str(&format!(
            "\"libs/{}\" = {{ url = \"{}\", revision = \"{}\" }}\n",
            name,
            bare.display(),
            spelt
        ));
        wanted.push((name, first));
    }
    env.write_config(&config);
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    for (name, first) in wanted {
        let checkout = env.playground.join("libs").join(name);
        assert_eq!(
            git_stdout(&checkout, &["rev-parse", "HEAD"]),
            first,
            "{}",
            name
        );
    }
}

/// A branch and a tag may share a name; the branch wins, as the resolver
/// documents (`ls_remote_revision`).
#[test]
fn edge_022_a_branch_wins_over_a_tag_of_the_same_name() {
    let env = TestEnv::new("pull_branch_and_tag");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "tagged")]);
    let tagged = git_stdout(&bare, &["rev-parse", "main"]);
    let tip = env.push_commit(&bare, "main", "a.txt", "branch tip");
    run_git_pub(&bare, &["tag", "release", &tagged]);
    run_git_pub(&bare, &["branch", "release", &tip]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"release\" }}\n",
        bare.display()
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        git_stdout(&env.playground.join("libs/lib"), &["rev-parse", "HEAD"]),
        tip
    );
}

/// A config with no entries has nothing to sync: the command says so and
/// succeeds.
#[test]
fn edge_023_with_no_entries_says_there_is_nothing_to_sync() {
    let env = TestEnv::new("pull_no_entries");
    env.write_config("[repos]\n");
    let out = env.run(&["sync"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(out.stdout, "Nothing to sync.\n");
}

/// Commits on a local branch are kept by the branch, so they do not block a
/// move: the checkout moves to its new pin and the branch keeps the work.
#[test]
fn edge_024_a_commit_on_a_local_branch_does_not_block_a_move() {
    let env = TestEnv::new("pull_local_branch");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "v1")]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    assert!(env.run(&["sync"]).success);
    let checkout = env.playground.join("libs/lib");
    run_git_pub(&checkout, &["switch", "-q", "-c", "mywork"]);
    std::fs::write(checkout.join("mine.txt"), "mine").unwrap();
    run_git_pub(&checkout, &["add", "mine.txt"]);
    run_git_pub(
        &checkout,
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
    let mine = git_stdout(&checkout, &["rev-parse", "HEAD"]);
    let tip = env.push_commit(&bare, "main", "a.txt", "v2");

    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(git_stdout(&checkout, &["rev-parse", "HEAD"]), tip);
    assert_eq!(
        git_stdout(&checkout, &["rev-parse", "refs/heads/mywork"]),
        mine
    );
}

/// Entries may nest (`deps` and `deps/inner`). Moving the outer checkout
/// must leave the inner one's files as they were: a joined inner checkout
/// stays writable.
#[test]
#[ignore = "bug: restore_writable/apply_readonly on the outer checkout walk into the nested one, leaving its files read-only"]
fn edge_025_moving_an_outer_checkout_leaves_a_nested_checkouts_write_bits() {
    use std::os::unix::fs::PermissionsExt;
    let writable =
        |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o200 != 0;

    let env = TestEnv::new("pull_nested_write_bits");
    let outer = env.create_bare_repo("outer", "main", &[("o.txt", "v1")]);
    let inner = env.create_bare_repo("inner", "main", &[("i.txt", "i")]);
    let root_remote = env.create_bare_repo("root", "main", &[("README.md", "root")]);
    env.write_config(&format!(
        "[repos]\n\"deps\" = {{ url = \"{}\", revision = \"main\" }}\n\
         \"deps/inner\" = {{ url = \"{}\", revision = \"main\" }}\n",
        outer.display(),
        inner.display()
    ));
    env.init_playground_git();
    env.set_playground_origin(root_remote.to_str().unwrap());
    assert!(env.run(&["sync"]).success);
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let out = env.run(&["topic", "join", "deps/inner"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let file = env.playground.join("deps/inner/i.txt");
    assert!(writable(&file), "develop makes the checkout writable");

    env.push_commit(&outer, "main", "o.txt", "v2");
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        std::fs::read_to_string(env.playground.join("deps/o.txt")).unwrap(),
        "v2"
    );
    assert!(
        writable(&file),
        "the developed nested checkout lost its write bits"
    );
}

/// A name with a trailing slash — what shell completion of a directory
/// gives — selects the entry, as it does for `git topic join`.
#[test]
#[ignore = "bug: names are matched as exact strings, so `sync libs/lib1/` is 'Unknown repos: libs/lib1/'"]
fn edge_029_a_name_with_a_trailing_slash_selects_its_entry() {
    let env = TestEnv::new("pull_trailing_slash");
    let bare = env.create_bare_repo("lib1", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib1\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    let out = env.run(&["sync", "libs/lib1/"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("libs/lib1/a.txt").is_file());
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// A commit is only ever a full SHA. An abbreviated one is taken for a branch
/// or tag name, which the remote does not have — and the failure says why.
#[test]
fn error_014_an_abbreviated_sha_fails_with_a_hint() {
    let env = TestEnv::new("short_sha_git");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "a")]);
    let short = git_stdout(&bare, &["rev-parse", "--short=9", "main"]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"{}\" }}\n",
        bare.display(),
        short
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success);
    assert!(out.stderr.contains("full SHA"), "{}", out.stderr);
    assert!(
        out.stderr.contains(&format!("git rev-parse {}", short)),
        "{}",
        out.stderr
    );
}

#[test]
fn error_015_an_unknown_name_is_refused() {
    let env = TestEnv::new("pull_unknown_name");
    let bare = env.create_bare_repo("lib1", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/lib1" = {{ url = "{}", revision = "main" }}
"#,
        bare.display(),
    ));

    let out = env.run(&["sync", "nonexistent"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("nonexistent is not a checkout of this workspace"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("libs/lib1").exists());

    // Mixed with a known name, nothing runs either.
    let out = env.run(&["sync", "libs/lib1", "nonexistent"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("nonexistent is not a checkout of this workspace"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("libs/lib1").exists(), "{}", out.stdout);
}

/// A sync that cannot reach the remote fails, rather than reporting `ok` for
/// a checkout it never updated.
#[test]
fn error_016_fails_when_the_remote_is_unreachable() {
    let env = TestEnv::new("pull_unreachable");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "v1")]);
    env.write_config(&format!(
        "[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    assert!(env.run(&["sync"]).success);
    let checkout = env.playground.join("libs/mylib");
    let head = git_stdout(&checkout, &["rev-parse", "HEAD"]);
    std::fs::rename(&bare, bare.with_extension("gone")).unwrap();

    let out = env.run(&["sync"]);
    assert!(!out.success, "pull should fail: {}", out.stdout);
    assert!(!out.stdout.contains("ok    libs/mylib"), "{}", out.stdout);
    assert!(out.stderr.contains("Error: "), "{}", out.stderr);
    assert_eq!(git_stdout(&checkout, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "v1"
    );
}

/// A move that local changes block fails, and the changes survive.
#[test]
fn error_017_fails_when_local_changes_block_the_move() {
    let env = TestEnv::new("pull_blocked_by_changes");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "v1")]);
    env.write_config(&format!(
        "[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    assert!(env.run(&["sync"]).success);
    let dest = env.playground.join("libs/mylib");
    support::edit(&dest.join("a.txt"), "edited");
    commit_to_bare(&bare, "main", "a.txt", "v2");

    let out = env.run(&["sync"]);
    assert!(!out.success, "pull should fail: {}", out.stdout);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(text.contains("FAIL  libs/mylib"), "{}", text);
    assert!(text.contains("uncommitted changes"), "{}", text);
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).unwrap(),
        "edited"
    );
}

#[test]
fn error_018_refuses_a_directory_that_holds_something_else() {
    let env = workspace_with_stray_directory("pull_stray_entry_dir", |dir| {
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
    });

    let out = env.run(&["sync"]);
    assert!(!out.success, "stdout: {}", out.stdout);
    let said = format!("{}{}", out.stdout, out.stderr);
    assert!(said.contains("holds no git repository"), "{}", said);
    assert_eq!(
        std::fs::read_to_string(env.playground.join("libs/core/notes.txt")).unwrap(),
        "mine",
        "whatever is there is somebody's, and must be left"
    );
    assert_eq!(
        git_out(&env.playground, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature"
    );
}

/// One entry that cannot be brought up to date — an artefact whose pipeline
/// has not published yet — fails the command, but only after everything else
/// is done: the rest placed and linked.
#[test]
fn error_019_one_failing_entry_fails_the_command_after_the_rest_is_done() {
    let env = TestEnv::new("res_fail_at_end");
    let (b, d) = diamond(&env);
    let art = env.artefact_repo("art", &[("app.bin", "v1")]);
    env.prefer(&art, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        "{}{}",
        env.registries(),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/d", &d, ", revision = \"v1.2.0\""),
            ("meta/art", &art, ", revision = \"v1.1.0\""),
        ])
    ));
    // A release whose image does not exist yet, before anything is cloned.
    env.push_commit(&art, "main", "README.md", "unpublished");
    run_git_pub(&art, &["tag", "v1.1.0", "main"]);

    let out = env.run(&["sync"]);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(!out.success, "sync should fail: {}", text);
    assert!(text.contains("no artefact for"), "{}", text);
    assert!(
        out.stderr.contains("FAIL  meta/art")
            && out.stderr.contains("Error: 1 repo(s) failed to place"),
        "{}",
        out.stderr
    );
    assert_eq!(
        head(&env, "imports/d"),
        tag_commit(&d, "v1.5.0"),
        "the rest is still placed"
    );
    assert!(
        env.playground.join("imports/b/libs/d").is_symlink(),
        "and linked"
    );
}

/// A revision the remote has neither as a branch nor as a tag fails the
/// entry, naming the revision, and leaves nothing behind.
#[test]
fn error_026_a_revision_the_remote_lacks_fails_naming_it() {
    let env = TestEnv::new("pull_missing_revision");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"no-such-branch\" }}\n",
        bare.display()
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("no-such-branch"), "{}", out.stderr);
    assert!(!env.playground.join("libs/lib").exists());
}

/// A checkout stays inside the workspace even when a directory on its path is
/// a symlink out of it — one the root repository tracks arrives with any
/// branch, just as the config does.
#[test]
#[ignore = "bug: directories are checked lexically only, so a symlinked component lets a checkout land outside the workspace"]
fn error_027_never_places_a_checkout_outside_the_workspace_through_a_symlink() {
    let env = TestEnv::new("pull_symlink_escape");
    let bare = env.create_bare_repo("lib", "main", &[("payload.sh", "echo pwned")]);
    let outside = env.repos_remote.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, env.playground.join("evil")).unwrap();
    env.write_config(&format!(
        "[repos]\n\"evil/x\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    let out = env.run(&["sync"]);
    assert!(
        !outside.join("x").exists(),
        "a checkout was written outside the workspace: {}{}",
        out.stdout,
        out.stderr
    );
    assert!(!out.success, "{}", out.stdout);
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

/// Each store is fetched once per sync, however many times resolution and
/// placement ask for it — the first sync and every later one.
#[test]
fn perf_028_fetches_each_store_once_per_placement() {
    let env = TestEnv::new("pull_fetch_count");
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
        assert_eq!(take_git_count(&log, "fetch --prune"), 5, "{} pull", round);
    }
}
