//! Lazy entries: a dependency declared `lazy = true`, or preferred lazy by
//! the workspace, is left out of every placement until it is on disk, named,
//! or on the topic; `prefer --eager` keeps one placed whatever the configs
//! say.

use crate::support::worktrees::*;
use crate::support::{run_git_pub, TestEnv};
use std::path::{Path, PathBuf};

/// `core` pinned at v1.0.0, and `fixtures` declared lazy at `main`.
fn lazy_fixture(name: &str) -> (Fixture, PathBuf) {
    let env = TestEnv::new(name);
    let core = env.create_bare_repo("core", "main", &[("lib.txt", "v1")]);
    run_git_pub(&core, &["tag", "v1.0.0", "main"]);
    let fixtures = env.create_bare_repo("fixtures", "main", &[("data.txt", "d1")]);
    let config = format!(
        "[repos]\n\
         \"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n\
         \"imports/fixtures\" = {{ url = \"{}\", revision = \"main\", lazy = true }}\n",
        core.display(),
        fixtures.display()
    );
    let root = env.create_bare_repo(
        "root",
        "main",
        &[("README.md", "root"), (".gitscale.toml", &config)],
    );
    (Fixture { env, core, root }, fixtures)
}

/// Squash `branch` of `bare` into its `main`, as a merge request would, and
/// delete the branch, as the hosting service does on merge.
fn squash_merge_and_delete(env: &TestEnv, bare: &Path, branch: &str) {
    let work = env.repos_remote.join("merge-tmp");
    let _ = std::fs::remove_dir_all(&work);
    run_git_pub(
        &env.repos_remote,
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    identity(&work);
    run_git_pub(
        &work,
        &["merge", "-q", "--squash", &format!("origin/{}", branch)],
    );
    run_git_pub(
        &work,
        &["commit", "-q", "-m", &format!("squashed {}", branch)],
    );
    run_git_pub(&work, &["push", "-q", "origin", "main"]);
    run_git_pub(&work, &["push", "-q", "origin", "--delete", branch]);
    let _ = std::fs::remove_dir_all(&work);
}

/// A bare clone of `bare` at `parent/.git`, as `git clone --bare` makes one.
fn bare_clone(bare: &Path, parent: &Path) {
    std::fs::create_dir_all(parent).unwrap();
    run_git_pub(
        parent,
        &["clone", "-q", "--bare", bare.to_str().unwrap(), ".git"],
    );
    identity(parent);
}

fn ls_row(ws: &Path, repo: &str) -> String {
    status_row(ws, repo)
}

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// A sync leaves an absent lazy entry out, without even fetching it, and
/// `ls` shows it as `lazy` rather than `missed`.
#[test]
fn normal_001_sync_leaves_an_absent_lazy_entry_out() {
    let (f, fixtures) = lazy_fixture("lazy_sync_skips");
    let ws = f.clone_root("ws");
    assert!(ws.join("imports/core/lib.txt").is_file());
    assert!(!ws.join("imports/fixtures").exists());
    assert!(!store_for(&ws, &fixtures).exists());

    let row = ls_row(&ws, "imports/fixtures");
    assert!(row.starts_with("○"), "{}", row);
    assert!(row.contains(" lazy"), "{}", row);
    assert!(!row.contains("missed"), "{}", row);
    let json = gs(&ws, &["ls", "--format", "json"]);
    ok(&json);
    let rows: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    let lazy = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "imports/fixtures")
        .unwrap();
    assert_eq!(lazy["lazy"], true);
    assert_eq!(lazy["exists"], false);
    let core = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "imports/core")
        .unwrap();
    assert_eq!(core["lazy"], false);
}

/// Named, it is checked out; from then on a plain sync keeps it on its pin,
/// and deleting the directory makes it lazy again.
#[test]
fn normal_002_naming_it_places_it_and_then_it_is_kept() {
    let (f, fixtures) = lazy_fixture("lazy_sync_named");
    let ws = f.clone_root("ws");
    let out = gs(&ws, &["sync", "imports/fixtures"]);
    ok(&out);
    let dest = ws.join("imports/fixtures");
    assert_eq!(
        std::fs::read_to_string(dest.join("data.txt")).unwrap(),
        "d1"
    );
    let row = ls_row(&ws, "imports/fixtures");
    assert!(row.contains(" ok"), "{}", row);

    let tip = f.env.push_commit(&fixtures, "main", "data.txt", "d2");
    ok(&gs(&ws, &["sync"]));
    assert_eq!(head(&dest), tip);

    make_writable_tree(&dest);
    std::fs::remove_dir_all(&dest).unwrap();
    ok(&gs(&ws, &["sync"]));
    assert!(!dest.exists());
    assert!(ls_row(&ws, "imports/fixtures").contains(" lazy"));
}

