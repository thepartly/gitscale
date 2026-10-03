//! `gitscale remove`: taking an entry out of the root config.

use crate::support::TestEnv;

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_removes_an_entry_and_keeps_the_rest() {
    let env = TestEnv::new("remove_entry");
    env.write_config(
        r#"[repos]
"libs/core" = { url = "https://github.com/org/core.git", revision = "main" }
"libs/utils" = { url = "https://github.com/org/utils.git", revision = "v1" }
"#,
    );

    let out = env.run(&["remove", "libs/core"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("remove_entry_stdout", out.stdout);

    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    assert!(!config.contains("libs/core"));
    assert!(config.contains("libs/utils"));
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_002_refuses_an_undeclared_directory() {
    let env = TestEnv::new("remove_unknown_fails");
    env.write_config(
        r#"[repos]
"libs/core" = { url = "https://github.com/org/core.git", revision = "main" }
"#,
    );

    let before = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();

    let out = env.run(&["remove", "nonexistent"]);
    assert!(!out.success);
    assert!(out.stderr.contains("not declared"));
    assert_eq!(
        std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap(),
        before
    );
}

/// `remove` rewrites the whole file, so a config it cannot read is left
/// exactly as it is.
#[test]
fn error_003_leaves_an_unparseable_config_untouched() {
    let env = TestEnv::new("remove_unparseable");
    let broken = "[repos\n\"libs/a\" = { url = \"https://github.com/org/a.git\" }\n";
    env.write_config(broken);

    let out = env.run(&["remove", "libs/a"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("invalid TOML"), "{}", out.stderr);
    assert_eq!(
        std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap(),
        broken
    );
}
