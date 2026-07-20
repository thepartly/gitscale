mod helpers;

use helpers::{strip_ansi, TestEnv};

// ---------------------------------------------------------------------------
// Add / Remove
// ---------------------------------------------------------------------------

#[test]
fn add_entry() {
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
fn add_entry_artefact() {
    let env = TestEnv::new("add_entry_artefact");
    env.write_config("");

    let out = env.run(&[
        "add",
        "meta/svc",
        "https://github.com/org/svc.git",
        "main",
        "--mode",
        "artefact",
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

    let out = env.run(&[
        "add",
        "libs/core",
        "https://github.com/org/other.git",
        "main",
    ]);
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

// ---------------------------------------------------------------------------
// Recursive dependency resolution
// ---------------------------------------------------------------------------

#[test]
fn recursive_basic_symlink() {
    let env = TestEnv::new("recursive_basic_symlink");

    // Create bare repo B
    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "hello from B")]);

    // Create bare repo A that declares B as a dependency
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "hello from A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );

    // Root config declares both
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"repoB" = {{ url = "{}", revision = "main" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));

    let out = env.run(&["clone"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // repoA/libs/b should be a symlink pointing to ../../repoB
    let link = env.playground.join("repoA/libs/b");
    assert!(link.is_symlink(), "expected symlink at repoA/libs/b");
    let target = std::fs::read_link(&link).unwrap();
    assert_eq!(
        target,
        std::path::PathBuf::from("../../repoB"),
        "symlink should be relative"
    );

    // Content accessible through symlink
    assert!(link.join("b.txt").is_file());
    let content = std::fs::read_to_string(link.join("b.txt")).unwrap();
    assert_eq!(content, "hello from B");
}

#[test]
fn recursive_missing_dep_errors() {
    let env = TestEnv::new("recursive_missing_dep_errors");

    // B is NOT declared at root level
    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "B")]);

    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );

    // Root only declares A, not B
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"#,
        bare_a.display(),
    ));

    let out = env.run(&["clone"]);
    assert!(!out.success, "should fail when child dep is not in root");
    assert!(
        out.stderr.contains("not declared in the root"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn recursive_revision_adoption() {
    let env = TestEnv::new("recursive_revision_adoption");

    // Create B with two branches
    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "B main")]);
    // Add a develop branch
    {
        let tmp = env.repos_remote.join("repoB-checkout");
        let _ = std::fs::remove_dir_all(&tmp);
        helpers::run_git_pub(
            &env.repos_remote,
            &["clone", bare_b.to_str().unwrap(), tmp.to_str().unwrap()],
        );
        helpers::run_git_pub(&tmp, &["config", "user.email", "t@t.com"]);
        helpers::run_git_pub(&tmp, &["config", "user.name", "T"]);
        helpers::run_git_pub(&tmp, &["checkout", "-b", "develop"]);
        std::fs::write(tmp.join("b.txt"), "B develop").unwrap();
        helpers::run_git_pub(&tmp, &["add", "."]);
        helpers::run_git_pub(&tmp, &["commit", "-m", "dev"]);
        helpers::run_git_pub(&tmp, &["push", "origin", "develop"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // A's config says it needs B at "develop"
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"develop\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );

    // Root declares B WITHOUT a revision (defer to child)
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"repoB" = {{ url = "{}" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));

    let out = env.run(&["clone"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // B should be checked out on "develop"
    let b_txt = std::fs::read_to_string(env.playground.join("repoB/b.txt")).unwrap();
    assert_eq!(b_txt, "B develop");

    // Symlink should exist
    let link = env.playground.join("repoA/libs/b");
    assert!(link.is_symlink());
}

#[test]
fn recursive_revision_conflict() {
    let env = TestEnv::new("recursive_revision_conflict");

    let bare_c = env.create_bare_repo("repoC", "main", &[("c.txt", "C")]);

    // A wants C at "main"
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/c\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    bare_c.display()
                ),
            ),
        ],
    );

    // B wants C at "develop" (different)
    let bare_b = env.create_bare_repo(
        "repoB",
        "main",
        &[
            ("b.txt", "B"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"deps/c\" = {{ url = \"{}\", revision = \"develop\" }}\n",
                    bare_c.display()
                ),
            ),
        ],
    );

    // Root declares all three, C without revision
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"repoB" = {{ url = "{}", revision = "main" }}
"repoC" = {{ url = "{}" }}
"#,
        bare_a.display(),
        bare_b.display(),
        bare_c.display(),
    ));

    let out = env.run(&["clone"]);
    assert!(!out.success, "should fail on conflicting revisions");
    assert!(
        out.stderr.contains("conflicting revisions"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn recursive_disabled() {
    let env = TestEnv::new("recursive_disabled");

    // B isn't declared at root, but A references it
    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "B")]);

    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );

    // Root declares A with recursive = false — B is not needed
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main", recursive = false }}
"#,
        bare_a.display(),
    ));

    let out = env.run(&["clone"]);
    assert!(
        out.success,
        "should succeed because recursion is disabled: stderr: {}",
        out.stderr
    );

    // No symlink should be created
    assert!(!env.playground.join("repoA/libs/b").exists());
}

