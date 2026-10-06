//! How a checkout arrives — its sources, or the artefact of its release —
//! chosen by `git scale prefer`, per repository, and applied by the next
//! placement.

use crate::support::status_clean::json_row;
use crate::support::TestEnv;

/// A workspace whose `meta/app` is a release with a published image,
/// synced: its sources, by default.
fn workspace(env: &TestEnv, dir: &str) -> std::path::PathBuf {
    let app = env.artefact_repo("app", &[("app.bin", "built")]);
    env.write_config(&format!(
        "{}[repos]\n\"{}\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        env.registries(),
        dir,
        app.display()
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    app
}

/// `prefer` records and changes nothing; `ls` shows the form to come, and
/// the next placement applies it — artefact, and back to sources.
#[test]
fn normal_001_prefer_records_and_the_next_placement_applies_it() {
    let env = TestEnv::new("prefer_forms");
    workspace(&env, "meta/app");
    let dest = env.playground.join("meta/app");
    assert!(dest.join(".git").exists());

    let out = env.run(&["prefer", "--artefact", "meta/app"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("meta/app   artefact"), "{}", out.stdout);
    assert!(dest.join(".git").exists(), "nothing placed yet");
    let row = json_row(&env, "meta/app");
    assert_eq!(row["as"], "source");
    assert_eq!(row["as_next"], "artefact");
    assert_eq!(row["as_reason"], "preferred");
    let out = env.run(&["prefer"]);
    assert!(out.stdout.contains("meta/app   artefact"), "{}", out.stdout);

    assert!(env.run(&["sync"]).success);
    assert!(!dest.join(".git").exists());
    assert!(dest.join("dist/app.bin").is_file());
    let row = json_row(&env, "meta/app");
    assert_eq!(row["as"], "artefact");
    assert_eq!(row["as_next"], serde_json::Value::Null);

    assert!(env.run(&["prefer", "--source", "meta/app"]).success);
    let out = env.run(&["prefer"]);
    assert!(
        out.stdout.contains("Every checkout takes its sources."),
        "{}",
        out.stdout
    );
}

/// A preference is the repository's, not the directory's: a checkout moved
/// to another directory keeps it, and every worktree of the root sees it.
#[test]
fn normal_002_a_preference_follows_the_repository_into_every_worktree() {
    let env = TestEnv::new("prefer_follows");
    workspace(&env, "meta/app");
    assert!(env.run(&["prefer", "--artefact", "meta/app"]).success);

    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    env.write_config(&config.replace("meta/app", "meta/moved"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("meta/moved/dist/app.bin").is_file());
    assert!(!env.playground.join("meta/moved/.git").exists());

    crate::support::run_git_pub(&env.playground, &["add", ".gitscale.toml"]);
    crate::support::run_git_pub(&env.playground, &["commit", "-q", "-m", "config"]);
    let other = env.repos_remote.join("other");
    crate::support::run_git_pub(
        &env.playground,
        &["worktree", "add", "-q", "--detach", other.to_str().unwrap()],
    );
    let out = env.run_in(&other, &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(other.join("meta/moved/dist/app.bin").is_file());
}

/// A checkout holding work is never replaced by its artefact: the placement
/// reports it and keeps it, and the work stays.
#[test]
fn edge_003_a_checkout_holding_work_keeps_its_sources() {
    let env = TestEnv::new("prefer_keeps_work");
    workspace(&env, "meta/app");
    let dest = env.playground.join("meta/app");
    crate::support::resolution::set_writable(&dest.join("README.md"));
    std::fs::write(dest.join("README.md"), "my work").unwrap();
    assert!(env.run(&["prefer", "--artefact", "meta/app"]).success);

    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("not replaced by its artefact"),
        "{}",
        out.stderr
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("README.md")).unwrap(),
        "my work"
    );
}

/// A form needs the checkouts it is for; a name no checkout has is an error,
/// and nothing is recorded.
#[test]
fn error_004_a_form_needs_checkouts_it_names() {
    let env = TestEnv::new("prefer_errors");
    workspace(&env, "meta/app");
    for (args, message) in [
        (
            &["prefer", "--artefact"][..],
            "--artefact names the checkouts it is for",
        ),
        (
            &["prefer", "--artefact", "meta/nowhere"][..],
            "meta/nowhere is not a checkout of this workspace",
        ),
        (
            &["prefer", "--artefact", "--source", "meta/app"][..],
            "cannot be used with",
        ),
    ] {
        let out = env.run(args);
        assert!(!out.success, "{:?}: {}", args, out.stdout);
        assert!(out.stderr.contains(message), "{:?}: {}", args, out.stderr);
    }
    let out = env.run(&["prefer"]);
    assert!(
        out.stdout.contains("Every checkout takes its sources."),
        "{}",
        out.stdout
    );
}
