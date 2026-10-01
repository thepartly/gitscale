//! Artefact mode end to end: `artefact publish` into a fake registry, and
//! every consumer command against it — what each one downloads, what it
//! refuses, and what it asks the registry for.

#[allow(dead_code)]
mod helpers;

use helpers::{git_stdout, redact_shas, run_git_pub, strip_ansi, TestEnv};
use std::path::{Path, PathBuf};

/// Two groups: a `vendor` layer that rarely changes and an `app` layer that
/// changes with every build.
const LAYERED: &str = "[artefact]\nroot = \"dist\"\n\n[[artefact.layer]]\nname = \"vendor\"\ninclude = [\"vendor/**\"]\n\n\
                       [[artefact.layer]]\nname = \"app\"\ninclude = [\"**\"]\n";

fn entry_config(env: &TestEnv, bare: &Path, revision: &str) -> String {
    format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"{}\", mode = \"artefact\" }}\n",
        env.registries(),
        bare.display(),
        revision
    )
}

fn read(env: &TestEnv, rel: &str) -> String {
    std::fs::read_to_string(env.playground.join(rel)).unwrap()
}

fn layered(env: &TestEnv, bare: &Path, app: &str) -> String {
    let (out, commit) = env.publish_with(
        bare,
        "main",
        LAYERED,
        &[("vendor/lib.js", "vendor v1"), ("app.js", app)],
        &[],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    commit
}

/// The commit `status` says `meta/app` was installed from.
fn installed_commit(env: &TestEnv) -> String {
    let out = env.run(&["status", "--format", "json"]);
    let rows: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    rows.as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "meta/app")
        .and_then(|r| r["artefact"]["installed"]["commit"].as_str())
        .unwrap_or_default()
        .to_string()
}

fn blob_downloads(env: &TestEnv) -> usize {
    env.registry().count("GET", "/blobs/sha256:")
}

// ---------------------------------------------------------------------------
// What gets published
// ---------------------------------------------------------------------------

#[test]
fn publish_tags_the_commit_and_annotates_the_image() {
    let env = TestEnv::new("art_publish_tags");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let commit = layered(&env, &bare, "app v1");

    assert_eq!(env.registry().tags(&env.image(&bare)), vec![commit.clone()]);
    let manifest = env.registry().manifest(&env.image(&bare), &commit).unwrap();
    assert_eq!(
        manifest["annotations"]["org.opencontainers.image.revision"],
        commit.as_str()
    );
    assert_eq!(
        manifest["annotations"]["org.opencontainers.image.source"],
        bare.display().to_string()
    );
    let titles: Vec<&str> = manifest["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            l["annotations"]["org.opencontainers.image.title"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(titles, vec!["vendor", "app"]);
}

#[test]
fn an_identical_republish_uploads_nothing() {
    let env = TestEnv::new("art_republish_same");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "app v1");
    env.registry().clear_log();

    let (out, _) = env.publish_with(
        &bare,
        "main",
        LAYERED,
        &[("vendor/lib.js", "vendor v1"), ("app.js", "app v1")],
        &[],
    );
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("Already published"), "{}", out.stdout);
    assert_eq!(env.registry().count("POST", "/blobs/uploads/"), 0);
    assert_eq!(env.registry().count("PUT", ""), 0);
}