/// A lazy entry of a dependency's own config, which the root does not
/// declare, is left out as well: the root does not inherit it.
#[test]
fn normal_003_a_dependencys_lazy_entry_is_left_out() {
    let env = TestEnv::new("lazy_child_entry");
    let fixtures = env.create_bare_repo("fixtures", "main", &[("data.txt", "d1")]);
    let lib = env.create_bare_repo(
        "lib",
        "main",
        &[
            ("lib.txt", "lib"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"test/fixtures\" = {{ url = \"{}\", revision = \"main\", lazy = true }}\n",
                    fixtures.display()
                ),
            ),
        ],
    );
    env.write_config(&format!(
        "[resolve]\nallow = [\"{}/*\"]\n\n[repos]\n\"imports/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        env.repos_remote.display(),
        lib.display(),
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/lib/lib.txt").is_file());
    assert!(!env.playground.join("imports/fixtures").exists());
    assert!(!env.playground.join("imports/lib/test/fixtures").exists());

    // Named by its hoisted directory, it is placed and linked.
    let out = env.run(&["sync", "imports/fixtures"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/fixtures/data.txt").is_file());
    assert!(env
        .playground
        .join("imports/lib/test/fixtures")
        .is_symlink());
}

/// `require --lazy` writes the flag, and checks nothing out.
#[test]
fn normal_004_require_lazy_writes_the_flag_and_fetches_nothing() {
    let env = TestEnv::new("lazy_require");
    let fixtures = env.create_bare_repo("fixtures", "main", &[("data.txt", "d1")]);
    env.init_playground_git();
    let out = env.run(&[
        "require",
        "--lazy",
        "imports/fixtures",
        fixtures.to_str().unwrap(),
        "main",
    ]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains(", lazy"), "{}", out.stdout);
    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    assert!(config.contains("lazy = true"), "{}", config);
    assert!(!env.playground.join("imports/fixtures").exists());
}

/// `git topic join` places an absent lazy entry at its pin and puts it on
/// the topic, writable.
#[test]
fn normal_005_join_places_a_lazy_entry_on_the_topic() {
    let (f, fixtures) = lazy_fixture("lazy_join");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    assert!(!ws.join("imports/fixtures").exists());

    let out = gs(&ws, &["topic", "join", "imports/fixtures"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/fixtures on feat, from main"),
        "{}",
        out.stdout
    );
    let dest = ws.join("imports/fixtures");
    assert_eq!(branch(&dest).as_deref(), Some("feat"));
    assert_eq!(head(&dest), git(&fixtures, &["rev-parse", "main"]));
    assert!(writable(&dest.join("data.txt")));
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&dest).as_deref(), Some("feat"));
}

/// A worktree of the topic made again — after the first was deleted by hand
/// — places the lazy entry the topic joined: its store has the branch.
#[test]
fn normal_006_a_joined_lazy_entry_comes_back_with_its_topic() {
    let (f, _) = lazy_fixture("lazy_rejoin");
    let app = f.env.repos_remote.join("app");
    bare_clone(&f.root, &app);
    ok(&gs(&app, &["topic", "switch", "main"]));
    ok(&gs(&app, &["topic", "start", "feat"]));
    let wt = app.join("feat");
    ok(&gs(&wt, &["topic", "join", "imports/fixtures"]));
    let work = commit_in(&wt.join("imports/fixtures"), "data.txt", "mine");
    assert!(!app.join("main/imports/fixtures").exists());

    make_writable_tree(&wt);
    std::fs::remove_dir_all(&wt).unwrap();
    run_git_pub(&app, &["worktree", "prune"]);
    ok(&gs(&app, &["topic", "switch", "feat"]));
    let dest = wt.join("imports/fixtures");
    assert_eq!(branch(&dest).as_deref(), Some("feat"));
    assert_eq!(head(&dest), work);
    // The main worktree still leaves it out.
    ok(&gs(&app.join("main"), &["sync"]));
    assert!(!app.join("main/imports/fixtures").exists());
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A topic branch its remote has brings an absent lazy entry in, on its
/// branch — on a developer machine and in CI alike — while a workspace off
/// the topic still leaves it out.
#[test]
fn edge_008_a_remote_topic_branch_places_an_absent_lazy_entry() {
    let (f, fixtures) = lazy_fixture("lazy_remote_topic");
    run_git_pub(&fixtures, &["branch", "feat", "main"]);
    let tip = f.env.push_commit(&fixtures, "feat", "data.txt", "on feat");
    let ws = f.clone_root("ws");
    assert!(!ws.join("imports/fixtures").exists());
    run_git_pub(&ws, &["switch", "-q", "-c", "feat"]);
    ok(&gs(&ws, &["sync"]));
    assert_eq!(head(&ws.join("imports/fixtures")), tip);

    let job = f.env.repos_remote.join("job");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            job.to_str().unwrap(),
        ],
    );
    run_git_pub(&job, &["checkout", "-q", "--detach"]);
    ok(&ci_job(&job, &f.env.cache, "main", &["sync"]));
    assert!(job.join("imports/core/lib.txt").is_file());
    assert!(!job.join("imports/fixtures").exists());
    ok(&ci_job(&job, &f.env.cache, "feat", &["sync"]));
    assert_eq!(head(&job.join("imports/fixtures")), tip);
}

/// `prefer --lazy` leaves an entry its config does not call lazy out, and
/// removes its checkout at once when that loses nothing; `ls` says the
/// workspace chose it, and `prefer` lists both axes.
#[test]
fn normal_011_prefer_lazy_leaves_an_entry_out() {
    let (f, _) = lazy_fixture("lazy_prefer_lazy");
    let ws = f.clone_root("ws");
    let dest = ws.join("imports/core");
    assert!(dest.join("lib.txt").is_file());

    let out = gs(&ws, &["prefer", "--lazy", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core   source, lazy"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("remove  imports/core (lazy)"),
        "{}",
        out.stdout
    );
    assert!(!dest.exists());
    ok(&gs(&ws, &["sync"]));
    assert!(!dest.exists());

    let json = gs(&ws, &["ls", "--format", "json"]);
    ok(&json);
    let rows: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    let row = |dir: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["directory"] == dir)
            .unwrap()
            .clone()
    };
    assert_eq!(row("imports/core")["lazy"], true);
    assert_eq!(row("imports/core")["lazy_reason"], "preferred");
    assert_eq!(row("imports/core")["as"], "source");
    assert_eq!(row("imports/fixtures")["lazy_reason"], "committed");

    let listed = gs(&ws, &["prefer"]);
    ok(&listed);
    assert!(
        listed.stdout.contains("imports/core   source, lazy"),
        "{}",
        listed.stdout
    );
    let one = gs(&ws, &["prefer", "imports/fixtures"]);
    assert!(
        one.stdout
            .contains("imports/fixtures   source, lazy (committed)"),
        "{}",
        one.stdout
    );
}

