//! Reading `.gitscale.toml`: where it may live and what it may say.

use crate::support::{git_stdout, run_git_pub, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// `-C` starts the upward search, so pointing it at a directory inside the
/// workspace finds the workspace's config and acts on every entry, placed
/// relative to the config rather than to that directory.
#[test]
fn normal_002_a_root_option_inside_the_workspace_finds_its_config() {
    let env = TestEnv::new("config_root_subdir");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    let inside = env.playground.join("docs/deep");
    std::fs::create_dir_all(&inside).unwrap();
    let out = env.run_in(&inside, &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("libs/lib/a.txt").is_file());
    assert!(!inside.join("libs").exists());
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// Unknown keys in an entry are ignored on read, as docs/configuration.md
/// says — so a misspelt `revision` leaves the entry with none, and it follows
/// the default branch. Pinned as documented; it is a trap worth knowing.
#[test]
fn edge_003_a_misspelt_entry_key_is_ignored_as_documented() {
    let env = TestEnv::new("config_misspelt_key");
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "v1")]);
    run_git_pub(&bare, &["tag", "v1", "main"]);
    let tip = env.push_commit(&bare, "main", "a.txt", "v2");
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revison = \"v1\" }}\n",
        bare.display()
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        git_stdout(&env.playground.join("libs/lib"), &["rev-parse", "HEAD"]),
        tip
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// The workspace is the top of a git repository; anything else is refused.
#[test]
fn error_001_a_config_outside_a_git_repository_is_refused() {
    let env = TestEnv::new("not_a_repo");
    let outside = env.repos_remote.join("plain-dir");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join(".gitscale.toml"), "[repos]\n").unwrap();
    let out = env.run_in(&outside, &["ls"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("not the top of a git repository"),
        "{}",
        out.stderr
    );
}

/// A config that is not TOML is refused by every command, naming the file
/// and the parser's reason, and nothing is cloned.
#[test]
fn error_004_invalid_toml_is_refused_naming_the_file() {
    let env = TestEnv::new("config_invalid_toml");
    env.write_config("[repos\n\"libs/a\" = { url = \"x\" }\n");
    for command in ["ls", "pull", "fetch", "sync", "push"] {
        let out = env.run(&[command]);
        assert!(!out.success, "{}", command);
        assert!(
            out.stderr.contains(".gitscale.toml") && out.stderr.contains("invalid TOML"),
            "{}: {}",
            command,
            out.stderr
        );
    }
    assert!(!env.playground.join("libs").exists());
}

/// A checkout directory must be a directory of its own inside the workspace:
/// not the workspace itself (`.`), and nothing inside its git directory,
/// where a checkout's files would be git's own — hooks included.
#[test]
#[ignore = "bug: check_directory only refuses '..' and absolute paths, so '.', './' and '.git/hooks' are accepted"]
fn error_005_the_workspace_itself_or_its_git_directory_is_refused() {
    let env = TestEnv::new("config_dot_entries");
    for directory in [".", "./", ".git", ".git/hooks", "libs/.git/x"] {
        env.write_config(&format!(
            "[repos]\n\"{}\" = {{ url = \"https://example.com/a.git\", revision = \"main\" }}\n",
            directory
        ));
        let out = env.run(&["ls"]);
        assert!(!out.success, "{:?} was accepted: {}", directory, out.stdout);
        assert!(
            out.stderr.contains(directory),
            "{}: {}",
            directory,
            out.stderr
        );
    }
}

/// Two entries that name one directory, spelt differently, would fight over
/// one checkout on every placement. They are refused when the config is read.
#[test]
#[ignore = "bug: entries are told apart by their spelling, so 'libs/x' and 'libs/x/' (or './libs/x') both load"]
fn error_006_two_entries_naming_one_directory_are_refused() {
    let env = TestEnv::new("config_same_dir");
    for (a, b) in [
        ("libs/x", "libs/x/"),
        ("libs/x", "./libs/x"),
        ("libs/x", "libs//x"),
        ("libs/x", "libs/x/."),
    ] {
        env.write_config(&format!(
            "[repos]\n\"{}\" = {{ url = \"https://example.com/a.git\", revision = \"v1\" }}\n\
             \"{}\" = {{ url = \"https://example.com/b.git\", revision = \"v2\" }}\n",
            a, b
        ));
        let out = env.run(&["ls"]);
        assert!(!out.success, "{:?} and {:?} both loaded", a, b);
    }
}

/// A refused value says what is wrong with it, not only where it is: the CLI
/// prints the outermost message alone, so the reason must be in it.
#[test]
#[ignore = "bug: these errors are wrapped in a context naming only the key, and the CLI prints only the outermost message"]
fn error_007_a_refused_value_says_why() {
    let env = TestEnv::new("config_errors_say_why");
    for (config, reason) in [
        ("[resolve]\nhoist_dir = \"../out\"\n", "'..'"),
        ("[clean]\nkeep_recent = \"soon\"\n", "soon"),
    ] {
        env.write_config(config);
        let out = env.run(&["ls"]);
        assert!(!out.success, "{}", config);
        assert!(out.stderr.contains(reason), "{}: {}", reason, out.stderr);
    }
}
