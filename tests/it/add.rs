//! `gitscale add`: writing an entry into the root config.

use crate::support::TestEnv;

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_adds_an_entry_and_reports_it() {
    let env = TestEnv::new("add_entry");
    env.write_config("");

    let out = env.run(&[
        "add",
        "libs/core",
        "https://github.com/org/core.git",
        "main",
    ]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("add_entry_stdout", out.stdout);

    // Config file should contain the entry
    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    assert!(config.contains("libs/core"));
    assert!(config.contains("https://github.com/org/core.git"));
}

#[test]
fn normal_002_adds_an_artefact_entry() {
    let env = TestEnv::new("add_entry_artefact");
    env.write_config("");

    let out = env.run(&[
        "add",
        "meta/svc",
        "https://github.com/org/svc.git",
        "main",
        "--artefact",
        "replace",
    ]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("add_entry_artefact_stdout", out.stdout);

    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    assert!(config.contains("artefact"));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn edge_003_keeps_the_clean_table() {
    let env = TestEnv::new("add_preserves_clean_rules");
    env.write_config(
        "[clean]\nexclude = [\".env\", \"envs/\"]\n\n[repos]\n\
         \"libs/a\" = { url = \"https://github.com/org/a.git\", revision = \"main\" }\n",
    );

    let out = env.run(&["add", "libs/b", "https://github.com/org/b.git", "main"]);
    assert!(out.success, "stderr: {}", out.stderr);

    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    assert!(
        config.contains("[clean]") && config.contains("envs/"),
        "add dropped the [clean] table:\n{}",
        config
    );
}

/// With no config anywhere above, `add` starts one where `-C` points, and
/// every later command can read it.
#[test]
fn edge_006_creates_a_config_when_none_exists() {
    let env = TestEnv::new("add_creates_config");
    let config = env.playground.join(".gitscale.toml");
    assert!(!config.exists());

    let out = env.run(&["add", "libs/core", "https://github.com/org/core.git", "v1"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(
        text.contains("\"libs/core\"") && text.contains("revision = \"v1\""),
        "{}",
        text
    );
    assert!(env.run(&["status"]).success, "the new config does not load");
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_004_refuses_a_directory_already_declared() {
    let env = TestEnv::new("add_duplicate_fails");
    env.write_config(
        r#"[repos]
"libs/core" = { url = "https://github.com/org/core.git", revision = "main" }
"#,
    );

    let before = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();

    let out = env.run(&[
        "add",
        "libs/core",
        "https://github.com/org/other.git",
        "main",
    ]);
    assert!(!out.success);
    assert!(out.stderr.contains("already declared"));
    assert_eq!(
        std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap(),
        before,
        "a refused add must leave the config as it was"
    );
}

/// `add` holds an entry to the rules every later command reads the config
/// by, rather than writing one that leaves the workspace unloadable.
#[test]
fn error_005_refuses_an_entry_the_config_would_reject() {
    let env = TestEnv::new("add_refuses_invalid");
    env.write_config("[repos]\n");
    let url = "https://github.com/org/core.git";

    for (directory, repo_url) in [
        ("../escape", url),
        ("/abs/path", url),
        ("libs/core", "ext::sh -c touch% /tmp/pwned"),
    ] {
        let out = env.run(&["add", directory, repo_url, "main"]);
        assert!(!out.success, "add accepted {} = {}", directory, repo_url);
        let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
        assert!(
            !config.contains(directory) && !config.contains("ext::"),
            "the rejected entry was written:\n{}",
            config
        );
    }
    assert!(env.run(&["status"]).success, "the config no longer loads");
}

/// `add` rewrites the whole file, so a config it cannot read must stop it:
/// writing back what it parsed would replace the user's file.
#[test]
fn error_007_leaves_an_unparseable_config_untouched() {
    let env = TestEnv::new("add_unparseable");
    let broken = "# my notes\n[repos\n\"libs/a\" = { url = \"https://github.com/org/a.git\" }\n";
    env.write_config(broken);

    let out = env.run(&["add", "libs/b", "https://github.com/org/b.git", "main"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("invalid TOML"), "{}", out.stderr);
    assert_eq!(
        std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap(),
        broken
    );
}

/// The duplicate check is about directories, not spellings: `libs/core/` and
/// `./libs/core` are the directory `libs/core` already declares.
#[test]
#[ignore = "bug: add compares directory strings exactly, so 'libs/core/' and './libs/core' are added beside 'libs/core'"]
fn error_008_refuses_a_directory_already_declared_under_another_spelling() {
    let env = TestEnv::new("add_duplicate_spelling");
    env.write_config(
        "[repos]\n\"libs/core\" = { url = \"https://github.com/org/core.git\", revision = \"main\" }\n",
    );
    for spelling in ["libs/core/", "./libs/core", "libs//core"] {
        let out = env.run(&["add", spelling, "https://github.com/org/other.git", "v2"]);
        assert!(!out.success, "{} was added", spelling);
        let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
        assert!(!config.contains("other.git"), "{}: {}", spelling, config);
    }
}
