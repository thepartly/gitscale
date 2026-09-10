// Cloning sub-repositories from a copy already on the machine.
//
// The workspace a clone borrows from is reached two different ways — a linked
// worktree, or a workspace cloned with `--reference` — and the two leave
// completely different traces on disk, so both are exercised here. Every test
// asserts on `objects/info/alternates`, since that file is the whole
// observable difference between a borrowed clone and an ordinary one.
#[allow(dead_code)]
mod helpers;

use helpers::TestEnv;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run git with this machine's own hooks disabled.
///
/// A developer running these tests may well have gitscale's own hook installed
/// globally or system-wide — that is what it is for. It fires on
/// `git worktree add` and on cloning a workspace, and populates the new
/// checkout by running the *installed* gitscale before the test can run this
/// one. Every assertion below would then be measuring the wrong binary.
fn git_isolated(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {:?} in {} failed:\n{}{}",
        args,
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Run the CLI against an arbitrary root: `TestEnv::run` always points `-C` at
/// the playground, and every test here needs a second workspace beside it.
fn clone_into(root: &Path) -> helpers::CliOutput {
    gitscale::run_cli_with(&["gitscale", "clone", "-C", root.to_str().unwrap()], false)
}

/// The alternates file of a cloned sub-repository, if it has one.
fn alternates_of(repo: &Path) -> Option<String> {
    std::fs::read_to_string(repo.join(".git/objects/info/alternates"))
        .ok()
        .map(|s| s.trim().to_string())
}

fn config_for(bare: &Path, extra: &str) -> String {
    format!(
        "{}[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        extra,
        bare.display()
    )
}

/// A workspace that is a committed git repo with its config tracked (so a
/// worktree of it has one), and with `libs/core` already cloned.
fn populated_workspace(env: &TestEnv, extra_config: &str) -> PathBuf {
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare, extra_config));
    // Commit before cloning, so the sub-repo stays untracked and a worktree
    // starts out empty — which is the situation being tested.
    env.init_playground_git();
    let out = env.run(&["clone"]);
    assert!(
        out.success,
        "seeding the source workspace failed: {:?}",
        out.stderr
    );
    assert!(env.playground.join("libs/core/.git").exists());
    bare
}

/// `git worktree add` inside the workspace, so the tree is cleaned up with it.
fn add_worktree(env: &TestEnv, name: &str) -> PathBuf {
    git_isolated(&env.playground, &["worktree", "add", "-b", name, name]);
    let wt = env.playground.join(name);
    assert!(
        wt.join(".gitscale.toml").is_file(),
        "the worktree should have the tracked config checked out"
    );
    wt
}

// ---------------------------------------------------------------------------
// Discovery — worktrees
// ---------------------------------------------------------------------------

#[test]
fn a_worktree_borrows_from_the_workspace_it_was_made_from() {
    let env = TestEnv::new("share_worktree");
    populated_workspace(&env, "");
    let wt = add_worktree(&env, "wt");

    let out = clone_into(&wt);
    assert!(
        out.success,
        "clone in the worktree failed: {:?}",
        out.stderr
    );

    let borrowed = wt.join("libs/core");
    let alternates = alternates_of(&borrowed).expect("the worktree clone should borrow objects");
    let expected = env
        .playground
        .join("libs/core/.git/objects")
        .canonicalize()
        .unwrap();
    assert_eq!(
        PathBuf::from(&alternates).canonicalize().unwrap(),
        expected,
        "it should borrow from the main worktree's copy"
    );
    assert_eq!(
        std::fs::read_to_string(borrowed.join("a.txt")).unwrap(),
        "a",
        "a borrowed clone must still have real working files"
    );
}

#[test]
fn a_plain_workspace_has_nothing_to_borrow_from() {
    let env = TestEnv::new("share_none");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare, ""));

    let out = env.run(&["clone"]);
    assert!(out.success, "{:?}", out.stderr);
    assert_eq!(
        alternates_of(&env.playground.join("libs/core")),
        None,
        "with no source workspace the clone should be ordinary"
    );
}

// ---------------------------------------------------------------------------
// Discovery — a workspace cloned with --reference
// ---------------------------------------------------------------------------

#[test]
fn a_referenced_workspace_borrows_for_its_children_too() {
    let env = TestEnv::new("share_alternates");
    populated_workspace(&env, "");

    // The workspace itself was cloned with --reference; its children should
    // follow it to the same source. Nested so it is cleaned up with the env.
    let borrower = env.playground.join("borrower");
    git_isolated(
        &env.playground,
        &[
            "clone",
            "--reference",
            env.playground.to_str().unwrap(),
            env.playground.to_str().unwrap(),
            borrower.to_str().unwrap(),
        ],
    );
    assert!(
        alternates_of(&borrower).is_some(),
        "precondition: the workspace clone itself should have an alternate"
    );

    let out = clone_into(&borrower);
    assert!(
        out.success,
        "clone in the borrower failed: {:?}",
        out.stderr
    );

    let alternates =
        alternates_of(&borrower.join("libs/core")).expect("the child should borrow too");
    assert_eq!(
        PathBuf::from(&alternates).canonicalize().unwrap(),
        env.playground
            .join("libs/core/.git/objects")
            .canonicalize()
            .unwrap(),
        "the child should borrow from the same workspace its parent did"
    );
}

