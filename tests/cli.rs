mod helpers;

use helpers::{strip_ansi, TestEnv};

// ---------------------------------------------------------------------------
// Add / Remove
// ---------------------------------------------------------------------------

#[test]
fn add_entry() {
    let env = TestEnv::new("add_entry");
    env.write_config("");

    let out = env.run(&["add", "libs/core", "https://github.com/org/core.git", "main"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("add_entry_stdout", out.stdout);

    // Config file should contain the entry
    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    assert!(config.contains("libs/core"));
    assert!(config.contains("https://github.com/org/core.git"));
}

#[test]
fn add_entry_artefact() {
    let env = TestEnv::new("add_entry_artefact");
    env.write_config("");

    let out = env.run(&[
        "add", "meta/svc", "https://github.com/org/svc.git", "main", "--mode", "artefact",
    ]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("add_entry_artefact_stdout", out.stdout);

    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    assert!(config.contains("artefact"));
}

#[test]
fn add_duplicate_fails() {
    let env = TestEnv::new("add_duplicate_fails");
    env.write_config(
        r#"[repos]
"libs/core" = { url = "https://github.com/org/core.git", revision = "main" }
"#,
    );

    let out = env.run(&["add", "libs/core", "https://github.com/org/other.git", "main"]);
    assert!(!out.success);
    assert!(out.stderr.contains("already declared"));
}

#[test]
fn remove_entry() {
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

#[test]
fn remove_unknown_fails() {
    let env = TestEnv::new("remove_unknown_fails");
    env.write_config(
        r#"[repos]
"libs/core" = { url = "https://github.com/org/core.git", revision = "main" }
"#,
    );

    let out = env.run(&["remove", "nonexistent"]);
    assert!(!out.success);
    assert!(out.stderr.contains("not declared"));
}

// ---------------------------------------------------------------------------
// Clone
// ---------------------------------------------------------------------------

#[test]
fn clone_readwrite() {
    let env = TestEnv::new("clone_readwrite");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# mylib\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run(&["clone"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("clone_readwrite_stdout", out.stdout);

    // Directory created with file
    assert!(env.playground.join("libs/mylib/README.md").is_file());
}

#[test]
fn clone_readonly() {
    let env = TestEnv::new("clone_readonly");
    let bare = env.create_bare_repo("rolib", "main", &[("data.txt", "hello\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/rolib" = {{ url = "{}", revision = "main", mode = "readonly" }}
"#,
        bare.display()
    ));

    let out = env.run(&["clone"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("clone_readonly_stdout", out.stdout);

    let file = env.playground.join("libs/rolib/data.txt");
    assert!(file.is_file());

    // Check readonly permission
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::metadata(&file).unwrap().permissions();
    assert_eq!(perms.mode() & 0o222, 0, "file should be readonly");
}

#[test]
fn clone_artefact_local() {
    let env = TestEnv::new("clone_artefact_local");
    let repo_url = "https://github.com/org/app.git";

    // Pre-create artefact in local storage
    env.create_artefact(repo_url, "main", &[("app.bin", "binary-content")]);

    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"meta/app" = {{ url = "{}", revision = "main", mode = "artefact" }}
"#,
        env.storage_url(),
        repo_url,
    ));

    let out = env.run(&["clone"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("clone_artefact_local_stdout", out.stdout);

    assert!(env.playground.join("meta/app/app.bin").is_file());
    let content = std::fs::read_to_string(env.playground.join("meta/app/app.bin")).unwrap();
    assert_eq!(content, "binary-content");
}

#[test]
fn clone_skip_existing() {
    let env = TestEnv::new("clone_skip_existing");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# mylib\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    // First clone
    let out1 = env.run(&["clone"]);
    assert!(out1.success);

    // Second clone should skip
    let out2 = env.run(&["clone"]);
    assert!(out2.success);
    insta::assert_snapshot!("clone_skip_existing_stdout", out2.stdout);
}

#[test]
fn clone_no_storage() {
    let env = TestEnv::new("clone_no_storage");

    env.write_config(
        r#"[repos]
"meta/app" = { url = "https://github.com/org/app.git", revision = "main", mode = "artefact" }
"#,
    );

    let out = env.run(&["clone"]);
    assert!(!out.success);
    assert!(out.stderr.contains("no [storage] configured"));
}

#[test]
fn clone_artefact_no_data() {
    let env = TestEnv::new("clone_artefact_no_data");

    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"meta/app" = {{ url = "https://github.com/org/app.git", revision = "main", mode = "artefact" }}
"#,
        env.storage_url(),
    ));

    let out = env.run(&["clone"]);
    assert!(out.success);
    insta::assert_snapshot!("clone_artefact_no_data_stdout", out.stdout);
}

#[test]
fn clone_selective() {
    let env = TestEnv::new("clone_selective");
    let bare1 = env.create_bare_repo("lib1", "main", &[("a.txt", "a")]);
    let bare2 = env.create_bare_repo("lib2", "main", &[("b.txt", "b")]);

    env.write_config(&format!(
        r#"[repos]
"libs/lib1" = {{ url = "{}", revision = "main" }}
"libs/lib2" = {{ url = "{}", revision = "main" }}
"#,
        bare1.display(),
        bare2.display(),
    ));

    let out = env.run(&["clone", "libs/lib1"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(env.playground.join("libs/lib1/a.txt").is_file());
    assert!(!env.playground.join("libs/lib2").exists());
}

#[test]
fn clone_unknown_name() {
    let env = TestEnv::new("clone_unknown_name");
    let bare = env.create_bare_repo("lib1", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/lib1" = {{ url = "{}", revision = "main" }}
"#,
        bare.display(),
    ));

    let out = env.run(&["clone", "nonexistent"]);
    assert!(!out.success);
    assert!(out.stderr.contains("Unknown repos"));
}

// ---------------------------------------------------------------------------
// Fetch
// ---------------------------------------------------------------------------

#[test]
fn fetch_artefact_local() {
    let env = TestEnv::new("fetch_artefact_local");
    let repo_url = "https://github.com/org/app.git";

    env.create_artefact(repo_url, "v1.0", &[("app.bin", "v1-content")]);

    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"meta/app" = {{ url = "{}", revision = "v1.0", mode = "artefact" }}
"#,
        env.storage_url(),
        repo_url,
    ));

    let out = env.run(&["fetch"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("fetch_artefact_local_stdout", out.stdout);

    // .etag-remote should be written
    assert!(env.playground.join("meta/app/.etag-remote").is_file());
}

#[test]
fn fetch_git_not_cloned() {
    let env = TestEnv::new("fetch_git_not_cloned");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run(&["fetch"]);
    assert!(out.success);
    insta::assert_snapshot!("fetch_git_not_cloned_stdout", out.stdout);
}

#[test]
fn fetch_git_cloned() {
    let env = TestEnv::new("fetch_git_cloned");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    env.run(&["clone"]);
    let out = env.run(&["fetch"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("fetch_git_cloned_stdout", out.stdout);
}

// ---------------------------------------------------------------------------
// Pull
// ---------------------------------------------------------------------------

#[test]
fn pull_artefact_local() {
    let env = TestEnv::new("pull_artefact_local");
    let repo_url = "https://github.com/org/app.git";

    env.create_artefact(repo_url, "main", &[("app.bin", "v1-content")]);

    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"meta/app" = {{ url = "{}", revision = "main", mode = "artefact" }}
"#,
        env.storage_url(),
        repo_url,
    ));

    // Clone first
    let out1 = env.run(&["clone"]);
    assert!(out1.success, "clone stderr: {}", out1.stderr);

    // Update artefact
    env.create_artefact(repo_url, "main", &[("app.bin", "v2-content")]);

    // Pull should download newer version
    let out2 = env.run(&["pull"]);
    assert!(out2.success, "pull stderr: {}", out2.stderr);
    insta::assert_snapshot!("pull_artefact_local_stdout", out2.stdout);

    let content = std::fs::read_to_string(env.playground.join("meta/app/app.bin")).unwrap();
    assert_eq!(content, "v2-content");
}

#[test]
fn pull_artefact_up_to_date() {
    let env = TestEnv::new("pull_artefact_up_to_date");
    let repo_url = "https://github.com/org/app.git";

    env.create_artefact(repo_url, "main", &[("app.bin", "content")]);

    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"meta/app" = {{ url = "{}", revision = "main", mode = "artefact" }}
"#,
        env.storage_url(),
        repo_url,
    ));

    env.run(&["clone"]);

    // Pull again — should be up to date (same etag)
    let out = env.run(&["pull"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("pull_artefact_up_to_date_stdout", out.stdout);
}

#[test]
fn pull_readwrite() {
    let env = TestEnv::new("pull_readwrite");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# v1\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    env.run(&["clone"]);
    let out = env.run(&["pull"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("pull_readwrite_stdout", out.stdout);
}

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

#[test]
fn push_skip_readonly() {
    let env = TestEnv::new("push_skip_readonly");
    let bare = env.create_bare_repo("rolib", "main", &[("data.txt", "hello\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/rolib" = {{ url = "{}", revision = "main", mode = "readonly" }}
"#,
        bare.display()
    ));

    env.run(&["clone"]);
    let out = env.run(&["push"]);
    assert!(out.success);
    insta::assert_snapshot!("push_skip_readonly_stdout", out.stdout);
}

#[test]
fn push_skip_artefact() {
    let env = TestEnv::new("push_skip_artefact");
    let repo_url = "https://github.com/org/app.git";
    env.create_artefact(repo_url, "main", &[("app.bin", "content")]);

    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"meta/app" = {{ url = "{}", revision = "main", mode = "artefact" }}
"#,
        env.storage_url(),
        repo_url,
    ));

    env.run(&["clone"]);
    let out = env.run(&["push"]);
    assert!(out.success);
    insta::assert_snapshot!("push_skip_artefact_stdout", out.stdout);
}

#[test]
fn push_readwrite() {
    let env = TestEnv::new("push_readwrite");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# v1\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    env.run(&["clone"]);
    let out = env.run(&["push"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("push_readwrite_stdout", out.stdout);
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[test]
fn status_table_missed() {
    let env = TestEnv::new("status_table_missed");
    env.init_playground_git();
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run(&["status"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    insta::assert_snapshot!("status_table_missed_stdout", plain);
}

#[test]
fn status_table_ok() {
    let env = TestEnv::new("status_table_ok");
    env.init_playground_git();
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    env.run(&["clone"]);
    let out = env.run(&["status"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    insta::assert_snapshot!("status_table_ok_stdout", plain);
}

#[test]
fn status_json() {
    let env = TestEnv::new("status_json");
    env.init_playground_git();
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    env.run(&["clone"]);
    let out = env.run(&["status", "--format", "json"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // Parse to validate JSON, then snapshot
    let parsed: serde_json::Value = serde_json::from_str(&out.stdout).expect("valid JSON");
    // Redact dynamic fields for stable snapshots
    insta::assert_json_snapshot!("status_json_output", parsed, {
        "[].current_ref" => "[ref]",
    });
}

#[test]
fn status_artefact_missed() {
    let env = TestEnv::new("status_artefact_missed");
    env.init_playground_git();
    let repo_url = "https://github.com/org/app.git";

    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"meta/app" = {{ url = "{}", revision = "main", mode = "artefact" }}
"#,
        env.storage_url(),
        repo_url,
    ));

    let out = env.run(&["status"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    insta::assert_snapshot!("status_artefact_missed_stdout", plain);
}

#[test]
fn status_artefact_ok() {
    let env = TestEnv::new("status_artefact_ok");
    env.init_playground_git();
    let repo_url = "https://github.com/org/app.git";
    env.create_artefact(repo_url, "main", &[("app.bin", "content")]);

    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"meta/app" = {{ url = "{}", revision = "main", mode = "artefact" }}
"#,
        env.storage_url(),
        repo_url,
    ));

    env.run(&["clone"]);
    let out = env.run(&["status"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    insta::assert_snapshot!("status_artefact_ok_stdout", plain);
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

#[test]
fn sync_full() {
    let env = TestEnv::new("sync_full");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# mylib\n")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("sync_full_stdout", out.stdout);

    assert!(env.playground.join("libs/mylib/README.md").is_file());
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

#[test]
fn hooks_post_sync() {
    let env = TestEnv::new("hooks_post_sync");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[hooks]
post_sync = "touch .hook-ran"

[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run(&["pull"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // Hook should have created this file
    assert!(env.playground.join(".hook-ran").exists());
}

#[test]
fn hooks_post_sync_failure() {
    let env = TestEnv::new("hooks_post_sync_failure");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[hooks]
post_sync = "exit 1"

[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run(&["pull"]);
    assert!(!out.success);
    assert!(out.stderr.contains("post_sync hook failed"));
}

// ---------------------------------------------------------------------------
// Multiple repos with mixed modes
// ---------------------------------------------------------------------------

#[test]
fn multiple_repos_mixed() {
    let env = TestEnv::new("multiple_repos_mixed");
    let bare_rw = env.create_bare_repo("rw-lib", "main", &[("rw.txt", "readwrite")]);
    let bare_ro = env.create_bare_repo("ro-lib", "main", &[("ro.txt", "readonly")]);
    let art_url = "https://github.com/org/art.git";
    env.create_artefact(art_url, "v1", &[("art.bin", "artefact-data")]);

    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"libs/ro-lib" = {{ url = "{}", revision = "main", mode = "readonly" }}
"libs/rw-lib" = {{ url = "{}", revision = "main" }}
"meta/art" = {{ url = "{}", revision = "v1", mode = "artefact" }}
"#,
        env.storage_url(),
        bare_ro.display(),
        bare_rw.display(),
        art_url,
    ));

    let out = env.run(&["clone"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("multiple_repos_mixed_stdout", out.stdout);

    assert!(env.playground.join("libs/rw-lib/rw.txt").is_file());
    assert!(env.playground.join("libs/ro-lib/ro.txt").is_file());
    assert!(env.playground.join("meta/art/art.bin").is_file());
}