/// `prefer --eager` places a lazy entry on a plain sync, and `--unset`
/// hands it back to the config, which makes it lazy — and removes it — again.
#[test]
fn normal_012_prefer_eager_overrides_a_committed_lazy() {
    let (f, _) = lazy_fixture("lazy_prefer_eager");
    let ws = f.clone_root("ws");
    let dest = ws.join("imports/fixtures");
    ok(&gs(&ws, &["prefer", "--eager", "imports/fixtures"]));
    ok(&gs(&ws, &["sync"]));
    assert!(dest.join("data.txt").is_file());
    assert!(ls_row(&ws, "imports/fixtures").contains(" ok"));

    let out = gs(&ws, &["prefer", "--unset", "imports/fixtures"]);
    ok(&out);
    assert!(!dest.exists(), "{}", out.stdout);
    assert!(ls_row(&ws, "imports/fixtures").contains(" lazy"));
}

/// `prefer --unset` removes both kinds of preference — form and presence —
/// and takes no other preference beside it.
#[test]
fn normal_025_prefer_unset_removes_form_and_presence() {
    let env = TestEnv::new("lazy_prefer_unset");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, gitscale::prefer::Form::Artefact);
    ok(&gs(&ws, &["prefer", "--eager", "meta/app"]));
    let out = gs(&ws, &["prefer", "meta/app"]);
    ok(&out);
    assert!(
        out.stdout.contains("meta/app   artefact, eager"),
        "{}",
        out.stdout
    );

    ok(&gs(&ws, &["prefer", "--unset", "meta/app"]));
    let out = gs(&ws, &["prefer"]);
    ok(&out);
    assert!(
        out.stdout.contains("Every checkout takes its sources."),
        "{}",
        out.stdout
    );

    let out = gs(&ws, &["prefer", "--unset", "--lazy", "meta/app"]);
    assert!(!out.success, "{}", out.stdout);
    let out = gs(&ws, &["prefer", "--unset", "--artefact", "meta/app"]);
    assert!(!out.success, "{}", out.stdout);
    let out = gs(&ws, &["prefer", "--unset"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("git scale prefer --unset <DIR>"),
        "{}",
        out.stderr
    );
}