// ---------------------------------------------------------------------------
// dissociate
// ---------------------------------------------------------------------------

#[test]
fn dissociate_keeps_the_speed_and_drops_the_link() {
    let env = TestEnv::new("share_dissociate");
    populated_workspace(&env, "[share]\ndissociate = true\n\n");
    let wt = add_worktree(&env, "wt");

    let out = clone_into(&wt);
    assert!(
        out.success,
        "clone in the worktree failed: {:?}",
        out.stderr
    );

    // Absence of an alternates file is also what a completely unshared clone
    // looks like, so pin down that a source really was found and really was
    // told to dissociate — otherwise this test passes on a broken feature.
    let source = gitscale::share::source_workspace(&wt).expect("the source should be discovered");
    let config = gitscale::config::load_config(&wt.join(".gitscale.toml")).unwrap();
    let reference =
        gitscale::share::reference_for(&source, &config.repos[0], config.share.dissociate)
            .expect("the source copy should be a valid candidate");
    assert!(
        reference.dissociate,
        "config should have turned dissociate on"
    );

    let borrowed = wt.join("libs/core");
    assert_eq!(
        alternates_of(&borrowed),
        None,
        "dissociate should leave no alternates file behind"
    );
    assert_eq!(
        std::fs::read_to_string(borrowed.join("a.txt")).unwrap(),
        "a"
    );
}

#[test]
fn a_dissociated_clone_survives_losing_its_source() {
    let env = TestEnv::new("share_dissociate_survives");
    populated_workspace(&env, "[share]\ndissociate = true\n\n");
    let wt = add_worktree(&env, "wt");
    assert!(clone_into(&wt).success);

    // The whole point of the option: deleting what it was cloned from must
    // not cost it its history.
    std::fs::remove_dir_all(env.playground.join("libs/core")).unwrap();

    let log = std::process::Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(wt.join("libs/core"))
        .output()
        .unwrap();
    assert!(
        log.status.success(),
        "a dissociated clone should still read its own history: {}",
        String::from_utf8_lossy(&log.stderr)
    );
}

// ---------------------------------------------------------------------------
// Refusing unsuitable candidates
// ---------------------------------------------------------------------------

#[test]
fn a_different_repository_at_the_same_path_is_not_borrowed_from() {
    let env = TestEnv::new("share_url_mismatch");
    populated_workspace(&env, "");
    let other = env.create_bare_repo("other", "main", &[("b.txt", "b")]);

    // Same relative path, different project. Nothing about the layout says so
    // — only the remote does.
    git_isolated(
        &env.playground.join("libs/core"),
        &["remote", "set-url", "origin", other.to_str().unwrap()],
    );

    let wt = add_worktree(&env, "wt");
    let out = clone_into(&wt);
    assert!(
        out.success,
        "clone should fall back, not fail: {:?}",
        out.stderr
    );
    assert_eq!(
        alternates_of(&wt.join("libs/core")),
        None,
        "a path holding an unrelated repository must not be borrowed from"
    );
}

#[test]
fn a_symlinked_checkout_is_not_borrowed_from() {
    let env = TestEnv::new("share_symlink");
    populated_workspace(&env, "");

    // What `resolve` leaves behind when a recursive dependency is deduped:
    // the path is a link to a checkout declared elsewhere, not one itself.
    let real = env.playground.join("core-actual");
    std::fs::rename(env.playground.join("libs/core"), &real).unwrap();
    std::os::unix::fs::symlink("../core-actual", env.playground.join("libs/core")).unwrap();

    let wt = add_worktree(&env, "wt");
    let out = clone_into(&wt);
    assert!(
        out.success,
        "clone should fall back, not fail: {:?}",
        out.stderr
    );
    assert_eq!(
        alternates_of(&wt.join("libs/core")),
        None,
        "a symlinked path is not a checkout to borrow from"
    );
}

