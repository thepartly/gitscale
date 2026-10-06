//! `git scale require` / `unrequire`: an entry written into, or taken out of,
//! the root config in place, and the workspace placed to match.

use crate::support::TestEnv;

fn config_text(env: &TestEnv) -> String {
    std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap()
}

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// The entry is written, said, and checked out at the revision it names.
#[test]
fn normal_001_requires_an_entry_reports_it_and_checks_it_out() {
    let env = TestEnv::new("add_entry");
    let core = env.create_bare_repo("core", "main", &[("lib.txt", "v1")]);
    env.write_config("");

    let out = env.run(&["require", "libs/core", core.to_str().unwrap(), "main"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains(&format!("Required libs/core → {} @ main", core.display())),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("ok    libs/core"), "{}", out.stdout);
    let config = config_text(&env);
    assert!(config.contains("\"libs/core\""), "{}", config);
    assert!(config.contains("revision = \"main\""), "{}", config);
    assert!(env.playground.join("libs/core/lib.txt").is_file());
}

/// Taken out: the entry goes, the rest of the file stays, and its checkout —
/// holding nothing of anyone's — goes with it.
#[test]
fn normal_009_unrequire_removes_the_entry_and_its_clean_checkout() {
    let env = TestEnv::new("remove_entry");
    let core = env.create_bare_repo("core", "main", &[("lib.txt", "v1")]);
    let utils = env.create_bare_repo("utils", "main", &[("u.txt", "v1")]);
    env.write_config(&format!(
        "# the workspace\n[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\" }}\n\
         \"libs/utils\" = {{ url = \"{}\", revision = \"main\" }} # kept\n",
        core.display(),
        utils.display()
    ));
    assert!(env.run(&["sync"]).success);

    let out = env.run(&["unrequire", "libs/core"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("Unrequired libs/core"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("remove  libs/core (no longer needed)"),
        "{}",
        out.stdout
    );
    let config = config_text(&env);
    assert!(!config.contains("libs/core"), "{}", config);
    assert!(
        config.contains("# the workspace") && config.contains("# kept"),
        "{}",
        config
    );
    assert!(!env.playground.join("libs/core").exists());
    assert!(env.playground.join("libs/utils/u.txt").is_file());
}

/// No revision given, none is written: the entry follows the default branch.
#[test]
fn normal_010_without_a_revision_none_is_written() {
    let env = TestEnv::new("require_no_revision");
    let core = env.create_bare_repo("core", "main", &[("lib.txt", "v1")]);
    env.write_config("");
    let out = env.run(&["require", "libs/core", core.to_str().unwrap()]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        !config_text(&env).contains("revision ="),
        "{}",
        config_text(&env)
    );
    assert!(env.playground.join("libs/core/lib.txt").is_file());
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// Edited in place: comments, key order, keys of other entries and tables
/// gitscale does not know about are all still there, after `require` and
/// after `unrequire`.
#[test]
fn edge_003_keeps_comments_and_every_other_table() {
    let env = TestEnv::new("add_preserves_clean_rules");
    let a = env.create_bare_repo("a", "main", &[("a.txt", "a")]);
    let b = env.create_bare_repo("b", "main", &[("b.txt", "b")]);
    env.write_config(&format!(
        "# notes on the workspace\n[clean]\nexclude = [\".env\", \"envs/\"] # keep these\n\n\
         [team]\nowner = \"platform\"\n\n[repos]\n\
         \"libs/a\" = {{ url = \"{}\", revision = \"main\", singleton = true }}\n",
        a.display()
    ));
    let kept = [
        "# notes on the workspace",
        "# keep these",
        "envs/",
        "[team]",
        "owner = \"platform\"",
        "singleton = true",
    ];

    let out = env.run(&["require", "libs/b", b.to_str().unwrap(), "main"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let config = config_text(&env);
    for kept in kept {
        assert!(config.contains(kept), "{:?} is gone:\n{}", kept, config);
    }
    assert!(
        config.find("libs/a").unwrap() < config.find("libs/b").unwrap(),
        "{}",
        config
    );

    let out = env.run(&["unrequire", "libs/b"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let config = config_text(&env);
    for kept in kept {
        assert!(config.contains(kept), "{:?} is gone:\n{}", kept, config);
    }
}

/// With no config anywhere above, `require` starts one at the top of the
/// repository, and every later command can read it.
#[test]
fn edge_006_creates_a_config_when_none_exists() {
    let env = TestEnv::new("add_creates_config");
    let core = env.create_bare_repo("core", "main", &[("lib.txt", "v1")]);
    run_git(&core, &["tag", "v1"]);
    let config = env.playground.join(".gitscale.toml");
    assert!(!config.exists());

    let out = env.run(&["require", "libs/core", core.to_str().unwrap(), "v1"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(
        text.contains("\"libs/core\"") && text.contains("revision = \"v1\""),
        "{}",
        text
    );
    assert!(env.run(&["ls"]).success, "the new config does not load");
}

/// An unrequired checkout that holds work stays, and the command says so
/// and fails, as every placement does.
#[test]
fn edge_012_an_unrequired_checkout_with_work_is_kept() {
    let env = TestEnv::new("unrequire_keeps_work");
    let core = env.create_bare_repo("core", "main", &[("lib.txt", "v1")]);
    env.write_config(&format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        core.display()
    ));
    assert!(env.run(&["sync"]).success);
    std::fs::write(env.playground.join("libs/core/work.txt"), "mine").unwrap();

    let out = env.run(&["unrequire", "libs/core"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("libs/core (no longer needed, but modified"),
        "{}",
        out.stdout
    );
    assert!(!config_text(&env).contains("libs/core"));
    assert!(env.playground.join("libs/core/work.txt").is_file());
}

/// The directory is a path from where the command runs.
#[test]
fn edge_013_the_directory_is_relative_to_the_current_directory() {
    let env = TestEnv::new("require_relative");
    let core = env.create_bare_repo("core", "main", &[("lib.txt", "v1")]);
    env.write_config("");
    let libs = env.playground.join("libs");
    std::fs::create_dir_all(&libs).unwrap();

    let out = env.run_in(&libs, &["require", "core", core.to_str().unwrap(), "main"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        config_text(&env).contains("\"libs/core\""),
        "{}",
        config_text(&env)
    );

    let out = env.run_in(&libs, &["unrequire", "core"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!config_text(&env).contains("libs/core"));
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
    let before = config_text(&env);

    let out = env.run(&[
        "require",
        "libs/core",
        "https://github.com/org/other.git",
        "main",
    ]);
    assert!(!out.success);
    assert!(out.stderr.contains("already declared"), "{}", out.stderr);
    assert_eq!(
        config_text(&env),
        before,
        "a refused require must leave the config as it was"
    );
}

/// `require` holds an entry to the rules every later command reads the
/// config by, rather than writing one that leaves the workspace unloadable.
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
        let out = env.run(&["require", directory, repo_url, "main"]);
        assert!(
            !out.success,
            "require accepted {} = {}",
            directory, repo_url
        );
        let config = config_text(&env);
        assert!(
            !config.contains(directory) && !config.contains("ext::"),
            "the rejected entry was written:\n{}",
            config
        );
    }
    assert!(env.run(&["ls"]).success, "the config no longer loads");
}

/// A config that does not read stops `require`, and stays as it is.
#[test]
fn error_007_leaves_an_unparseable_config_untouched() {
    let env = TestEnv::new("add_unparseable");
    let broken = "# my notes\n[repos\n\"libs/a\" = { url = \"https://github.com/org/a.git\" }\n";
    env.write_config(broken);

    let out = env.run(&["require", "libs/b", "https://github.com/org/b.git", "main"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("invalid TOML"), "{}", out.stderr);
    assert_eq!(config_text(&env), broken);
}

/// The duplicate check is about directories, not spellings: `libs/core/` and
/// `./libs/core` are the directory `libs/core` already declares.
#[test]
fn error_008_refuses_a_directory_already_declared_under_another_spelling() {
    let env = TestEnv::new("add_duplicate_spelling");
    env.write_config(
        "[repos]\n\"libs/core\" = { url = \"https://github.com/org/core.git\", revision = \"main\" }\n",
    );
    for spelling in ["libs/core/", "./libs/core", "libs//core"] {
        let out = env.run(&[
            "require",
            spelling,
            "https://github.com/org/other.git",
            "v2",
        ]);
        assert!(!out.success, "{} was added", spelling);
        let config = config_text(&env);
        assert!(!config.contains("other.git"), "{}: {}", spelling, config);
    }
}

#[test]
fn error_011_unrequire_refuses_an_undeclared_directory() {
    let env = TestEnv::new("remove_unknown_fails");
    env.write_config(
        r#"[repos]
"libs/core" = { url = "https://github.com/org/core.git", revision = "main" }
"#,
    );
    let before = config_text(&env);

    let out = env.run(&["unrequire", "nonexistent"]);
    assert!(!out.success);
    assert!(out.stderr.contains("not declared"), "{}", out.stderr);
    assert_eq!(config_text(&env), before);
}

/// A config that does not read is left exactly as it is.
#[test]
fn error_014_unrequire_leaves_an_unparseable_config_untouched() {
    let env = TestEnv::new("remove_unparseable");
    let broken = "[repos\n\"libs/a\" = { url = \"https://github.com/org/a.git\" }\n";
    env.write_config(broken);

    let out = env.run(&["unrequire", "libs/a"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("invalid TOML"), "{}", out.stderr);
    assert_eq!(config_text(&env), broken);
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    crate::support::run_git_pub(dir, args);
}