/// Made lazy while it holds uncommitted work, a checkout stays, and says why.
#[test]
fn edge_013_prefer_lazy_keeps_a_modified_checkout() {
    let (f, _) = lazy_fixture("lazy_prefer_dirty");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["prefer", "--eager", "imports/fixtures"]));
    ok(&gs(&ws, &["sync"]));
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/fixtures"]));
    ok(&gs(&ws, &["topic", "leave", "imports/fixtures"]));
    let dest = ws.join("imports/fixtures");
    make_writable_tree(&dest);
    std::fs::write(dest.join("data.txt"), "unsaved").unwrap();
    let out = gs(&ws, &["prefer", "--lazy", "imports/fixtures"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("kept  imports/fixtures (1 change uncommitted: commit and push it"),
        "{}",
        out.stdout
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("data.txt")).unwrap(),
        "unsaved"
    );
}

/// A lazy entry joined from nothing and taken off the topic, by `leave` or
/// by `finish`, goes back to nothing; its commits stay in its store.
#[test]
fn normal_014_leave_and_finish_take_a_lazy_entry_away() {
    let (f, fixtures) = lazy_fixture("lazy_leave_finish");
    let ws = f.clone_root("ws");
    let dest = ws.join("imports/fixtures");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/fixtures"]));
    let out = gs(&ws, &["topic", "leave", "imports/fixtures"]);
    ok(&out);
    assert!(out.stdout.contains("removed, lazy"), "{}", out.stdout);
    assert!(!dest.exists());

    ok(&gs(&ws, &["topic", "join", "imports/fixtures"]));
    commit_in(&dest, "data.txt", "d2");
    run_git_pub(&dest, &["push", "-q", "-u", "origin", "feat"]);
    squash_merge_and_delete(&f.env, &fixtures, "feat");
    // `--force` ends it whatever else holds a finish up; what is checked here
    // is that the lazy checkout goes.
    let out = gs(&ws, &["topic", "finish", "--force"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("remove  imports/fixtures (lazy)"),
        "{}",
        out.stdout
    );
    assert!(!dest.exists());
    assert!(ls_row(&ws, "imports/fixtures").contains(" lazy"));
}

/// In CI, on a topic, a lazy repository's remote that cannot be asked fails
/// `check` rather than reading as "no branch", which would let a change in it
/// go unseen.
#[test]
fn edge_015_an_unreachable_lazy_remote_fails_check_in_ci() {
    let (f, fixtures) = lazy_fixture("lazy_unreachable");
    run_git_pub(&fixtures, &["branch", "feat", "main"]);
    f.env.push_commit(&fixtures, "feat", "data.txt", "on feat");
    let job = f.env.repos_remote.join("job");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            job.to_str().unwrap(),
        ],
    );
    run_git_pub(&job, &["checkout", "-q", "--detach"]);
    let out = ci_job(&job, &f.env.cache, "feat", &["check"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("was taken from branch feat"),
        "{}",
        out.stderr
    );

    std::fs::rename(&fixtures, fixtures.with_extension("moved")).unwrap();
    let out = ci_job(&job, &f.env.cache, "feat", &["check"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("cannot ask"), "{}", out.stderr);
}

/// Two repositories asking for one lazy repository under different names:
/// it would be placed under the repository's own name, which is where `ls`
/// shows it and where `sync` places it — and once placed it stays.
#[test]
fn edge_016_a_lazy_repository_two_requesters_name_differently() {
    let env = TestEnv::new("lazy_two_names");
    let fixtures = env.create_bare_repo("fixtures", "main", &[("data.txt", "d1")]);
    let lib = |name: &str, dir: &str| {
        env.create_bare_repo(
            name,
            "main",
            &[
                ("lib.txt", name),
                (
                    ".gitscale.toml",
                    &format!(
                        "[repos]\n\"{}\" = {{ url = \"{}\", revision = \"main\", lazy = true }}\n",
                        dir,
                        fixtures.display()
                    ),
                ),
            ],
        )
    };
    let liba = lib("liba", "test/fx-a");
    let libb = lib("libb", "test/fx-b");
    env.write_config(&format!(
        "[resolve]\nallow = [\"{}/*\"]\n\n[repos]\n\"imports/liba\" = {{ url = \"{}\", revision = \"main\" }}\n\"imports/libb\" = {{ url = \"{}\", revision = \"main\" }}\n",
        env.repos_remote.display(),
        liba.display(),
        libb.display(),
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(ls_row(&env.playground, "imports/fixtures").contains(" lazy"));

    let out = env.run(&["sync", "imports/fixtures"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/fixtures/data.txt").is_file());
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/fixtures/data.txt").is_file());
    assert!(env.playground.join("imports/liba/test/fx-a").is_symlink());
}

/// A major's branch of the topic, `<topic>@v<major>`, on the remote wakes a
/// lazy repository as the topic's own does; until a placement puts it on
/// disk, `ls --fetch` says it is missed, not lazy.
#[test]
fn edge_017_a_majors_topic_branch_wakes_a_lazy_repository() {
    let (f, fixtures) = lazy_fixture("lazy_major_branch");
    run_git_pub(&fixtures, &["branch", "feat@v1", "main"]);
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat"]);
    let out = gs(&ws, &["ls", "--fetch"]);
    ok(&out);
    let row = out
        .stdout
        .lines()
        .find(|l| l.contains("imports/fixtures"))
        .unwrap_or_default();
    assert!(row.contains("missed"), "{}", out.stdout);
    ok(&gs(&ws, &["sync"]));
    assert!(ws.join("imports/fixtures/data.txt").is_file());
}

/// `prefer --lazy` leaves a repository another one needs placed: that
/// repository's request without `lazy` wins, and `prefer` and `ls` say so.
#[test]
fn edge_018_prefer_lazy_yields_to_a_repository_that_needs_it() {
    let env = TestEnv::new("lazy_required");
    let fixtures = env.create_bare_repo("fixtures", "main", &[("data.txt", "d1")]);
    let lib = env.create_bare_repo(
        "lib",
        "main",
        &[
            ("lib.txt", "lib"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"test/fixtures\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    fixtures.display()
                ),
            ),
        ],
    );
    env.write_config(&format!(
        "[resolve]\nallow = [\"{}/*\"]\n\n[repos]\n\"imports/lib\" = {{ url = \"{}\", revision = \"main\" }}\n\"imports/fixtures\" = {{ url = \"{}\", revision = \"main\" }}\n",
        env.repos_remote.display(),
        lib.display(),
        fixtures.display(),
    ));
    env.init_playground_git();
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let out = env.run(&["prefer", "--lazy", "imports/fixtures"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("lazy, but placed: imports/lib requires it"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("kept  imports/fixtures"),
        "{}",
        out.stdout
    );
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/fixtures/data.txt").is_file());
    let row = ls_row(&env.playground, "imports/fixtures");
    assert!(row.contains("imports/lib requires it"), "{}", row);
}

/// Lazy and taken as its artefact: `prefer --lazy` takes the image away,
/// a plain `sync` leaves it out, and `sync DIR` installs the image again.
#[test]
fn normal_019_a_lazy_artefact_is_installed_when_named() {
    let env = TestEnv::new("lazy_artefact");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, gitscale::prefer::Form::Artefact);
    ok(&gs(&ws, &["sync"]));
    let dest = ws.join("meta/app");
    assert!(dest.join("dist/app.bin").is_file());

    let out = gs(&ws, &["prefer", "--lazy", "meta/app"]);
    ok(&out);
    assert!(
        out.stdout.contains("remove  meta/app (lazy)"),
        "{}",
        out.stdout
    );
    assert!(!dest.exists());
    ok(&gs(&ws, &["sync"]));
    assert!(!dest.exists());

    ok(&gs(&ws, &["sync", "meta/app"]));
    assert!(!dest.join(".git").exists(), "its image, not its sources");
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "main build"
    );
}

/// A lazy checkout already on disk before it joined stays when it leaves:
/// only one `join` brought from nothing goes back to nothing.
#[test]
fn edge_020_leave_keeps_a_lazy_checkout_that_was_there_before() {
    let (f, _) = lazy_fixture("lazy_leave_keeps");
    let ws = f.clone_root("ws");
    let dest = ws.join("imports/fixtures");
    ok(&gs(&ws, &["sync", "imports/fixtures"]));
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/fixtures"]));
    let out = gs(&ws, &["topic", "leave", "imports/fixtures"]);
    ok(&out);
    assert!(!out.stdout.contains("removed"), "{}", out.stdout);
    assert!(dest.join("data.txt").is_file());
}

/// On a developer machine, a lazy remote that cannot be asked is a warning:
/// the repository is left out and the command carries on. `check` still
/// fails, as it does in CI.
#[test]
fn edge_021_an_unreachable_lazy_remote_is_a_warning_locally() {
    let (f, fixtures) = lazy_fixture("lazy_unreachable_local");
    let ws = f.clone_root("ws");
    std::fs::rename(&fixtures, fixtures.with_extension("moved")).unwrap();
    ok(&gs(&ws, &["topic", "start", "feat"]));
    // The binary itself: the warning is on its stderr.
    let out = gs_bin(&ws, &["sync"], &[]);
    ok(&out);
    assert!(
        out.stderr.contains("cannot ask for branch feat"),
        "{}",
        out.stderr
    );
    assert!(!ws.join("imports/fixtures").exists());
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let out = gs(&ws, &["check"]);
    assert!(!out.success, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stderr.contains("cannot ask"), "{}", out.stderr);
}

/// Joining a lazy repository works with stores fetched before the root's
/// pins moved: a pin to a tag pushed since is fetched, not an error.
#[test]
fn edge_022_join_works_with_a_store_older_than_the_pins() {
    let (f, _) = lazy_fixture("lazy_join_stale");
    let ws = f.clone_root("ws");
    f.env.push_commit(&f.core, "main", "lib.txt", "v1.1");
    run_git_pub(&f.core, &["tag", "v1.1.0", "main"]);
    let config = std::fs::read_to_string(ws.join(".gitscale.toml"))
        .unwrap()
        .replace("v1.0.0", "v1.1.0");
    std::fs::write(ws.join(".gitscale.toml"), config).unwrap();
    run_git_pub(&ws, &["commit", "-q", "-am", "core v1.1.0"]);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat"]);
    let out = gs(&ws, &["topic", "join", "imports/fixtures"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        branch(&ws.join("imports/fixtures")).as_deref(),
        Some("feat")
    );
}

/// A checkout whose git status cannot be read is kept: nothing says its
/// files are safe to delete.
#[test]
fn edge_023_prefer_lazy_keeps_a_checkout_it_cannot_read() {
    let (f, _) = lazy_fixture("lazy_unreadable");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["sync", "imports/fixtures"]));
    let dest = ws.join("imports/fixtures");
    let index = git(&dest, &["rev-parse", "--git-path", "index"]);
    std::fs::write(dest.join(index.trim()), "not an index").unwrap();
    let out = gs(&ws, &["prefer", "--lazy", "imports/fixtures"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("kept  imports/fixtures (its git status cannot be read"),
        "{}",
        out.stdout
    );
    assert!(dest.join("data.txt").is_file());
}

/// Only a checkout of another major, `<name>_v<major>`, keeps a lazy
/// repository placed: a directory that merely starts with its name does not.
#[test]
fn edge_024_a_sibling_with_a_longer_name_does_not_wake_it() {
    let (f, _) = lazy_fixture("lazy_sibling_name");
    let ws = f.clone_root("ws");
    std::fs::create_dir_all(ws.join("imports/fixtures_vendor")).unwrap();
    std::fs::create_dir_all(ws.join("imports/fixtures_v2x")).unwrap();
    ok(&gs(&ws, &["sync"]));
    assert!(!ws.join("imports/fixtures").exists());
    std::fs::create_dir_all(ws.join("imports/fixtures_v2")).unwrap();
    ok(&gs(&ws, &["sync"]));
    assert!(ws.join("imports/fixtures/data.txt").is_file());
}

/// In CI, `sync DIR` places a lazy repository, and the job's later commands
/// keep it; a later job reusing the build directory leaves it out again.
#[test]
fn normal_026_named_in_ci_it_stays_for_the_rest_of_the_job() {
    let (f, _) = lazy_fixture("lazy_ci_named");
    let job = f.env.repos_remote.join("job");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            job.to_str().unwrap(),
        ],
    );
    run_git_pub(&job, &["checkout", "-q", "--detach"]);
    let first = [("CI_JOB_ID", "101")];
    let out = ci_job_with(
        &job,
        &f.env.cache,
        "main",
        &first,
        &["sync", "imports/fixtures"],
    );
    ok(&out);
    let dest = job.join("imports/fixtures");
    assert!(dest.join("data.txt").is_file());

    let out = ci_job_with(&job, &f.env.cache, "main", &first, &["sync"]);
    ok(&out);
    assert!(!out.stdout.contains("no longer needed"), "{}", out.stdout);
    assert!(dest.join("data.txt").is_file());

    let next = [("CI_JOB_ID", "102")];
    ok(&ci_job_with(&job, &f.env.cache, "main", &next, &["sync"]));
    assert!(!dest.exists());
}

/// `git upgrade <dir>` raises a lazy dependency that is not placed: the
/// raise needs its requests and its releases, not a checkout.
#[test]
fn normal_027_upgrade_raises_a_lazy_dependency_left_out() {
    let env = TestEnv::new("lazy_raise");
    let fixtures = env.create_bare_repo("fixtures", "main", &[("data.txt", "d1")]);
    run_git_pub(&fixtures, &["tag", "v1.0.0", "main"]);
    let config = format!(
        "[repos]\n\"imports/fixtures\" = {{ url = \"{}\", revision = \"v1.0.0\", lazy = true }}\n",
        fixtures.display()
    );
    let root = env.create_bare_repo(
        "root",
        "main",
        &[("README.md", "root"), (".gitscale.toml", &config)],
    );
    let newer = env.push_commit(&fixtures, "main", "data.txt", "d2");
    run_git_pub(&fixtures, &["tag", "v1.1.0", &newer]);
    let f = Fixture {
        env,
        core: fixtures.clone(),
        root,
    };
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "bump"]));
    let out = gs(&ws, &["upgrade", "imports/fixtures"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/fixtures   v1.0.0 → v1.1.0"),
        "{}",
        out.stdout
    );
    let text = std::fs::read_to_string(ws.join(".gitscale.toml")).unwrap();
    assert!(text.contains("revision = \"v1.1.0\""), "{}", text);
    assert!(!ws.join("imports/fixtures").exists());
}