#[test]
fn a_different_build_of_a_published_commit_needs_force() {
    let env = TestEnv::new("art_republish_force");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "app v1");

    let rebuild = [("vendor/lib.js", "vendor v1"), ("app.js", "app rebuilt")];
    let (out, _) = env.publish_with(&bare, "main", LAYERED, &rebuild, &[]);
    assert!(!out.success, "an overwrite was not asked for");
    assert!(out.stderr.contains("--force"), "{}", out.stderr);

    // Only the layer that changed goes up.
    env.registry().clear_log();
    let (out, _) = env.publish_with(&bare, "main", LAYERED, &rebuild, &["--force"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        out.stdout.contains("vendor already in the registry"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("pushed app"), "{}", out.stdout);
}

#[test]
fn a_dry_run_lists_the_layers_and_sends_nothing() {
    let env = TestEnv::new("art_dry_run");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let (out, _) = env.publish_with(
        &bare,
        "main",
        LAYERED,
        &[
            ("vendor/lib.js", "v"),
            ("app.js", "a"),
            ("css/site.css", "c"),
        ],
        &["--dry-run"],
    );
    assert!(out.success, "{}", out.stderr);
    let text = redact_shas(&out.stdout);
    assert!(text.contains("Would publish"), "{}", text);
    assert!(text.contains("layer vendor: 1 file"), "{}", text);
    assert!(text.contains("layer app: 2 files"), "{}", text);
    assert!(text.contains("    css/site.css"), "{}", text);
    assert!(
        env.registry().log().is_empty(),
        "{:?}",
        env.registry().log()
    );
}

/// The way to see what would ship: a dry run lists the files even where the
/// image cannot be worked out yet — no registry mapping, no commit.
#[test]
fn a_dry_run_lists_files_without_a_registry() {
    let env = TestEnv::new("art_dry_run_offline");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let producer = env.producer(&bare, "main");
    std::fs::create_dir_all(producer.join("dist/.well-known")).unwrap();
    std::fs::write(producer.join("dist/index.html"), "i").unwrap();
    std::fs::write(producer.join("dist/.well-known/security.txt"), "s").unwrap();
    std::fs::write(
        producer.join(".gitscale.toml"),
        "[artefact]\nroot = \"dist\"\ninclude = [\"**\"]\n",
    )
    .unwrap();
    let out = env.run_in(&producer, &["artefact", "publish", "--dry-run"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        out.stdout.contains("image: unknown (no registry is known"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("    index.html"), "{}", out.stdout);
    assert!(
        out.stdout.contains("    .well-known/security.txt"),
        "{}",
        out.stdout
    );
    // A real publish still needs both.
    let out = env.run_in(&producer, &["artefact", "publish"]);
    assert!(!out.success);
}

#[test]
fn a_group_that_matches_nothing_fails_the_publish() {
    let env = TestEnv::new("art_empty_group");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let (out, _) = env.publish_with(&bare, "main", LAYERED, &[("app.js", "a")], &[]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("\"vendor\" matches no files"),
        "{}",
        out.stderr
    );
    assert!(env.registry().tags(&env.image(&bare)).is_empty());
}

#[test]
fn publishing_needs_an_artefact_table() {
    let env = TestEnv::new("art_no_table");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let producer = env.producer(&bare, "main");
    std::fs::write(producer.join(".gitscale.toml"), env.registries()).unwrap();
    let out = env.run_in(&producer, &["artefact", "publish"]);
    assert!(!out.success);
    assert!(out.stderr.contains("no [artefact] table"), "{}", out.stderr);
}

// ---------------------------------------------------------------------------
// Resolving a revision
// ---------------------------------------------------------------------------

#[test]
fn an_annotated_tag_resolves_to_its_commit() {
    let env = TestEnv::new("art_annotated_tag");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(
        &bare,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "tag",
            "-a",
            "-m",
            "release",
            "v2.0",
            "main",
        ],
    );
    let commit = env.publish(&bare, "v2.0", &[("app.bin", "tagged")]);
    env.write_config(&entry_config(&env, &bare, "v2.0"));

    let out = env.run(&["clone"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/app.bin"), "tagged");
    assert_eq!(installed_commit(&env), commit);
}

#[test]
fn a_full_sha_is_used_as_given() {
    let env = TestEnv::new("art_full_sha");
    let bare = env.artefact_repo("app", &[("app.bin", "pinned")]);
    let commit = git_stdout(&bare, &["rev-parse", "main"]);
    // Main moves on, unpublished: the pinned commit is unaffected.
    env.push_commit(&bare, "main", "README.md", "later");
    env.write_config(&entry_config(&env, &bare, &commit));

    let out = env.run(&["clone"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/app.bin"), "pinned");
}

/// An abbreviated commit is taken for a name the remote does not have, and
/// the failure says to give the full SHA.
#[test]
fn an_abbreviated_sha_is_refused_with_directions() {
    let env = TestEnv::new("art_short_sha");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    let short = &git_stdout(&bare, &["rev-parse", "main"])[..9];
    env.write_config(&entry_config(&env, &bare, short));

    let out = env.run(&["clone"]);
    assert!(!out.success);
    assert!(out.stderr.contains("full SHA"), "{}", out.stderr);
    assert!(
        out.stderr.contains(&format!("git rev-parse {}", short)),
        "{}",
        out.stderr
    );
}

#[test]
fn an_all_hex_tag_name_is_a_tag() {
    let env = TestEnv::new("art_hex_tag");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "20241001", "main"]);
    env.publish(&bare, "20241001", &[("app.bin", "dated")]);
    env.write_config(&entry_config(&env, &bare, "20241001"));
    let out = env.run(&["clone"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/app.bin"), "dated");
}

#[test]
fn no_revision_follows_the_default_branch() {
    let env = TestEnv::new("art_default_branch");
    let bare = env.create_bare_repo("app", "trunk", &[("README.md", "app")]);
    env.publish(&bare, "trunk", &[("app.bin", "from trunk")]);
    env.write_config(&format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", mode = \"artefact\" }}\n",
        env.registries(),
        bare.display()
    ));

    let out = env.run(&["clone"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/app.bin"), "from trunk");
}

#[test]
fn a_branch_name_matches_exactly() {
    let env = TestEnv::new("art_exact_branch");
    let bare = env.artefact_repo("app", &[("app.bin", "main")]);
    // `ls-remote <url> main` would also list this one.
    run_git_pub(&bare, &["branch", "feature/main", "main"]);
    env.push_commit(&bare, "feature/main", "README.md", "f");
    env.write_config(&entry_config(&env, &bare, "main"));

    let out = env.run(&["clone"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/app.bin"), "main");
}

// ---------------------------------------------------------------------------
// What crosses the wire
// ---------------------------------------------------------------------------

#[test]
fn a_pull_downloads_only_the_layer_that_changed() {
    let env = TestEnv::new("art_layer_pull");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);
    assert_eq!(blob_downloads(&env), 2);

    env.push_commit(&bare, "main", "README.md", "v2");
    layered(&env, &bare, "app v2");
    env.registry().clear_log();
    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/app.js"), "app v2");
    assert_eq!(read(&env, "meta/app/vendor/lib.js"), "vendor v1");
    assert_eq!(blob_downloads(&env), 1, "{:?}", env.registry().log());
}

#[test]
fn a_second_workspace_downloads_nothing() {
    let env = TestEnv::new("art_second_ws");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);

    // Another workspace on the same machine, same cache.
    std::fs::remove_dir_all(env.playground.join("meta")).unwrap();
    env.registry().clear_log();
    let out = env.run(&["clone"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/app.js"), "app v1");
    assert_eq!(blob_downloads(&env), 0, "{:?}", env.registry().log());
    assert_eq!(env.registry().count("GET", "/manifests/"), 0);
}

#[test]
fn without_the_cache_every_layer_is_downloaded() {
    let env = TestEnv::new("art_no_cache");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "app v1");
    env.write_config(&format!(
        "[cache]\nenabled = false\n\n{}",
        entry_config(&env, &bare, "main")
    ));
    for _ in 0..2 {
        let _ = std::fs::remove_dir_all(env.playground.join("meta"));
        assert!(env.run(&["clone"]).success);
    }
    assert_eq!(blob_downloads(&env), 4);
    assert!(env.cache_entries("artefacts").is_empty());
}

#[test]
fn parallel_cold_clones_download_each_blob_once() {
    let env = TestEnv::new("art_parallel");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "app v1");
    let workspaces: Vec<PathBuf> = (0..4)
        .map(|i| env.playground.join(format!("ws{}", i)))
        .collect();
    for ws in &workspaces {
        std::fs::create_dir_all(ws).unwrap();
        std::fs::write(
            ws.join(".gitscale.toml"),
            format!(
                "[cache]\ndir = \"{}\"\n\n{}",
                env.cache.display(),
                entry_config(&env, &bare, "main")
            ),
        )
        .unwrap();
    }
    std::thread::scope(|scope| {
        for ws in &workspaces {
            scope.spawn(move || {
                let out = gitscale::run_cli_with(
                    &["gitscale", "clone", "-C", ws.to_str().unwrap()],
                    false,
                );
                assert!(out.success, "{}", out.stderr);
            });
        }
    });
    for ws in &workspaces {
        assert_eq!(
            std::fs::read_to_string(ws.join("meta/app/app.js")).unwrap(),
            "app v1"
        );
    }
    assert_eq!(blob_downloads(&env), 2, "{:?}", env.registry().log());
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

fn status_line(env: &TestEnv) -> String {
    let out = env.run(&["status"]);
    assert!(out.success, "{}", out.stderr);
    strip_ansi(&out.stdout)
        .lines()
        .find(|l| l.contains("meta/app"))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn status_says_behind_and_missing_after_a_fetch() {
    let env = TestEnv::new("art_status_behind");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));

    // A new commit whose pipeline has not published yet.
    env.push_commit(&bare, "main", "README.md", "v2");
    let fetch = env.run(&["fetch"]);
    assert!(!fetch.success, "a missing artefact is an error");
    assert!(
        fetch.stderr.contains("may not have published yet"),
        "{}",
        fetch.stderr
    );
    assert!(
        status_line(&env).ends_with("behind, missing"),
        "{}",
        status_line(&env)
    );

    // Published now: just behind, until a pull.
    env.publish(&bare, "main", &[("app.bin", "v2")]);
    assert!(env.run(&["fetch"]).success);
    assert!(
        status_line(&env).ends_with("behind"),
        "{}",
        status_line(&env)
    );
    assert!(env.run(&["pull"]).success);
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));
    assert_eq!(read(&env, "meta/app/app.bin"), "v2");
}

#[test]
fn a_republished_commit_shows_as_changed_and_pull_takes_it() {
    let env = TestEnv::new("art_status_changed");
    let bare = env.artefact_repo("app", &[("app.bin", "first build")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);

    let artefact = "[artefact]\nroot = \"dist\"\ninclude = [\"**\"]\n";
    let (out, _) = env.publish_with(
        &bare,
        "main",
        artefact,
        &[("app.bin", "second build")],
        &["--force"],
    );
    assert!(out.success, "{}", out.stderr);
    // A pull alone asks the registry nothing: the commit has not moved.
    assert!(env.run(&["pull"]).success);
    assert_eq!(read(&env, "meta/app/app.bin"), "first build");

    assert!(env.run(&["fetch"]).success);
    assert!(
        status_line(&env).ends_with("changed"),
        "{}",
        status_line(&env)
    );
    assert!(env.run(&["pull"]).success);
    assert_eq!(read(&env, "meta/app/app.bin"), "second build");
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));
}

#[test]
fn status_json_carries_the_installed_commit_and_digest() {
    let env = TestEnv::new("art_status_json");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    let commit = git_stdout(&bare, &["rev-parse", "main"]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);

    let out = env.run(&["status", "--format", "json"]);
    let rows: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "meta/app")
        .unwrap();
    assert_eq!(row["artefact"]["installed"]["commit"], commit.as_str());
    assert!(row["artefact"]["installed"]["digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
}

#[test]
fn changing_the_configured_revision_is_a_ref_mismatch() {
    let env = TestEnv::new("art_ref_mismatch");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    run_git_pub(&bare, &["tag", "v1", "main"]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);
    env.write_config(&entry_config(&env, &bare, "v1"));
    assert!(
        status_line(&env).ends_with("ref-mismatch"),
        "{}",
        status_line(&env)
    );
    assert!(env.run(&["pull"]).success);
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));
}

/// A directory gitscale has no record of installing — one from an older
/// gitscale, or made by hand — is replaced by `pull`, dot files and all.
#[test]
fn an_old_style_checkout_is_replaced_on_pull() {
    let env = TestEnv::new("art_legacy");
    let bare = env.artefact_repo("app", &[("app.bin", "new")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    let dest = env.playground.join("meta/app");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join(".etag"), "\"abc\"").unwrap();
    std::fs::write(dest.join("stale.bin"), "old").unwrap();

    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/app.bin"), "new");
    assert!(!dest.join("stale.bin").exists());
    assert!(!dest.join(".etag").exists());
}

/// What gitscale records about a checkout lives in the workspace's git
/// directory, never in the checkout, whose every file is the artefact's.
#[test]
fn records_live_in_the_git_directory_not_the_checkout() {
    let env = TestEnv::new("art_records");
    env.init_playground_git();
    let bare = env.artefact_repo("app", &[("app.bin", "x"), (".env", "y")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);

    let mut names: Vec<String> = std::fs::read_dir(env.playground.join("meta/app"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec![".env", "app.bin"]);
    let records = env.playground.join(".git/gitscale/artefacts");
    assert_eq!(
        std::fs::read_dir(&records).unwrap().count(),
        2,
        "installed and remote"
    );
    assert!(!env.playground.join(".gitscale").exists());
}

/// A workspace that is not the top of a git repository keeps them beside its
/// config instead.
#[test]
fn without_a_repository_records_live_beside_the_config() {
    let env = TestEnv::new("art_records_plain");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);
    assert!(env.playground.join(".gitscale/artefacts").is_dir());
    assert_eq!(
        installed_commit(&env),
        git_stdout(&bare, &["rev-parse", "main"])
    );
}

/// Deleting a checkout by hand is noticed: it is not installed any more,
/// whatever was recorded.
#[test]
fn a_deleted_checkout_is_cloned_again() {
    let env = TestEnv::new("art_deleted");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);
    std::fs::remove_dir_all(env.playground.join("meta/app")).unwrap();
    assert!(
        status_line(&env).ends_with("missed"),
        "{}",
        status_line(&env)
    );
    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert!(!out.stdout.contains("up to date"), "{}", out.stdout);
    assert_eq!(read(&env, "meta/app/app.bin"), "x");
}

// ---------------------------------------------------------------------------
// artefact show and artefact list
// ---------------------------------------------------------------------------

/// The value `artefact show` prints on `label`'s line for the first entry.
fn shown(text: &str, label: &str) -> String {
    text.lines()
        .find(|l| l.trim_start().starts_with(&format!("{} ", label)))
        .map(|l| l.trim_start()[label.len()..].trim().to_string())
        .unwrap_or_default()
}

#[test]
fn show_says_what_the_registry_and_the_checkout_hold() {
    let env = TestEnv::new("art_show");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let first = layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));

    let out = env.run(&["artefact", "show"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.starts_with("meta/app\n"), "{}", out.stdout);
    assert_eq!(shown(&out.stdout, "revision"), "main");
    assert_eq!(shown(&out.stdout, "commit"), first);
    assert!(
        shown(&out.stdout, "published").starts_with("sha256:"),
        "{}",
        out.stdout
    );
    assert!(
        shown(&out.stdout, "layers").starts_with("vendor "),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("app  "), "{}", out.stdout);
    assert_eq!(shown(&out.stdout, "installed"), "nothing");
    assert_eq!(shown(&out.stdout, "status"), "not installed");

    assert!(env.run(&["clone"]).success);
    let out = env.run(&["artefact", "show", "meta/app"]);
    assert!(
        shown(&out.stdout, "installed").starts_with(&first),
        "{}",
        out.stdout
    );
    assert_eq!(shown(&out.stdout, "status"), "ok");

    // A new commit nobody has published: show says so without a fetch, and
    // changes nothing.
    let second = env.push_commit(&bare, "main", "README.md", "v2");
    let out = env.run(&["artefact", "show"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(shown(&out.stdout, "commit"), second);
    assert!(
        shown(&out.stdout, "published").starts_with("no"),
        "{}",
        out.stdout
    );
    assert_eq!(shown(&out.stdout, "status"), "behind, missing");
    assert!(
        status_line(&env).ends_with("ok"),
        "show must not record anything"
    );
}

#[test]
fn show_reports_an_entry_it_cannot_look_up_and_fails() {
    let env = TestEnv::new("art_show_error");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    // No [registries]: nothing maps this repository to a registry.
    env.write_config(&format!(
        "[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"main\", mode = \"artefact\" }}\n",
        bare.display()
    ));
    let out = env.run(&["artefact", "show"]);
    assert!(!out.success);
    assert!(
        shown(&out.stdout, "error").contains("no registry is known"),
        "{}",
        out.stdout
    );
    assert!(
        out.stderr
            .contains("1 artefact entry could not be looked up"),
        "{}",
        out.stderr
    );
}

#[test]
fn show_and_list_refuse_an_entry_that_is_not_an_artefact() {
    let env = TestEnv::new("art_show_git");
    let lib = env.create_bare_repo("lib", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        lib.display()
    ));
    for command in ["show", "list"] {
        let out = env.run(&["artefact", command, "libs/lib"]);
        assert!(!out.success);
        assert!(
            out.stderr.contains("Not artefact entries: libs/lib"),
            "{}",
            out.stderr
        );
        let all = env.run(&["artefact", command]);
        assert!(all.success, "{}", all.stderr);
        assert!(
            all.stdout.contains("No artefact entries declared."),
            "{}",
            all.stdout
        );
    }
}

#[test]
fn list_shows_every_published_commit_with_its_refs() {
    let env = TestEnv::new("art_list");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let first = env.publish(&bare, "main", &[("app.bin", "v1")]);
    run_git_pub(&bare, &["tag", "v1", "main"]);
    let second = env.push_commit(&bare, "main", "README.md", "v2");
    env.publish(&bare, "main", &[("app.bin", "v2")]);
    // A third, on a branch that is gone by now: only the registry remembers it.
    run_git_pub(&bare, &["branch", "topic", "main"]);
    let third = env.push_commit(&bare, "topic", "README.md", "v3");
    env.publish(&bare, "topic", &[("app.bin", "v3")]);
    run_git_pub(&bare, &["branch", "-D", "topic"]);

    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);

    let out = env.run(&["artefact", "list"]);
    assert!(out.success, "{}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(lines[0].starts_with("meta/app  "), "{}", out.stdout);
    assert!(
        lines[0].ends_with("(3 images)"),
        "three tags, over two pages: {}",
        out.stdout
    );
    let row = |sha: &str| {
        lines
            .iter()
            .find(|l| l.contains(sha))
            .copied()
            .unwrap_or_default()
    };
    assert_eq!(
        row(&second).trim(),
        format!("{}  main  (installed)", second)
    );
    assert_eq!(row(&first).trim(), format!("{}  v1", first));
    assert_eq!(row(&third).trim(), third);
    // Commits a ref names come before the ones only the registry remembers.
    assert!(
        lines.iter().position(|l| l.contains(&third))
            > lines.iter().position(|l| l.contains(&first))
    );
}

#[test]
fn list_of_a_repository_nothing_published_for() {
    let env = TestEnv::new("art_list_empty");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    let out = env.run(&["artefact", "list"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        out.stdout.trim_end().ends_with("(0 images)"),
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Registry access
// ---------------------------------------------------------------------------

#[test]
fn a_refusal_says_how_to_get_access() {
    let env = TestEnv::new("art_forbidden");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.registry().refuse_with(403);
    env.write_config(&entry_config(&env, &bare, "main"));
    let out = env.run(&["clone"]);
    assert!(!out.success);
    assert!(out.stderr.contains("refused"), "{}", out.stderr);
    assert!(out.stderr.contains("docker login"), "{}", out.stderr);
}

fn gitlab_job(env: &TestEnv) -> Vec<(&'static str, String)> {
    let addr = env.registry().addr.clone();
    let port = addr.rsplit(':').next().unwrap().to_string();
    vec![
        ("CI_JOB_TOKEN", "job-token-value".to_string()),
        ("CI_SERVER_URL", format!("http://127.0.0.1:{}", port)),
        ("CI_REGISTRY", addr),
    ]
}

#[test]
fn a_ci_job_logs_in_with_its_job_token() {
    let env = TestEnv::new("art_job_token");
    let bare = env.artefact_repo("app", &[("app.bin", "from ci")]);
    env.registry().require_auth(helpers::registry::Auth {
        user: "gitlab-ci-token".into(),
        password: "job-token-value".into(),
        realm_host: "127.0.0.1".into(),
    });
    env.write_config(&entry_config(&env, &bare, "main"));

    let vars = gitlab_job(&env);
    let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let out = env.run_with_env(&vars, &["clone"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/app.bin"), "from ci");
    assert!(
        env.registry().count("GET", "/token [auth]") > 0,
        "{:?}",
        env.registry().log()
    );
}

#[test]
fn the_job_token_never_goes_to_a_token_service_elsewhere() {
    let env = TestEnv::new("art_foreign_realm");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    // The registry sends clients to a token service on another host name.
    env.registry().require_auth(helpers::registry::Auth {
        user: "gitlab-ci-token".into(),
        password: "job-token-value".into(),
        realm_host: "localhost".into(),
    });
    env.write_config(&entry_config(&env, &bare, "main"));

    let vars = gitlab_job(&env);
    let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let out = env.run_with_env(&vars, &["clone"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("Job token permissions"),
        "{}",
        out.stderr
    );
    assert!(env.registry().count("GET", "/token") > 0);
    assert_eq!(
        env.registry().count("GET", "/token [auth]"),
        0,
        "the token went to a host the CI server does not own: {:?}",
        env.registry().log()
    );
}

#[test]
fn a_docker_login_is_used_outside_ci() {
    let env = TestEnv::new("art_docker_login");
    let bare = env.artefact_repo("app", &[("app.bin", "logged in")]);
    env.registry().require_auth(helpers::registry::Auth {
        user: "dev".into(),
        password: "personal-token".into(),
        realm_host: "127.0.0.1".into(),
    });
    env.write_config(&entry_config(&env, &bare, "main"));
    let docker = env.repos_remote.join("docker-config");
    std::fs::create_dir_all(&docker).unwrap();
    let auth = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        "dev:personal-token",
    );
    std::fs::write(
        docker.join("config.json"),
        format!(
            r#"{{"auths": {{"{}": {{"auth": "{}"}}}}}}"#,
            env.registry().addr,
            auth
        ),
    )
    .unwrap();

    let without = env.run_with_env(&[], &["clone"]);
    assert!(!without.success, "no login, no access");
    let out = env.run_with_env(&[("DOCKER_CONFIG", docker.to_str().unwrap())], &["clone"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/app.bin"), "logged in");
}

// ---------------------------------------------------------------------------
// The cache commands
// ---------------------------------------------------------------------------

fn artefact_entry(env: &TestEnv) -> PathBuf {
    let entries = env.cache_entries("artefacts");
    assert_eq!(entries.len(), 1, "{:?}", entries);
    env.cache.join("artefacts").join(&entries[0])
}

#[test]
fn cache_status_has_an_artefacts_column() {
    let env = TestEnv::new("art_cache_status");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);

    let out = env.run_in(&env.playground, &["cache", "status"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("ARTEFACTS"), "{}", out.stdout);
    let row = out.stdout.lines().find(|l| l.contains("meta/app")).unwrap();
    let fields: Vec<&str> = row.split_whitespace().collect();
    // REPO, MIRROR, SNAPSHOTS, ARTEFACTS (two words), …
    assert_eq!((fields[1], fields[2]), ("-", "-"), "{}", row);
    assert_ne!(fields[3], "-", "{}", row);
    // The commit is listed under the row.
    assert!(
        redact_shas(&out.stdout).contains("      [sha]"),
        "{}",
        out.stdout
    );
}

#[test]
fn cache_update_warms_without_a_checkout() {
    let env = TestEnv::new("art_cache_update");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&entry_config(&env, &bare, "main"));

    let out = env.run_in(&env.playground, &["cache", "update"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!env.playground.join("meta/app").exists());
    artefact_entry(&env);

    env.registry().clear_log();
    assert!(env.run(&["clone"]).success);
    assert_eq!(blob_downloads(&env), 0);
}

#[test]
fn cache_repair_drops_a_damaged_blob_and_the_next_clone_heals_it() {
    let env = TestEnv::new("art_cache_repair");
    let bare = env.artefact_repo("app", &[("app.bin", "intact")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);

    let blobs = artefact_entry(&env).join("blobs/sha256");
    let biggest = std::fs::read_dir(&blobs)
        .unwrap()
        .flatten()
        .max_by_key(|e| e.metadata().unwrap().len())
        .unwrap()
        .path();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&biggest, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(&biggest, "rot").unwrap();

    let out = env.run_in(&env.playground, &["cache", "repair"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        out.stdout
            .contains("repair meta/app (1 damaged blob removed)"),
        "{}",
        out.stdout
    );

    std::fs::remove_dir_all(env.playground.join("meta")).unwrap();
    assert!(env.run(&["clone"]).success);
    assert_eq!(read(&env, "meta/app/app.bin"), "intact");
}

#[test]
fn cache_compact_drops_cold_commits_and_their_blobs() {
    let env = TestEnv::new("art_cache_compact");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let old = layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["clone"]).success);
    env.push_commit(&bare, "main", "README.md", "v2");
    let new = layered(&env, &bare, "app v2");
    assert!(env.run(&["pull"]).success);

    let entry = artefact_entry(&env);
    let blobs = || {
        std::fs::read_dir(entry.join("blobs/sha256"))
            .unwrap()
            .count()
    };
    let before = blobs();
    // The old commit has not been wanted for a long time.
    let marker = entry.join("gitscale-pins").join(&old);
    assert!(marker.is_file());
    let touched = std::process::Command::new("touch")
        .args(["-d", "2000-01-01", marker.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(touched.success());

    let out = env.run_in(
        &env.playground,
        &["cache", "compact", "--keep-recent", "30d"],
    );
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("1 pin dropped"), "{}", out.stdout);
    let index = std::fs::read_to_string(entry.join("index.json")).unwrap();
    assert!(!index.contains(&old) && index.contains(&new), "{}", index);
    // The old manifest and its app layer go; the shared vendor layer stays.
    assert_eq!(
        blobs(),
        before - 2,
        "{:?}",
        std::fs::read_dir(entry.join("blobs/sha256"))
            .unwrap()
            .count()
    );

    std::fs::remove_dir_all(env.playground.join("meta")).unwrap();
    env.registry().clear_log();
    assert!(env.run(&["clone"]).success);
    assert_eq!(blob_downloads(&env), 0);
}