#[test]
fn recursive_root_revision_wins() {
    let env = TestEnv::new("recursive_root_revision_wins");

    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "B main")]);

    // A wants B at "develop"
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"develop\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );

    // Root pins B at "main" — root wins, no conflict
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"repoB" = {{ url = "{}", revision = "main" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));

    let out = env.run(&["clone"]);
    assert!(
        out.success,
        "root revision wins, no error: stderr: {}",
        out.stderr
    );

    // B should be on main (root wins)
    let b_txt = std::fs::read_to_string(env.playground.join("repoB/b.txt")).unwrap();
    assert_eq!(b_txt, "B main");

    // Symlink should still be created
    let link = env.playground.join("repoA/libs/b");
    assert!(link.is_symlink());
}

#[test]
fn recursive_artefact_with_config() {
    let env = TestEnv::new("recursive_artefact_config");
    let repo_url_art = "https://github.com/org/art.git";
    let bare_dep = env.create_bare_repo("dep", "main", &[("dep.txt", "dep content")]);

    // Create artefact that contains a .gitscale.toml
    let child_config = format!(
        "[repos]\n\"vendor/dep\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare_dep.display()
    );
    env.create_artefact(
        repo_url_art,
        "v1",
        &[("art.bin", "binary"), (".gitscale.toml", &child_config)],
    );

    // Root declares both artefact and the dep
    env.write_config(&format!(
        r#"[storage]
url = "{}"

[repos]
"meta/art" = {{ url = "{}", revision = "v1", mode = "artefact" }}
"dep" = {{ url = "{}", revision = "main" }}
"#,
        env.storage_url(),
        repo_url_art,
        bare_dep.display(),
    ));

    let out = env.run(&["clone"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // Symlink inside artefact dir
    let link = env.playground.join("meta/art/vendor/dep");
    assert!(link.is_symlink(), "expected symlink inside artefact");
    assert!(link.join("dep.txt").is_file());
}

// ---------------------------------------------------------------------------
// Relink
// ---------------------------------------------------------------------------

/// Helper: set up a workspace with repoA (recursive) depending on repoB,
/// clone it (creating a symlink), then replace the symlink with a real clone.
/// Returns (env, path_to_link).
fn setup_unlinked_env(name: &str) -> (TestEnv, std::path::PathBuf) {
    let env = TestEnv::new(name);

    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "hello from B")]);
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "hello from A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );

    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main", recursive = true }}
"repoB" = {{ url = "{}", revision = "main" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));

    // Clone creates the symlink
    let out = env.run(&["clone"]);
    assert!(out.success, "clone failed: {}", out.stderr);

    let link = env.playground.join("repoA/libs/b");
    assert!(link.is_symlink(), "expected symlink after clone");

    // Replace symlink with a real clone (simulating `gitscale sync` from inside repoA)
    std::fs::remove_file(&link).unwrap();
    helpers::run_git_pub(
        &env.playground,
        &[
            "clone",
            "--branch",
            "main",
            bare_b.to_str().unwrap(),
            link.to_str().unwrap(),
        ],
    );
    assert!(!link.is_symlink(), "should be a real dir now");
    assert!(link.join("b.txt").is_file());

    (env, link)
}