fn make_writable_tree(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    for entry in walk(dir) {
        if let Ok(meta) = std::fs::symlink_metadata(&entry) {
            if meta.file_type().is_symlink() {
                continue;
            }
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o200);
            let _ = std::fs::set_permissions(&entry, perms);
        }
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut found = vec![dir.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                found.extend(walk(&path));
            } else {
                found.push(path);
            }
        }
    }
    found
}

/// Run `args` in `dir` and count the round trips git made to `remote`: each
/// `git-upload-pack` started for it, a fetch's or a ref advertisement's.
fn round_trips(f: &Fixture, dir: &Path, args: &[&str], remote: &Path) -> usize {
    let trace = f.env.repos_remote.join(format!("trace-{}", args.join("-")));
    std::fs::create_dir_all(&trace).unwrap();
    let out = gs_bin(dir, args, &[("GIT_TRACE2_EVENT", trace.to_str().unwrap())]);
    ok(&out);
    std::fs::read_dir(&trace)
        .unwrap()
        .flatten()
        .map(|file| {
            std::fs::read_to_string(file.path())
                .unwrap_or_default()
                .lines()
                .filter(|l| {
                    l.contains("\"event\":\"start\"")
                        && l.contains("\"git-upload-pack\"")
                        && l.contains(remote.to_str().unwrap())
                })
                .count()
        })
        .sum()
}

