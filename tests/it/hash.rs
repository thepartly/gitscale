//! `git scale hash`: the source hash of a repository — its tree and its
//! dependencies', resolved as its own pipeline resolves them.

use std::path::{Path, PathBuf};

use crate::support::resolution::{allow, dependant, repos, root_workspace, tagged};
use crate::support::status_clean::gitscale_at;
use crate::support::worktrees::*;
use crate::support::{run_git_pub, TestEnv};

/// The hash `git scale hash` prints for `dir`, run in `ws`.
fn hash(ws: &Path, dir: &str) -> String {
    let out = gs(ws, &["hash", dir]);
    ok(&out);
    let (hash, named) = out.stdout.trim().split_once("  ").unwrap();
    assert_eq!(named, dir, "{}", out.stdout);
    assert_eq!(hash.len(), 64, "{}", out.stdout);
    hash.to_string()
}

/// `repo` cloned as a workspace of its own at `name`, synced.
fn standalone(env: &TestEnv, repo: &Path, name: &str) -> PathBuf {
    let dest = env.repos_remote.join(name);
    run_git_pub(
        &env.repos_remote,
        &[
            "clone",
            "-q",
            repo.to_str().unwrap(),
            dest.to_str().unwrap(),
        ],
    );
    identity(&dest);
    ok(&gs(&dest, &["sync"]));
    dest
}