#[test]
fn sync_relinks_clean_clone() {
    let (env, link) = setup_unlinked_env("sync_relinks_clean");

    // Sync should auto-relink since the clone is clean
    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(link.is_symlink(), "expected symlink restored after sync");
    let target = std::fs::read_link(&link).unwrap();
    assert_eq!(
        target,
        std::path::PathBuf::from("../../repoB"),
        "symlink target mismatch: {:?}",
        target
    );
    // Verify content is accessible through the symlink
    assert!(
        env.playground.join("repoB/b.txt").is_file(),
        "repoB/b.txt should exist"
    );
    assert!(
        link.join("b.txt").is_file(),
        "b.txt not accessible through symlink; link={:?}, target={:?}, exists={}, is_dir={}",
        link,
        target,
        link.exists(),
        link.is_dir()
    );
}

#[test]
fn sync_skips_modified_clone_without_force() {
    let (env, link) = setup_unlinked_env("sync_skips_modified");

    // Make the clone dirty (uncommitted changes)
    std::fs::write(link.join("dirty.txt"), "local change").unwrap();
    helpers::run_git_pub(&link, &["add", "."]);

    // Sync without --force should fail (non-zero exit)
    let out = env.run(&["sync"]);
    assert!(
        !out.success,
        "expected sync to fail when skipping modified unlinked clone"
    );
    assert!(
        out.stdout.contains("skip") || out.stderr.contains("skip"),
        "expected skip message, got stdout: {}, stderr: {}",
        out.stdout,
        out.stderr
    );

    assert!(
        !link.is_symlink(),
        "should still be a real dir (not relinked)"
    );
    assert!(link.join("dirty.txt").is_file());
}

#[test]
fn sync_force_relinks_modified_clone() {
    let (env, link) = setup_unlinked_env("sync_force_relinks_modified");

    // Make the clone dirty
    std::fs::write(link.join("dirty.txt"), "local change").unwrap();
    helpers::run_git_pub(&link, &["add", "."]);

    // Sync with --force should relink even though modified
    let out = env.run(&["sync", "--force"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(link.is_symlink(), "expected symlink restored with --force");
    let target = std::fs::read_link(&link).unwrap();
    assert_eq!(target, std::path::PathBuf::from("../../repoB"));
    // dirty.txt should be gone (it was in the clone that got removed)
    assert!(!link.join("dirty.txt").exists());
}

#[test]
fn sync_skips_clone_with_unpushed_commits() {
    let (env, link) = setup_unlinked_env("sync_skips_unpushed");

    // Make a commit that isn't pushed
    helpers::run_git_pub(&link, &["config", "user.email", "t@t.com"]);
    helpers::run_git_pub(&link, &["config", "user.name", "T"]);
    std::fs::write(link.join("new.txt"), "new file").unwrap();
    helpers::run_git_pub(&link, &["add", "."]);
    helpers::run_git_pub(&link, &["commit", "-m", "unpushed"]);

    // Sync without --force should fail (non-zero exit)
    let out = env.run(&["sync"]);
    assert!(
        !out.success,
        "expected sync to fail when skipping unlinked clone with unpushed commits"
    );

    assert!(
        !link.is_symlink(),
        "should still be a real dir (unpushed commits)"
    );
    assert!(link.join("new.txt").is_file());
}

#[test]
fn sync_force_relinks_clone_with_unpushed_commits() {
    let (env, link) = setup_unlinked_env("sync_force_unpushed");

    // Make a commit that isn't pushed
    helpers::run_git_pub(&link, &["config", "user.email", "t@t.com"]);
    helpers::run_git_pub(&link, &["config", "user.name", "T"]);
    std::fs::write(link.join("new.txt"), "new file").unwrap();
    helpers::run_git_pub(&link, &["add", "."]);
    helpers::run_git_pub(&link, &["commit", "-m", "unpushed"]);

    // Sync with --force should relink
    let out = env.run(&["sync", "--force"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(link.is_symlink(), "expected symlink restored with --force");
}

#[test]
fn status_shows_unlinked() {
    let (env, _link) = setup_unlinked_env("status_shows_unlinked");

    let out = env.run(&["status"]);
    assert!(out.success, "stderr: {}", out.stderr);

    let plain = strip_ansi(&out.stdout);
    assert!(
        plain.contains("unlinked"),
        "expected 'unlinked' in status output: {}",
        plain
    );
}

#[test]
fn status_shows_unlinked_modified() {
    let (env, link) = setup_unlinked_env("status_shows_unlinked_modified");

    // Make dirty
    std::fs::write(link.join("dirty.txt"), "change").unwrap();
    helpers::run_git_pub(&link, &["add", "."]);

    let out = env.run(&["status"]);
    assert!(out.success, "stderr: {}", out.stderr);

    let plain = strip_ansi(&out.stdout);
    assert!(plain.contains("unlinked"), "expected 'unlinked': {}", plain);
    assert!(plain.contains("modified"), "expected 'modified': {}", plain);
}

// ---------------------------------------------------------------------------
// Orphaned symlinks
// ---------------------------------------------------------------------------

/// Helper: set up a workspace with repoA (recursive) depending on repoB, clone
/// it, then plant an orphaned gitscale-style symlink `repoA/libs/c -> ../../repoC`
/// for a dependency that is not declared anywhere (simulating a removed dep).
/// When `valid_target` is true the target `repoC` dir is created so the orphan
/// resolves; otherwise the orphan is left broken. Returns (env, orphan_link).
fn setup_orphan_env(name: &str, valid_target: bool) -> (TestEnv, std::path::PathBuf) {
    let env = TestEnv::new(name);

    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "hello from B")]);
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "hello from A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"main\" }}\n",
                    bare_b.display()
                ),
            ),
        ],
    );

    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main", recursive = true }}