/// `topic start` asks no lazy remote for the new topic's branch: its name is
/// new on the root's remote, so the answer is almost always no, and each
/// question is a round trip to a remote the task may never touch.
#[test]
fn perf_028_topic_start_asks_no_lazy_remote() {
    let (f, fixtures) = lazy_fixture("lazy_start_asks_nothing");
    let ws = f.clone_root("ws");
    let asked = round_trips(&f, &ws, &["topic", "start", "feat"], &fixtures);
    assert_eq!(asked, 0, "topic start asked the lazy remote");
}

/// `topic join` and `leave` place only the checkouts they name, so they ask
/// no other lazy remote for the topic's branch.
#[test]
fn perf_030_join_and_leave_ask_no_other_lazy_remote() {
    let (f, fixtures) = lazy_fixture("lazy_join_asks_nothing");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    let asked = round_trips(&f, &ws, &["topic", "join", "imports/core"], &fixtures);
    assert_eq!(asked, 0, "topic join asked the lazy remote");
    assert_eq!(branch(&ws.join("imports/core")).as_deref(), Some("feat"));
    let asked = round_trips(&f, &ws, &["topic", "leave", "imports/core"], &fixtures);
    assert_eq!(asked, 0, "topic leave asked the lazy remote");
    assert_eq!(branch(&ws.join("imports/core")), None);
}

