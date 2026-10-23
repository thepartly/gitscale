//! A topic's state taken from its joined checkouts: a root with no commits
//! of its own — a workspace repository that only declares the others — is
//! merged once each joined checkout is merged into the branch its pin
//! follows, and `finish` takes such a checkout back to its pin without a
//! release. A checkout pinned to a release still waits for `git upgrade`.

use crate::support::worktrees::*;
use crate::support::{run_git_pub, strip_ansi, TestEnv};
use std::path::Path;

/// Squash `branch` of `bare` into `into`, as a merge request would, and
/// delete the branch, as the hosting service does on merge.
fn squash_merge_and_delete(env: &TestEnv, bare: &Path, branch: &str, into: &str) {
    let work = env.repos_remote.join("merge-tmp");
    let _ = std::fs::remove_dir_all(&work);
    run_git_pub(
        &env.repos_remote,
        &[
            "clone",
            "-q",
            "-b",
            into,
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
    run_git_pub(&work, &["push", "-q", "origin", into]);
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

fn list_row(ws: &Path, topic: &str, fetch: bool) -> String {
    let mut args = vec!["topic", "list"];
    if fetch {
        args.push("--fetch");
    }
    let out = gs(ws, &args);
    ok(&out);
    out.stdout
        .lines()
        .find(|l| l.contains(topic))
        .unwrap_or_default()
        .to_string()
}

fn status_text(dir: &Path) -> String {
    let out = gs(dir, &["topic", "status"]);
    ok(&out);
    strip_ansi(&out.stdout)
}

fn root_row(text: &str) -> String {
    text.lines()
        .find(|l| l.trim_start().starts_with(". "))
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// A workspace task over two repositories, one following its default branch
/// with no revision and one pinned to `main`: `not pushed`, then `pushed`,
/// then — once both are squash-merged and their branches deleted — `merged`,
/// and `finish` removes the worktree without `--force`.
#[test]
fn normal_001_a_workspace_task_is_merged_when_its_joined_repositories_are() {
    let env = TestEnv::new("state_workspace_task");
    let app_repo = env.create_bare_repo("app", "main", &[("app.txt", "v1")]);
    let docs = env.create_bare_repo("docs", "main", &[("docs.txt", "v1")]);
    let config = format!(
        "[repos]\n\
         \"repos/app\" = {{ url = \"{}\" }}\n\
         \"repos/docs\" = {{ url = \"{}\", revision = \"main\" }}\n",
        app_repo.display(),
        docs.display()
    );
    let root = env.create_bare_repo("workspace", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    bare_clone(&root, &ws);
    ok(&gs(&ws, &["topic", "switch", "main"]));

    ok(&gs(&ws, &["topic", "start", "PROJ-7"]));
    let task = ws.join("PROJ-7");
    ok(&gs(&task, &["topic", "join", "repos/app", "repos/docs"]));
    commit_in(&task.join("repos/app"), "app.txt", "v2");
    commit_in(&task.join("repos/docs"), "docs.txt", "v2");
    let row = list_row(&ws, "PROJ-7", false);
    assert!(row.ends_with("2 not pushed"), "{}", row);
    for repo in ["repos/app", "repos/docs"] {
        run_git_pub(&task.join(repo), &["push", "-q", "-u", "origin", "PROJ-7"]);
    }
    let row = list_row(&ws, "PROJ-7", false);
    assert!(row.ends_with(" pushed"), "{}", row);

    // One merged: not yet, and finish names the other.
    squash_merge_and_delete(&env, &app_repo, "PROJ-7", "main");
    let row = list_row(&ws, "PROJ-7", true);
    assert!(row.ends_with(" pushed"), "{}", row);
    let out = gs(&task, &["topic", "finish"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "repos/docs is still on the topic, not merged: merge it, or git topic leave repos/docs"
        ),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains("repos/app is still"), "{}", out.stderr);
    assert!(task.exists());

    squash_merge_and_delete(&env, &docs, "PROJ-7", "main");
    let row = list_row(&ws, "PROJ-7", true);
    assert!(row.ends_with("merged"), "{}", row);
    let text = status_text(&task);
    assert!(root_row(&text).ends_with("merged into main"), "{}", text);
    assert!(text.contains("next: git topic finish"), "{}", text);

    let out = gs(&task, &["topic", "finish"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!task.exists());
    assert!(!list_row(&ws, "PROJ-7", false).contains("PROJ-7"));
}

/// In a plain clone, a child that follows `main` and is merged there goes
/// back to its pin when the topic finishes: on `main` again, detached.
#[test]
fn normal_002_finish_takes_a_merged_child_that_follows_a_branch_back_to_it() {
    let f = fixture("state_finish_plain", "");
    let docs = f
        .env
        .create_bare_repo("docs", "main", &[("docs.txt", "v1")]);
    let ws = f.clone_root("ws");
    let config = std::fs::read_to_string(ws.join(".gitscale.toml")).unwrap();
    std::fs::write(
        ws.join(".gitscale.toml"),
        format!(
            "{}\"imports/docs\" = {{ url = \"{}\", revision = \"main\" }}\n",
            config,
            docs.display()
        ),
    )
    .unwrap();
    run_git_pub(&ws, &["commit", "-q", "-am", "docs"]);
    run_git_pub(&ws, &["push", "-q", "origin", "HEAD"]);
    ok(&gs(&ws, &["sync"]));
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/docs"]));
    let dest = ws.join("imports/docs");
    commit_in(&dest, "docs.txt", "v2");
    run_git_pub(&dest, &["push", "-q", "-u", "origin", "feat"]);
    squash_merge_and_delete(&f.env, &docs, "feat", "main");

    let out = gs(&ws, &["topic", "finish"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(branch(&ws).as_deref(), Some("main"));
    assert_eq!(branch(&dest), None);
    assert_eq!(head(&dest), git(&docs, &["rev-parse", "main"]));
    assert!(!has_ref(&store_for(&ws, &docs), "refs/heads/feat"));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A child pinned to a release, squash-merged and its branch deleted, is not
/// merged until `git upgrade` promotes it: `list` does not say `merged`,
/// `status` says it waits and that nothing is left to push, and `finish`
/// refuses it.
#[test]
fn edge_003_a_child_pinned_to_a_release_waits_for_upgrade() {
    let f = fixture("state_release_child", "");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    commit_in(&ws.join("imports/core"), "lib.txt", "change");
    ok(&gs(&ws, &["push", "--quiet"]));
    squash_merge_and_delete(&f.env, &f.core, "feat", "main");

    let row = list_row(&ws, "feat", true);
    assert!(!row.ends_with("merged"), "{}", row);
    assert!(row.ends_with(" pushed"), "{}", row);
    let text = status_text(&ws);
    assert!(
        root_row(&text).contains("waits on imports/core"),
        "{}",
        text
    );
    assert!(!text.contains("git scale push"), "{}", text);
    let out = gs(&ws, &["topic", "finish"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "imports/core is still on the topic: git upgrade, or git topic leave imports/core"
        ),
        "{}",
        out.stderr
    );
}

/// A child pinned to `develop`, not its default branch, is merged once
/// `develop` holds its change: `list` says `merged`, and `finish` loses
/// nothing by dropping its branch, though `main` lacks the change.
#[test]
fn edge_004_a_child_following_another_branch_is_judged_against_it() {
    let env = TestEnv::new("state_develop_child");
    let lib = env.create_bare_repo("lib", "main", &[("lib.txt", "v1")]);
    run_git_pub(&lib, &["branch", "develop", "main"]);
    let config = format!(
        "[repos]\n\"repos/lib\" = {{ url = \"{}\", revision = \"develop\" }}\n",
        lib.display()
    );
    let root = env.create_bare_repo("workspace", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    bare_clone(&root, &ws);
    ok(&gs(&ws, &["topic", "start", "PROJ-8"]));
    let task = ws.join("PROJ-8");
    ok(&gs(&task, &["topic", "join", "repos/lib"]));
    commit_in(&task.join("repos/lib"), "lib.txt", "v2");
    run_git_pub(
        &task.join("repos/lib"),
        &["push", "-q", "-u", "origin", "PROJ-8"],
    );
    squash_merge_and_delete(&env, &lib, "PROJ-8", "develop");

    let row = list_row(&ws, "PROJ-8", true);
    assert!(row.ends_with("merged"), "{}", row);
    let text = status_text(&task);
    assert!(text.contains("merged into develop"), "{}", text);
    let out = gs(&task, &["topic", "finish"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("dropped"), "{}", out.stdout);
    assert!(!task.exists());
}

/// A root whose own commits were merged with a merge commit — so that, as
/// after a fast-forward, the default branch has all of them — is merged
/// though a checkout joined to it has no change.
#[test]
fn edge_005_a_root_merged_with_a_merge_commit_is_merged() {
    let f = fixture("state_root_no_ff", "");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    commit_in(&ws, "README.md", "root change");
    run_git_pub(&ws, &["push", "-q", "-u", "origin", "feat"]);
    let work = f.env.repos_remote.join("merge-tmp");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    identity(&work);
    run_git_pub(
        &work,
        &["merge", "-q", "--no-ff", "-m", "merge feat", "origin/feat"],
    );
    run_git_pub(&work, &["push", "-q", "origin", "main"]);

    let row = list_row(&ws, "feat", true);
    assert!(row.ends_with("merged"), "{}", row);
    let text = status_text(&ws);
    assert!(root_row(&text).ends_with("merged into main"), "{}", text);
}

/// Two majors of one repository on the topic, `feat` and `feat@v1`, are two
/// joined checkouts, each judged on its own branch.
#[test]
fn edge_006_two_majors_are_two_joined_checkouts() {
    let env = TestEnv::new("state_two_majors");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    env.push_commit(&d, "main", "d.txt", "v2");
    let v2 = git(&d, &["rev-parse", "main"]);
    run_git_pub(&d, &["tag", "v2.0.0", &v2]);
    let config = format!(
        "[repos]\n\"imports/d\" = {{ url = \"{0}\", revision = \"v2.0.0\" }}\n\
         \"imports/d_v1\" = {{ url = \"{0}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let root = env.create_bare_repo("workspace", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    bare_clone(&root, &ws);
    ok(&gs(&ws, &["topic", "start", "PROJ-9"]));
    let task = ws.join("PROJ-9");
    ok(&gs(&task, &["topic", "join", "imports/d", "imports/d_v1"]));
    commit_in(&task.join("imports/d"), "d.txt", "v2 fix");
    commit_in(&task.join("imports/d_v1"), "d.txt", "v1 fix");
    let row = list_row(&ws, "PROJ-9", false);
    assert!(row.ends_with("2 not pushed"), "{}", row);
    run_git_pub(
        &task.join("imports/d"),
        &["push", "-q", "-u", "origin", "PROJ-9"],
    );
    run_git_pub(
        &task.join("imports/d_v1"),
        &["push", "-q", "-u", "origin", "PROJ-9@v1"],
    );
    let row = list_row(&ws, "PROJ-9", false);
    assert!(row.ends_with(" pushed"), "{}", row);
}