"repoB" = {{ url = "{}", revision = "main" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));

    let out = env.run(&["clone"]);
    assert!(out.success, "clone failed: {}", out.stderr);

    // Plant an orphaned gitscale-style symlink for an undeclared dependency.
    let orphan = env.playground.join("repoA/libs/c");
    std::os::unix::fs::symlink("../../repoC", &orphan).unwrap();
    if valid_target {
        std::fs::create_dir_all(env.playground.join("repoC")).unwrap();
        std::fs::write(env.playground.join("repoC/c.txt"), "hello from C").unwrap();
    }

    (env, orphan)
}

#[test]
fn status_shows_orphan() {
    let (env, _orphan) = setup_orphan_env("status_shows_orphan", true);

    let out = env.run(&["status"]);
    assert!(out.success, "stderr: {}", out.stderr);

    let plain = strip_ansi(&out.stdout);
    assert!(
        plain.contains("orphan"),
        "expected 'orphan' in status output: {}",
        plain
    );
}

#[test]
fn sync_removes_broken_orphan_by_default() {
    let (env, orphan) = setup_orphan_env("sync_removes_broken_orphan", false);

    // A broken orphan (target missing) is removed automatically, no --force.
    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(
        std::fs::symlink_metadata(&orphan).is_err(),
        "expected broken orphan symlink to be removed"
    );
}

#[test]
fn sync_skips_orphan_with_valid_target_without_force() {
    let (env, orphan) = setup_orphan_env("sync_skips_orphan_valid", true);

    // An orphan whose target still resolves requires --force.
    let out = env.run(&["sync"]);
    assert!(
        !out.success,
        "expected sync to fail when skipping orphan with valid target"
    );
    assert!(
        out.stdout.contains("orphan") || out.stderr.contains("orphan"),
        "expected orphan message, stdout: {}, stderr: {}",
        out.stdout,
        out.stderr
    );
    assert!(
        std::fs::symlink_metadata(&orphan)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false),
        "orphan symlink with valid target should remain without --force"
    );
}

#[test]
fn sync_force_removes_orphan_with_valid_target() {
    let (env, orphan) = setup_orphan_env("sync_force_removes_orphan", true);

    let out = env.run(&["sync", "--force"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(
        std::fs::symlink_metadata(&orphan).is_err(),
        "expected orphan symlink to be removed with --force"
    );
}