/// A lazy repository whose store already fetched the topic's branch, for
/// another worktree of the root, joins that branch: asking no remote, the
/// store's refs still say it has one.
#[test]
fn edge_032_join_takes_a_branch_the_shared_store_already_fetched() {
    let (f, fixtures) = lazy_fixture("lazy_join_store_branch");
    run_git_pub(&fixtures, &["branch", "feat", "main"]);
    let tip = f.env.push_commit(&fixtures, "feat", "data.txt", "on feat");
    let app = f.env.repos_remote.join("app");
    bare_clone(&f.root, &app);
    ok(&gs(&app, &["topic", "switch", "main"]));
    ok(&gs(&app, &["topic", "start", "other"]));
    // The fixtures store, shared by every worktree, fetches origin/feat.
    ok(&gs(
        &app.join("other"),
        &["topic", "join", "imports/fixtures"],
    ));
    ok(&gs(&app, &["topic", "start", "feat"]));
    let wt = app.join("feat");

    ok(&gs(&wt, &["topic", "join", "imports/fixtures"]));
    let dest = wt.join("imports/fixtures");
    assert_eq!(branch(&dest).as_deref(), Some("feat"));
    assert_eq!(head(&dest), tip);
}

/// `finish` fetches a store on the topic once, though both its check of the
/// children and its check of the remote branches read it.
#[test]
fn perf_031_finish_fetches_each_store_once() {
    let (f, fixtures) = lazy_fixture("lazy_finish_fetches_once");
    let app = f.env.repos_remote.join("app");
    bare_clone(&f.root, &app);
    ok(&gs(&app, &["topic", "switch", "main"]));
    ok(&gs(&app, &["topic", "start", "feat"]));
    let wt = app.join("feat");
    ok(&gs(&wt, &["topic", "join", "imports/fixtures"]));
    let fetches = round_trips(&f, &wt, &["topic", "finish", "--force"], &fixtures);
    assert!(!wt.exists());
    assert_eq!(fetches, 1, "finish fetched the store more than once");
}

/// A lazy repository whose remote already has the topic's branch, though the
/// root's remote does not, stays out at `topic start`; joining it checks out
/// that branch, not a new one from its pin.
#[test]
fn edge_029_join_takes_the_branch_a_lazy_remote_already_has() {
    let (f, fixtures) = lazy_fixture("lazy_join_remote_branch");
    run_git_pub(&fixtures, &["branch", "feat", "main"]);
    let tip = f.env.push_commit(&fixtures, "feat", "data.txt", "on feat");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    assert!(!ws.join("imports/fixtures").exists());

    ok(&gs(&ws, &["topic", "join", "imports/fixtures"]));
    let dest = ws.join("imports/fixtures");
    assert_eq!(branch(&dest).as_deref(), Some("feat"));
    assert_eq!(head(&dest), tip);
}
