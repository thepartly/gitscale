//! When resolution asks the remotes: always in CI, where a runner keeps what
//! an earlier job fetched; online for `sync` and `pull`; only on a miss
//! after any other git command.

use crate::support::resolution::repos;
use crate::support::worktrees::{gs, head, identity, ok};
use crate::support::{run_git_pub, TestEnv};
use std::path::PathBuf;

/// A root clone at `ws` declaring `lib` on its `main` branch, not placed.
fn on_a_branch(env: &TestEnv) -> (PathBuf, PathBuf) {
    let lib = env.create_bare_repo("lib", "main", &[("a.txt", "v1")]);
    let root = env.create_bare_repo(
        "root",
        "main",
        &[
            (
                ".gitscale.toml",
                &repos(&[("imports/lib", &lib, ", revision = \"main\"")]),
            ),
            (".gitignore", "imports/\n"),
        ],
    );
    (clone(env, &root, "ws"), lib)
}

/// Another clone of the root at `name`.
fn clone(env: &TestEnv, root: &std::path::Path, name: &str) -> PathBuf {
    let ws = env.repos_remote.join(name);
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    identity(&ws);
    ws
}

fn in_ci(env: &TestEnv, ws: &std::path::Path, args: &[&str]) -> crate::support::CliOutput {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(["-C", ws.to_str().unwrap(), "--no-cache"])
        .args(args)
        .env("CI", "true")
        .env("GITSCALE_CACHE_DIR", &env.cache)
        .output()
        .unwrap();
    crate::support::CliOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        success: out.status.success(),
    }
}

/// In CI a branch resolves to where its remote has it now, though the build
/// directory still holds what the last job fetched.
#[test]
fn normal_001_in_ci_a_moved_branch_resolves_to_its_new_commit() {
    let env = TestEnv::new("net_ci_moved");
    let (ws, lib) = on_a_branch(&env);
    ok(&in_ci(&env, &ws, &["sync"]));
    let moved = env.push_commit(&lib, "main", "a.txt", "v2");
    ok(&in_ci(&env, &ws, &["sync"]));
    assert_eq!(head(&ws.join("imports/lib")), moved);
}

/// In CI a remote that cannot be reached fails resolution, rather than
/// building what an earlier job fetched; off CI the refs fetched before
/// still answer, with a warning.
#[test]
fn error_002_in_ci_an_unreachable_remote_fails_instead_of_using_old_refs() {
    let env = TestEnv::new("net_ci_unreachable");
    let (ws, lib) = on_a_branch(&env);
    let mine = clone(&env, &env.repos_remote.join("root.git"), "mine");
    ok(&in_ci(&env, &ws, &["sync"]));
    ok(&gs(&mine, &["sync"]));
    let away = lib.with_extension("moved");
    std::fs::rename(&lib, &away).unwrap();

    let out = in_ci(&env, &ws, &["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("cannot fetch"), "{}", out.stderr);
    assert!(
        !out.stderr.contains("using what was fetched before"),
        "{}",
        out.stderr
    );

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(["-C", mine.to_str().unwrap(), "ls", "--fetch"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{}", stderr);
    assert!(
        stderr.contains("using what was fetched before"),
        "{}",
        stderr
    );
}

/// After a git command that moved a `HEAD`, a developer machine fetches only
/// what resolution lacks, so a branch it has is not fetched again; in CI the
/// placement is online.
#[test]
fn normal_003_a_forwarded_command_places_online_only_in_ci() {
    let env = TestEnv::new("net_forward");
    let (ws, lib) = on_a_branch(&env);
    ok(&gs(&ws, &["sync"]));
    let before = head(&ws.join("imports/lib"));
    let moved = env.push_commit(&lib, "main", "a.txt", "v2");

    ok(&gs(&ws, &["commit", "-q", "--allow-empty", "-m", "here"]));
    assert_eq!(head(&ws.join("imports/lib")), before, "nothing was missing");

    let out = in_ci(&env, &ws, &["commit", "-q", "--allow-empty", "-m", "ci"]);
    ok(&out);
    assert_eq!(head(&ws.join("imports/lib")), moved, "{}", out.stdout);
}
