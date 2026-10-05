//! Finding the workspace: the root above the current directory — never a
//! child, which belongs to the root whose stores it is a worktree of.

use crate::support::resolution::{repos, tagged};
use crate::support::worktrees::{gs, identity, ok};
use crate::support::{run_git_pub, strip_ansi, TestEnv};
use std::path::PathBuf;

/// d; b asking for d; a root clone declaring both, placed.
fn workspace(env: &TestEnv) -> PathBuf {
    let d = tagged(env, "d", &[("v1.0.0", "")]);
    let b = tagged(
        env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.0.0\"")]),
        )],
    );
    let config = repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.0.0\""),
    ]);
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    identity(&ws);
    ok(&gs(&ws, &["sync"]));
    ws
}

/// The REPO column of `ls`'s table.
fn rows(stdout: &str) -> Vec<String> {
    strip_ansi(stdout)
        .lines()
        .skip(1)
        .filter(|l| !l.starts_with("hint:"))
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_string))
        .collect()
}

/// From a child with a config of its own, or from deep inside it, the
/// workspace is the root's: a child is never a workspace.
#[test]
fn normal_001_from_a_child_or_inside_one_the_root_is_the_workspace() {
    let env = TestEnv::new("ws_find_child");
    let ws = workspace(&env);
    let child = ws.join("imports/b");
    assert!(child.join(".gitscale.toml").is_file());
    std::fs::create_dir_all(child.join("deep/er")).unwrap();
    for dir in [child.clone(), child.join("deep/er")] {
        let out = gs(&dir, &["ls"]);
        ok(&out);
        assert_eq!(
            rows(&out.stdout),
            vec!["imports/b", "imports/d"],
            "{}",
            out.stdout
        );
    }
}

/// A child with no config of its own finds the root the same way.
#[test]
fn edge_002_a_child_without_a_config_finds_the_root() {
    let env = TestEnv::new("ws_find_plain_child");
    let ws = workspace(&env);
    let child = ws.join("imports/d");
    assert!(!child.join(".gitscale.toml").exists());
    let out = gs(&child, &["ls"]);
    ok(&out);
    assert_eq!(
        rows(&out.stdout),
        vec!["imports/b", "imports/d"],
        "{}",
        out.stdout
    );
}

/// A clone made by hand inside the workspace, with a config at its top, is
/// a workspace of its own.
#[test]
fn normal_003_a_clone_made_by_hand_is_a_workspace_of_its_own() {
    let env = TestEnv::new("ws_find_hand_clone");
    let ws = workspace(&env);
    let other = env.create_bare_repo("other", "main", &[("o.txt", "o")]);
    let own = env.create_bare_repo(
        "own",
        "main",
        &[(
            ".gitscale.toml",
            &repos(&[("libs/other", &other, ", revision = \"main\"")]),
        )],
    );
    let clone = ws.join("tools/own");
    std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
    run_git_pub(
        &ws,
        &[
            "clone",
            "-q",
            own.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    let out = gs(&clone, &["ls"]);
    ok(&out);
    assert_eq!(rows(&out.stdout), vec!["libs/other"], "{}", out.stdout);
}

/// A config that is not at the top of a repository makes no workspace; the
/// error says so.
#[test]
fn error_004_a_config_not_at_a_repository_top_is_no_workspace() {
    let env = TestEnv::new("ws_find_stray_config");
    let stray = env.repos_remote.join("plain/dir");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join(".gitscale.toml"), "[repos]\n").unwrap();
    let out = gs(&stray, &["ls"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("not inside a GitScale workspace"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("has a .gitscale.toml but is not the top of a git repository"),
        "{}",
        out.stderr
    );
}