#[test]
fn a_missing_checkout_in_the_source_is_skipped() {
    let env = TestEnv::new("share_absent");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare, ""));
    env.init_playground_git();
    // Deliberately not cloned in the source workspace: the worktree is the
    // first place this entry has ever been checked out.
    let wt = add_worktree(&env, "wt");

    let out = clone_into(&wt);
    assert!(out.success, "clone failed: {:?}", out.stderr);
    assert_eq!(
        alternates_of(&wt.join("libs/core")),
        None,
        "there is nothing on disk to borrow from"
    );
    assert!(
        wt.join("libs/core/a.txt").is_file(),
        "it should still clone"
    );
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[test]
fn dissociate_defaults_to_off() {
    let env = TestEnv::new("share_default");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare, ""));
    let config = gitscale::config::load_config(&env.playground.join(".gitscale.toml")).unwrap();
    assert!(!config.share.dissociate);
}

/// `add` and `remove` rewrite the whole file, so every table has to survive
/// the round trip — the alternative is silently deleting a user's config.
#[test]
fn editing_the_config_preserves_every_table() {
    let env = TestEnv::new("share_rewrite_all");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[share]\ndissociate = true\n\n\
         [storage]\nurl = \"https://example.com/artefacts\"\n\n\
         [hooks]\npost_sync = \"echo hi\"\non_pull_error = \"fail\"\n\n\
         [repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\", mode = \"readonly\" }}\n",
        bare.display()
    ));

    let out = env.run(&["add", "libs/extra", "https://example.com/extra.git", "main"]);
    assert!(out.success, "{:?}", out.stderr);

    let config = gitscale::config::load_config(&env.playground.join(".gitscale.toml")).unwrap();
    assert!(config.share.dissociate, "[share] should survive");
    assert_eq!(
        config.storage_url, "https://example.com/artefacts",
        "[storage] should survive"
    );
    assert_eq!(
        config.hooks.post_sync.as_deref(),
        Some("echo hi"),
        "[hooks].post_sync should survive"
    );
    assert_eq!(
        config.hooks.on_pull_error,
        Some(gitscale::config::OnHookError::Fail),
        "[hooks].on_pull_error should survive"
    );
    assert_eq!(config.repos.len(), 2);
    assert_eq!(config.repos[0].mode, gitscale::config::RepoMode::Readonly);
}

/// A `post_sync` command is an arbitrary shell line. Written back unescaped it
/// would end the TOML string early and take the rest of the config with it.
#[test]
fn a_rewrite_escapes_values_that_would_break_the_file() {
    let env = TestEnv::new("share_rewrite_escape");
    let awkward = r#"echo "it's \ fine" && touch x"#;
    let mut cfg = String::from("[hooks]\npost_sync = ");
    cfg.push_str(&format!("{:?}", awkward)); // Rust's debug quoting is TOML-compatible here
    cfg.push('\n');
    env.write_config(&cfg);

    // Precondition: it parses before we touch it.
    let before = gitscale::config::load_config(&env.playground.join(".gitscale.toml")).unwrap();
    assert_eq!(before.hooks.post_sync.as_deref(), Some(awkward));

    let out = env.run(&["add", "libs/extra", "https://example.com/extra.git", "main"]);
    assert!(out.success, "{:?}", out.stderr);

    let after = gitscale::config::load_config(&env.playground.join(".gitscale.toml"))
        .expect("the rewritten config must still parse");
    assert_eq!(
        after.hooks.post_sync.as_deref(),
        Some(awkward),
        "the command should come back byte for byte"
    );
}

#[test]
fn editing_the_config_preserves_the_share_table() {
    let env = TestEnv::new("share_rewrite");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare, "[share]\ndissociate = true\n\n"));

    // `add` rewrites the whole file; the option must not be dropped on the way
    // through.
    let out = env.run(&["add", "libs/extra", "https://example.com/extra.git", "main"]);
    assert!(out.success, "{:?}", out.stderr);

    let config = gitscale::config::load_config(&env.playground.join(".gitscale.toml")).unwrap();
    assert!(
        config.share.dissociate,
        "share.dissociate should survive a config rewrite"
    );
    assert_eq!(config.repos.len(), 2);
}

// ---------------------------------------------------------------------------
// The path a hook actually takes
// ---------------------------------------------------------------------------

#[test]
fn a_pull_into_an_empty_worktree_borrows_too() {
    let env = TestEnv::new("share_pull");
    populated_workspace(&env, "");
    let wt = add_worktree(&env, "wt");

    // With hooks installed, `git worktree add` fires post-checkout and the
    // first gitscale command to touch the new worktree is `pull`, not
    // `clone` — so that path has to reach the same source.
    let out = gitscale::run_cli_with(&["gitscale", "pull", "-C", wt.to_str().unwrap()], false);
    assert!(out.success, "pull in the worktree failed: {:?}", out.stderr);

    let alternates = alternates_of(&wt.join("libs/core"))
        .expect("a pull that has to clone first should borrow as well");
    assert_eq!(
        PathBuf::from(&alternates).canonicalize().unwrap(),
        env.playground
            .join("libs/core/.git/objects")
            .canonicalize()
            .unwrap()
    );
}