/// B asks for D at v1.0.0; C for D at v1.1.0; the root for B and C — so the
/// root's workspace holds D at v1.1.0, above what B asks for.
fn raised(env: &TestEnv) -> (PathBuf, PathBuf) {
    let d = tagged(env, "d", &[("v1.0.0", ""), ("v1.1.0", "")]);
    let b = dependant(env, "b", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let c = dependant(env, "c", &[("libs/d", &d, ", revision = \"v1.1.0\"")]);
    let ws = root_workspace(
        env,
        &format!(
            "{}{}",
            allow(env),
            repos(&[
                ("imports/b", &b, ", revision = \"v1.0.0\""),
                ("imports/c", &c, ", revision = \"v1.0.0\""),
            ])
        ),
    );
    ok(&gs(&ws, &["sync"]));
    (ws, b)
}

/// A repository's hash in another workspace is the one its own pipeline
/// takes, though that workspace raised one of its dependencies: B is hashed
/// with D at v1.0.0, which it asks for, not at the v1.1.0 checked out.
#[test]
fn normal_001_a_checkout_hashes_as_its_own_pipeline_would() {
    let env = TestEnv::new("hash_own_pipeline");
    let (ws, b) = raised(&env);
    let alone = standalone(&env, &b, "b-alone");
    assert_eq!(hash(&ws, "imports/b"), hash(&alone, "."));
    assert_ne!(hash(&ws, "imports/b"), hash(&ws, "imports/c"));
}

/// The hash is of trees: a commit that changes no file keeps it, a
/// dependency at other content changes it.
#[test]
fn normal_002_it_follows_trees_not_commits() {
    let env = TestEnv::new("hash_trees");
    let (_, b) = raised(&env);
    let alone = standalone(&env, &b, "b-alone");
    let before = hash(&alone, ".");
    run_git_pub(&alone, &["commit", "-q", "--allow-empty", "-m", "nothing"]);
    assert_eq!(hash(&alone, "."), before);

    let config = std::fs::read_to_string(alone.join(".gitscale.toml")).unwrap();
    std::fs::write(
        alone.join(".gitscale.toml"),
        config.replace("v1.0.0", "v1.1.0"),
    )
    .unwrap();
    run_git_pub(&alone, &["commit", "-q", "-am", "d v1.1.0"]);
    assert_ne!(hash(&alone, "."), before);
}

/// A checkout at its pin hashes with its dependencies at their pins; joined
/// to the topic, with the topic's branches — as its pipeline on the topic
/// branch would build it.
#[test]
fn normal_003_a_checkout_on_the_topic_hashes_with_the_topics_branches() {
    let env = TestEnv::new("hash_topic");
    let (ws, _) = raised(&env);
    let pinned = hash(&ws, "imports/b");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/d"]));
    let d = ws.join("imports/d");
    identity(&d);
    std::fs::write(d.join("VERSION"), "changed").unwrap();
    run_git_pub(&d, &["commit", "-q", "-am", "the change"]);
    assert_eq!(hash(&ws, "imports/b"), pinned);

    ok(&gs(&ws, &["topic", "join", "imports/b"]));
    assert_ne!(hash(&ws, "imports/b"), pinned);
}

/// An entry with `recursive = false` reaches only that repository, not its
/// dependencies.
#[test]
fn edge_004_recursive_false_keeps_an_entrys_dependencies_out() {
    let env = TestEnv::new("hash_not_recursive");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let ws = root_workspace(
        &env,
        &repos(&[(
            "imports/b",
            &b,
            ", revision = \"v1.0.0\", recursive = false",
        )]),
    );
    ok(&gs(&ws, &["sync"]));
    let out = gs(&ws, &["hash", "--format", "json"]);
    ok(&out);
    let rows: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let urls: Vec<String> = rows[0]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["url"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(urls.len(), 2, "{:?}", urls);
    assert!(urls.contains(&b.display().to_string()), "{:?}", urls);
    assert_eq!(rows[0]["sources"][1]["directory"], "imports/b");
}

/// `ls --format json` carries each checkout's source hash.
#[test]
fn edge_005_ls_json_carries_each_checkouts_source_hash() {
    let env = TestEnv::new("hash_in_ls");
    let (ws, _) = raised(&env);
    let out = gs(&ws, &["ls", "--format", "json"]);
    ok(&out);
    let rows: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "imports/b")
        .unwrap();
    assert_eq!(row["source_hash"], hash(&ws, "imports/b"));
}

/// Uncommitted changes in a source fail the hash, which would not be of
/// anything built; `--committed` hashes the commit, changes left out.
#[test]
fn error_006_uncommitted_changes_fail_unless_committed() {
    let env = TestEnv::new("hash_uncommitted");
    let (ws, _) = raised(&env);
    let before = hash(&ws, ".");
    std::fs::write(ws.join("README.md"), "edited").unwrap();
    let out = gs(&ws, &["hash"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains(". has uncommitted changes; commit them, or --committed to hash its commit"),
        "{}",
        out.stderr
    );
    let out = gs(&ws, &["hash", "--committed"]);
    ok(&out);
    assert!(out.stdout.starts_with(&before), "{}", out.stdout);
}

/// How the workspace takes a checkout changes nothing of what it was built
/// from: the same hash as its sources and as its artefact.
#[test]
fn normal_007_a_checkout_hashes_the_same_as_source_and_as_artefact() {
    let env = TestEnv::new("hash_forms");
    let app = env.artefact_repo("app", &[("app.bin", "built")]);
    env.write_config(&format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        env.registries(),
        app.display()
    ));
    ok(&env.run(&["sync"]));
    let as_source = hash(&env.playground, "meta/app");
    env.prefer(&app, gitscale::prefer::Form::Artefact);
    ok(&env.run(&["sync"]));
    assert!(!env.playground.join("meta/app/.git").exists());
    assert_eq!(hash(&env.playground, "meta/app"), as_source);
}

/// A CI job keeps no stores of its own: the tree of a dependency at the
/// version a checkout's own pipeline takes — below the one its workspace
/// raised it to — comes from that commit, fetched depth 1 with no files, not
/// from an image the dependency never published. With the cache on or off,
/// the job hashes B as a developer machine does.
#[test]
fn normal_008_a_ci_job_hashes_a_checkout_whose_dependency_was_raised() {
    let env = TestEnv::new("hash_ci_raised");
    let (ws, b) = raised(&env);
    let want = hash(&standalone(&env, &b, "b-alone"), ".");
    let origin = git(&ws, &["remote", "get-url", "origin"]);
    for (job, sync) in [
        ("job-no-cache", vec!["sync", "--no-cache"]),
        ("job-cache", vec!["sync"]),
    ] {
        let job = env.repos_remote.join(job);
        run_git_pub(
            &env.repos_remote,
            &["clone", "-q", &origin, job.to_str().unwrap()],
        );
        let ci = [("CI", "true")];
        let placed = gitscale_at(&job, &env.cache, &ci, &sync);
        assert_eq!(placed.code, Some(0), "{}{}", placed.stdout, placed.stderr);
        let hashed = gitscale_at(&job, &env.cache, &ci, &["hash", "imports/b"]);
        assert_eq!(hashed.code, Some(0), "{}{}", hashed.stdout, hashed.stderr);
        assert_eq!(
            hashed.stdout.split_once("  ").map(|(h, _)| h),
            Some(want.as_str()),
            "{}",
            hashed.stdout
        );
    }
}
