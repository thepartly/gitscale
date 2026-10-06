//! Artefacts end to end: `artefact publish` into a fake registry, and every
//! consumer command against it — what each one downloads, what it refuses,
//! and what it asks the registry for.

use crate::support;
use crate::support::artefacts::*;
use crate::support::resolution::{allow, repos, tag_commit, tagged};
use crate::support::worktrees::*;
use crate::support::{git_stdout, redact_shas, run_git_pub, TestEnv};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// The image is tagged with the source hash of what it was built from and
/// with the commit's version tags — never the commit — and records the
/// commit, its tree and the hash.
#[test]
fn normal_001_publish_tags_the_source_hash_and_annotates_the_image() {
    let env = TestEnv::new("art_publish_tags");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let commit = layered(&env, &bare, "v1.0.0", "app v1");

    let mut tags = env.registry().tags(&env.image(&bare));
    let hash = hash_tags(tags.clone()).pop().expect("a source hash tag");
    tags.sort();
    assert_eq!(tags, {
        let mut want = vec![hash.clone(), "v1.0.0".to_string()];
        want.sort();
        want
    });
    let manifest = env.registry().manifest(&env.image(&bare), &hash).unwrap();
    assert_eq!(
        env.registry()
            .manifest(&env.image(&bare), "v1.0.0")
            .unwrap(),
        manifest
    );
    assert_eq!(
        manifest["annotations"]["org.opencontainers.image.revision"],
        commit.as_str()
    );
    assert_eq!(manifest["annotations"]["dev.gitscale.hash"], hash.as_str());
    assert_eq!(
        manifest["annotations"]["dev.gitscale.tree"],
        git_stdout(&bare, &["rev-parse", &format!("{}^{{tree}}", commit)]).as_str()
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
    // The groups, in order, and nothing else: the repository's own
    // .gitscale.toml travels in the manifest, for consumers resolving its
    // dependencies.
    assert_eq!(titles, vec!["vendor", "app"]);
    // The config the producer published with, as it was: the registries the
    // publish job added, then its groups.
    let config = manifest["annotations"]["dev.gitscale.config"]
        .as_str()
        .unwrap();
    assert!(config.starts_with("[registries]\n"), "{}", config);
    assert!(config.ends_with(LAYERED), "{}", config);
}

/// One group is one layer: the config travels in the manifest, so the image
/// of a single group is a single layer — the form a deployer such as Argo CD
/// takes as an OCI source.
#[test]
fn normal_084_one_group_publishes_a_single_layer_image() {
    let env = TestEnv::new("art_single_layer");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    let producer = "[artefact]\ninclude = [\"dist/**\"]\n";
    let (out, _) = env.publish_with(
        &bare,
        "main",
        producer,
        &[("app.bin", "built")],
        &["v1.0.0"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let manifest = env
        .registry()
        .manifest(&env.image(&bare), "v1.0.0")
        .unwrap();
    assert_eq!(
        manifest["layers"].as_array().unwrap().len(),
        1,
        "{}",
        manifest
    );
    assert!(
        manifest["annotations"]["dev.gitscale.config"]
            .as_str()
            .unwrap()
            .ends_with(producer),
        "{}",
        manifest
    );
}

/// The repository's `.gitscale.toml` is never shipped in a layer, even by a
/// group that matches it, and `gitscale` is a group name like any other.
#[test]
fn edge_085_no_group_ships_the_config_and_any_may_be_named_gitscale() {
    let env = TestEnv::new("art_group_named_gitscale");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let (out, _) = env.publish_with(
        &bare,
        "main",
        "[[artefact.layer]]\nname = \"gitscale\"\ninclude = [\"**\"]\n",
        &[("app.bin", "built")],
        &["--dry-run"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("  layer gitscale: "), "{}", out.stdout);
    assert!(
        out.stdout.contains("  config: .gitscale.toml, "),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("    dist/app.bin\n"), "{}", out.stdout);
    assert!(
        !out.stdout.lines().any(|l| l.trim() == ".gitscale.toml"),
        "{}",
        out.stdout
    );
}

#[test]
fn normal_002_a_dry_run_lists_the_layers_and_sends_nothing() {
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
    assert!(text.contains("    dist/css/site.css"), "{}", text);
    assert!(
        env.registry().log().is_empty(),
        "{:?}",
        env.registry().log()
    );
}

#[test]
fn normal_003_an_annotated_tag_is_a_release() {
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
            "v2.0.0",
            "main",
        ],
    );
    env.publish(&bare, "v2.0.0", &[("app.bin", "tagged")]);
    env.write_config(&entry_config(&env, &bare, "v2.0.0"));

    let out = env.run(&["sync"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "tagged");
    assert_eq!(installed_tag(&env), "v2.0.0");
}

/// An artefact is only ever a release's: a commit, a branch, or no revision
/// at all fails the checkout, naming who asks for it.
#[test]
fn error_004_an_artefact_needs_a_release() {
    let env = TestEnv::new("art_not_a_release");
    let bare = env.artefact_repo("app", &[("app.bin", "pinned")]);
    let commit = git_stdout(&bare, &["rev-parse", "main"]);
    for (revision, said) in [
        (
            commit.as_str(),
            format!("root asks for {}, which is not a release", commit),
        ),
        (
            "main",
            "root asks for main, which is not a release".to_string(),
        ),
        (
            "",
            "nothing asks for a release of it: it follows its default branch".to_string(),
        ),
    ] {
        env.write_config(&entry_config(&env, &bare, revision));
        let out = env.run(&["sync"]);
        assert!(!out.success, "{}: {}", revision, out.stdout);
        assert!(
            out.stderr
                .contains(&format!("taken as an artefact, but {}", said)),
            "{}: {}",
            revision,
            out.stderr
        );
        assert!(!env.playground.join("meta/app").exists());
    }
}

#[test]
fn normal_006_sync_installs_the_image_of_a_release() {
    let env = TestEnv::new("pull_artefact_replace");
    let bare = env.artefact_repo("app", &[("app.bin", "binary-content")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("sync_artefact_replace_stdout", out.stdout);

    // Image paths are the repository's own.
    let file = env.playground.join("meta/app/dist/app.bin");
    assert_eq!(std::fs::read_to_string(file).unwrap(), "binary-content");
    assert!(!env.playground.join("meta/app/.git").exists());
}

#[test]
fn normal_007_sync_takes_the_image_of_a_newer_release() {
    let env = TestEnv::new("pull_artefact_local");
    let bare = env.artefact_repo("app", &[("app.bin", "v1-content")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out1 = env.run(&["sync"]);
    assert!(out1.success, "first pull stderr: {}", out1.stderr);

    // A new release, and its pipeline's artefact.
    env.push_commit(&bare, "main", "README.md", "v2");
    env.release(&bare, "v1.1.0", &[("app.bin", "v2-content")]);
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));

    let out2 = env.run(&["sync"]);
    assert!(out2.success, "pull stderr: {}", out2.stderr);
    insta::assert_snapshot!("sync_artefact_local_stdout", out2.stdout);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "v2-content");
}

#[test]
fn normal_008_fetch_records_what_the_registry_holds_and_installs_nothing() {
    let env = TestEnv::new("fetch_artefact_local");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    env.release(&bare, "v1.0.0", &[("app.bin", "v1-content")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));

    let out = env.run(&["fetch"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("fetch_artefact_local_stdout", out.stdout);

    // What the fetch saw is recorded outside the checkout; nothing is
    // downloaded, and no directory is made for it.
    assert!(!env.playground.join("meta/app").exists());
}

/// A release wanted that has no image yet is `missing` once a fetch has
/// looked; published, the checkout is only at the wrong release until a
/// sync.
#[test]
fn normal_009_ls_says_missing_after_a_fetch() {
    let env = TestEnv::new("art_status_behind");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));

    // A new release whose pipeline has not published yet.
    env.push_commit(&bare, "main", "README.md", "v2");
    run_git_pub(&bare, &["tag", "v1.1.0", "main"]);
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    let fetch = env.run(&["fetch"]);
    assert!(!fetch.success, "a missing artefact is an error");
    assert!(
        fetch.stderr.contains("no artefact for v1.1.0"),
        "{}",
        fetch.stderr
    );
    assert!(
        status_line(&env).ends_with("ref-mismatch, missing"),
        "{}",
        status_line(&env)
    );

    env.publish(&bare, "main", &[("app.bin", "v2")]);
    assert!(env.run(&["fetch"]).success);
    assert!(
        status_line(&env).ends_with("ref-mismatch"),
        "{}",
        status_line(&env)
    );
    assert!(env.run(&["sync"]).success);
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "v2");
}

#[test]
fn normal_010_a_republished_release_shows_as_changed_and_sync_takes_it() {
    let env = TestEnv::new("art_status_changed");
    let bare = env.artefact_repo("app", &[("app.bin", "first build")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);

    let artefact = "[artefact]\ninclude = [\"dist/**\"]\n";
    let (out, _) = env.publish_with(
        &bare,
        "main",
        artefact,
        &[("app.bin", "second build")],
        &["--force", "v1.0.0"],
    );
    assert!(out.success, "{}", out.stderr);
    // A sync alone asks the registry nothing: the release has not moved.
    env.registry().clear_log();
    assert!(env.run(&["sync"]).success);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "first build");
    assert!(
        env.registry().log().is_empty(),
        "a pull of an unmoved release asked the registry: {:?}",
        env.registry().log()
    );

    assert!(env.run(&["fetch"]).success);
    assert!(
        status_line(&env).ends_with("changed"),
        "{}",
        status_line(&env)
    );
    assert!(env.run(&["sync"]).success);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "second build");
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));
}

#[test]
fn normal_011_ls_json_carries_the_installed_release_and_digest() {
    let env = TestEnv::new("art_status_json");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);

    let out = env.run(&["ls", "--format", "json"]);
    let rows: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "meta/app")
        .unwrap();
    assert_eq!(row["artefact"]["installed"]["tag"], "v1.0.0");
    assert!(row["artefact"]["installed"]["digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
}

#[test]
fn normal_012_wanting_another_release_is_a_ref_mismatch() {
    let env = TestEnv::new("art_ref_mismatch");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.push_commit(&bare, "main", "README.md", "v2");
    env.release(&bare, "v1.1.0", &[("app.bin", "v2")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    assert!(
        status_line(&env).ends_with("ref-mismatch"),
        "{}",
        status_line(&env)
    );
    assert!(env.run(&["sync"]).success);
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));
}

/// What gitscale records about a checkout lives in the workspace's git
/// directory, never in the checkout, whose every file is the artefact's.
#[test]
fn normal_013_records_live_in_the_git_directory_not_the_checkout() {
    let env = TestEnv::new("art_records");
    env.init_playground_git();
    let bare = env.artefact_repo("app", &[("app.bin", "x"), (".env", "y")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);

    let listed = |dir: PathBuf| {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    // The producer's .gitscale.toml arrives from the manifest, read-only
    // like every file of the artefact.
    assert_eq!(
        listed(env.playground.join("meta/app")),
        vec![".gitscale.toml", "dist"]
    );
    assert!(
        std::fs::metadata(env.playground.join("meta/app/.gitscale.toml"))
            .unwrap()
            .permissions()
            .readonly()
    );
    assert_eq!(
        listed(env.playground.join("meta/app/dist")),
        vec![".env", "app.bin"]
    );
    let records = env.playground.join(".git/gitscale/artefacts");
    assert_eq!(
        std::fs::read_dir(&records).unwrap().count(),
        2,
        "installed and remote"
    );
    // The images themselves are the root's: shared by every worktree of it.
    assert!(env.playground.join(".git/gitscale/images").is_dir());
}

#[test]
fn normal_014_show_says_what_the_registry_and_the_checkout_hold() {
    let env = TestEnv::new("art_show");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let hash = hash_tags(env.registry().tags(&env.image(&bare)))
        .pop()
        .unwrap();

    let out = env.run(&["artefact", "show"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.starts_with("meta/app\n"), "{}", out.stdout);
    assert_eq!(shown(&out.stdout, "release"), "v1.0.0");
    assert!(
        shown(&out.stdout, "published").starts_with("sha256:"),
        "{}",
        out.stdout
    );
    assert_eq!(shown(&out.stdout, "sources"), hash);
    assert!(
        shown(&out.stdout, "config").starts_with(".gitscale.toml "),
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

    assert!(env.run(&["sync"]).success);
    let out = env.run(&["artefact", "show", "meta/app"]);
    assert!(
        shown(&out.stdout, "installed").starts_with("v1.0.0 (sha256:"),
        "{}",
        out.stdout
    );
    assert_eq!(shown(&out.stdout, "status"), "ok");

    // A new release nobody has published: show says so without a fetch,
    // and records nothing.
    env.push_commit(&bare, "main", "README.md", "v2");
    run_git_pub(&bare, &["tag", "v1.1.0", "main"]);
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    let out = env.run(&["artefact", "show"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(shown(&out.stdout, "release"), "v1.1.0");
    assert_eq!(shown(&out.stdout, "published"), "no");
    assert_eq!(shown(&out.stdout, "status"), "ref-mismatch, missing");
    assert!(
        status_line(&env).ends_with("ref-mismatch"),
        "show must not record anything: {}",
        status_line(&env)
    );
}

/// `list` shows each release with an image, newest first, with the source
/// hash it was built from — not the builds no release names.
#[test]
fn normal_015_list_shows_every_release_with_its_source_hash() {
    let env = TestEnv::new("art_list");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    env.release(&bare, "v1.0.0", &[("app.bin", "v1")]);
    env.push_commit(&bare, "main", "README.md", "v2");
    env.release(&bare, "v1.1.0", &[("app.bin", "v2")]);
    // A branch build: no release names it.
    run_git_pub(&bare, &["branch", "topic", "main"]);
    env.push_commit(&bare, "topic", "README.md", "v3");
    env.publish(&bare, "topic", &[("app.bin", "v3")]);

    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    assert!(env.run(&["sync"]).success);

    let hash = |tag: &str| {
        env.registry().manifest(&env.image(&bare), tag).unwrap()["annotations"]["dev.gitscale.hash"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let out = env.run(&["artefact", "list"]);
    assert!(out.success, "{}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(lines[0].starts_with("meta/app  "), "{}", out.stdout);
    assert!(lines[0].ends_with("(2 releases)"), "{}", out.stdout);
    assert_eq!(
        lines[1].trim(),
        format!("v1.1.0  {}  (installed)", hash("v1.1.0"))
    );
    assert_eq!(lines[2].trim(), format!("v1.0.0  {}", hash("v1.0.0")));
    assert_eq!(lines.len(), 3, "{}", out.stdout);
}

/// Placement prunes the image store by itself — at most once a day, so the cost
/// on every other placement is one stat.
#[test]
fn normal_016_placement_prunes_cold_images_once_a_day() {
    let env = TestEnv::new("art_daily_prune");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    let config = |release: &str| {
        format!(
            "[clean]\nkeep_recent = \"30d\"\n\n{}",
            entry_config(&env, &bare, release)
        )
    };
    env.write_config(&config("v1.0.0"));
    assert!(env.run(&["sync"]).success);
    env.push_commit(&bare, "main", "README.md", "v2");
    layered(&env, &bare, "v1.1.0", "app v2");
    env.write_config(&config("v1.1.0"));
    let entry = image_store(&env);
    let held = || std::fs::read_to_string(entry.join("index.json")).unwrap();

    // Pruned today already: nothing goes.
    age(&entry, "v1.0.0");
    assert!(env.run(&["sync"]).success);
    assert!(held().contains("\"v1.0.0\""));

    // A day later, it does.
    let marker = env.playground.join(".git/gitscale/images/gitscale-pruned");
    let touched = std::process::Command::new("touch")
        .args(["-d", "2000-01-01", marker.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(touched.success());
    assert!(env.run(&["sync"]).success);
    assert!(!held().contains("\"v1.0.0\""), "{}", held());
}

/// On the topic, a checkout taken as an artefact is its sources: the
/// topic's branch of it, followed. Off the topic, the image of its release
/// again.
#[test]
fn normal_017_an_artefact_on_a_topic_is_its_sources() {
    let env = TestEnv::new("wt_artefact_topic");
    let app = env.artefact_repo("app", &[("app.bin", "release build")]);
    let ws = artefact_workspace(&env, &app, gitscale::prefer::Form::Artefact);
    ok(&gs(&ws, &["sync"]));
    let dest = ws.join("meta/app");
    assert!(dest.join("dist/app.bin").is_file());

    run_git_pub(&app, &["branch", "feat/x", "main"]);
    env.push_commit(&app, "feat/x", "README.md", "feature");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["sync"]));
    assert!(dest.join(".git").exists(), "the sources");
    assert_eq!(branch(&dest).as_deref(), Some("feat/x"));
    let rows = support::status_clean::json_rows(&gs(&ws, &["ls", "--format", "json"]).stdout);
    let row = rows.iter().find(|r| r["directory"] == "meta/app").unwrap();
    assert_eq!(row["as_reason"], "topic");

    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["sync"]));
    assert!(!dest.join(".git").exists());
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "release build"
    );
}

/// Images other tools publish unpack too: a Docker-typed gzip layer and an
/// uncompressed OCI tar layer, in order, so a later layer's file replaces an
/// earlier one's. gitscale's own publisher writes neither, but an artefact
/// is an ordinary image, and the docs say any registry and tool works.
#[test]
fn normal_044_foreign_layer_media_types_unpack_in_order() {
    let env = TestEnv::new("artefact_foreign_media_types");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    push_image(
        &env,
        &bare,
        "v1.0.0",
        &[
            Layer {
                media_type: support::registry::DOCKER_LAYER_GZIP,
                bytes: Tar::new()
                    .file("dist/a.txt", "from docker")
                    .file("dist/shared.txt", "first")
                    .gzip(),
                title: "docker",
            },
            Layer {
                media_type: support::registry::LAYER_TAR,
                bytes: Tar::new()
                    .file("dist/b.txt", "from plain tar")
                    .file("dist/shared.txt", "second")
                    .bytes(),
                title: "plain",
            },
        ],
    );
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/a.txt"), "from docker");
    assert_eq!(read(&env, "meta/app/dist/b.txt"), "from plain tar");
    assert_eq!(read(&env, "meta/app/dist/shared.txt"), "second");
    assert_eq!(installed_tag(&env), "v1.0.0");
}

/// In a GitLab job, `publish` names the image after the job's project
/// (`CI_PROJECT_URL`), which is the URL consumers declare: under any other
/// name no consumer would find it.
#[test]
fn normal_045_publish_in_a_gitlab_job_names_the_image_after_its_project() {
    let env = TestEnv::new("artefact_publish_gitlab_job");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let commit = tip(&bare, "main");
    let producer = producer_with_build(&env, &bare);
    let project = format!("{}/project.git", env.repos_remote.display());

    let out = run_bin_in(
        &env,
        &producer,
        &[("GITLAB_CI", "true"), ("CI_PROJECT_URL", &project)],
        &["artefact", "publish"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let tags = env.registry().tags("project/gitscale");
    assert_eq!(hash_tags(tags.clone()), tags);
    assert!(env.registry().tags("app/gitscale").is_empty());
    let manifest = env
        .registry()
        .manifest("project/gitscale", &tags[0])
        .unwrap();
    assert_eq!(
        manifest["annotations"]["org.opencontainers.image.source"],
        project.as_str()
    );
    assert_eq!(
        manifest["annotations"]["org.opencontainers.image.revision"],
        commit.as_str()
    );
}

/// The same in a GitHub workflow: `GITHUB_SERVER_URL`/`GITHUB_REPOSITORY`
/// name the repository.
#[test]
fn normal_046_publish_in_a_github_workflow_names_the_image_after_its_repository() {
    let env = TestEnv::new("artefact_publish_github_workflow");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let producer = producer_with_build(&env, &bare);
    let server = env.repos_remote.display().to_string();

    let out = run_bin_in(
        &env,
        &producer,
        &[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SERVER_URL", &server),
            ("GITHUB_REPOSITORY", "org/proj"),
        ],
        &["artefact", "publish"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let tags = env.registry().tags("org/proj/gitscale");
    assert_eq!(hash_tags(tags.clone()).len(), 1, "{:?}", tags);
    let manifest = env
        .registry()
        .manifest("org/proj/gitscale", &tags[0])
        .unwrap();
    assert_eq!(
        manifest["annotations"]["org.opencontainers.image.source"],
        format!("{}/org/proj", server).as_str()
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// The way to see what would ship: a dry run lists the files even where the
/// image cannot be worked out yet — no registry mapping, no commit.
#[test]
fn edge_019_a_dry_run_lists_files_without_a_registry() {
    let env = TestEnv::new("art_dry_run_offline");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let producer = env.producer(&bare, "main");
    std::fs::create_dir_all(producer.join("dist/.well-known")).unwrap();
    std::fs::write(producer.join("dist/index.html"), "i").unwrap();
    std::fs::write(producer.join("dist/.well-known/security.txt"), "s").unwrap();
    std::fs::write(
        producer.join(".gitscale.toml"),
        "[artefact]\ninclude = [\"dist/**\"]\n",
    )
    .unwrap();
    std::fs::write(producer.join(".git/info/exclude"), "/dist/\n").unwrap();
    let out = env.run_in(&producer, &["artefact", "publish", "--dry-run"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        out.stdout.contains("image: unknown (no registry is known"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("    dist/index.html"), "{}", out.stdout);
    assert!(
        out.stdout.contains("    dist/.well-known/security.txt"),
        "{}",
        out.stdout
    );
    // A real publish still needs both.
    let out = env.run_in(&producer, &["artefact", "publish"]);
    assert!(!out.success);
}

/// Read-only applies to the whole archive, not just its top level, dot files
/// included — and an update still gets past the read-only files it replaces.
/// What gitscale records about the checkout is kept outside it.
#[test]
fn edge_022_an_installed_image_is_readonly_at_every_depth() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode();

    let env = TestEnv::new("artefact_readonly_depth");
    let files = |v: &'static str| {
        [
            ("app.bin", v),
            ("bin/tool", v),
            ("share/doc/readme", v),
            (".env", v),
            (".config/settings", v),
        ]
    };
    let bare = env.artefact_repo("app", &files("v1"));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let root = env.playground.join("meta/app");
    let dest = root.join("dist");

    let out = env.run(&["sync"]);
    assert!(out.success, "first pull stderr: {}", out.stderr);
    for (name, _) in files("") {
        assert_eq!(
            mode(&dest.join(name)) & 0o222,
            0,
            "{} should be readonly",
            name
        );
    }
    // Dot files are the artefact's like any other: nothing of gitscale's is
    // kept in the checkout.
    let listed = |dir: &std::path::Path| {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    // The producer's .gitscale.toml arrives from the manifest.
    assert_eq!(listed(&root), vec![".gitscale.toml", "dist"]);
    assert_eq!(
        listed(&dest),
        vec![".config", ".env", "app.bin", "bin", "share"]
    );

    // An update has to get past the read-only files it replaces.
    env.push_commit(&bare, "main", "README.md", "v2");
    env.release(&bare, "v1.1.0", &files("v2"));
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "pull stderr: {}", out.stderr);
    for (name, _) in files("") {
        let path = dest.join(name);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "v2");
        assert_eq!(
            mode(&path) & 0o222,
            0,
            "{} should be readonly after pull",
            name
        );
    }
}

/// A fetch only looks: an artefact it found in the registry is still to be
/// downloaded by the sync that follows.
#[test]
fn edge_023_sync_after_fetch_still_downloads_the_artefact() {
    let env = TestEnv::new("pull_after_fetch_artefact");
    let bare = env.artefact_repo("app", &[("app.bin", "v1-content")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));

    assert!(env.run(&["fetch"]).success);
    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join("meta/app/dist/app.bin").is_file(),
        "the artefact was never downloaded; pull said:\n{}",
        out.stdout
    );
}

/// A directory gitscale has no record of installing — one from an older
/// gitscale, or made by hand — is replaced by `sync`, dot files and all.
#[test]
fn edge_024_an_old_style_checkout_is_replaced_on_sync() {
    let env = TestEnv::new("art_legacy");
    let bare = env.artefact_repo("app", &[("app.bin", "new")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let dest = env.playground.join("meta/app");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join(".etag"), "\"abc\"").unwrap();
    std::fs::write(dest.join("stale.bin"), "old").unwrap();

    let out = env.run(&["sync"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "new");
    assert!(!dest.join("stale.bin").exists());
    assert!(!dest.join(".etag").exists());
}

/// Deleting a checkout by hand is noticed: it is not installed any more,
/// whatever was recorded.
#[test]
fn edge_025_a_deleted_checkout_is_cloned_again() {
    let env = TestEnv::new("art_deleted");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);
    std::fs::remove_dir_all(env.playground.join("meta/app")).unwrap();
    assert!(
        status_line(&env).ends_with("missed"),
        "{}",
        status_line(&env)
    );
    let out = env.run(&["sync"]);
    assert!(out.success, "{}", out.stderr);
    assert!(!out.stdout.contains("up to date"), "{}", out.stdout);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "x");
}

#[test]
fn edge_026_list_of_a_repository_nothing_published_for() {
    let env = TestEnv::new("art_list_empty");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["artefact", "list"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        out.stdout.trim_end().ends_with("(0 releases)"),
        "{}",
        out.stdout
    );
}

/// A blob that no longer matches its digest is never used: it is deleted and
/// downloaded again.
#[test]
fn edge_027_a_damaged_blob_is_downloaded_again() {
    let env = TestEnv::new("art_damaged_blob");
    let bare = env.artefact_repo("app", &[("app.bin", "intact")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);

    let blobs = image_store(&env).join("blobs/sha256");
    let biggest = std::fs::read_dir(&blobs)
        .unwrap()
        .flatten()
        .max_by_key(|e| e.metadata().unwrap().len())
        .unwrap()
        .path();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&biggest, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(&biggest, "rot").unwrap();

    std::fs::remove_dir_all(env.playground.join("meta")).unwrap();
    env.registry().clear_log();
    assert!(env.run(&["sync"]).success);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "intact");
    // The damaged one, and only that — manifest or layer, whichever it was.
    let gets = env.registry().count("GET", "");
    assert_eq!(gets, 1, "{:?}", env.registry().log());
}

/// An image whose directories are read-only — as another tool may write
/// them — still installs, and the next version still replaces it. gitscale
/// makes the files read-only itself; a directory it cannot write into would
/// leave the checkout impossible to install or update.
#[test]
#[ignore = "bug: a layer with a 0555 directory cannot be unpacked, nor the checkout replaced"]
fn edge_051_an_image_with_a_readonly_directory_installs_and_updates() {
    let env = TestEnv::new("artefact_readonly_directory");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    push_image(
        &env,
        &bare,
        "v1.0.0",
        &[gzip_layer(
            "app",
            Tar::new().dir("ro", 0o555).file("ro/f.txt", "v1"),
        )],
    );
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/ro/f.txt"), "v1");

    env.push_commit(&bare, "main", "README.md", "v2");
    run_git_pub(&bare, &["tag", "v1.1.0", "main"]);
    push_image(
        &env,
        &bare,
        "v1.1.0",
        &[gzip_layer(
            "app",
            Tar::new().dir("ro", 0o555).file("ro/f.txt", "v2"),
        )],
    );
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    let out = env.run(&["sync"]);
    make_tree_writable(&env.playground.join("meta"));
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/ro/f.txt"), "v2");
}

/// OCI whiteout files (`.wh.<name>`) are unpacked as ordinary files, and
/// the file they delete stays. This pins current behaviour: gitscale's own
/// images never hold whiteouts, and whether foreign multi-layer images
/// should get OCI deletion semantics is the owner's call.
#[test]
fn edge_052_whiteout_files_are_unpacked_as_ordinary_files() {
    let env = TestEnv::new("artefact_whiteouts");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    push_image(
        &env,
        &bare,
        "v1.0.0",
        &[
            gzip_layer(
                "base",
                Tar::new()
                    .file("dist/old.txt", "o")
                    .file("dist/keep.txt", "k"),
            ),
            gzip_layer("delete", Tar::new().file("dist/.wh.old.txt", "")),
        ],
    );
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let dist = env.playground.join("meta/app/dist");
    assert!(dist.join("keep.txt").is_file());
    assert!(dist.join("old.txt").is_file());
    assert!(dist.join(".wh.old.txt").is_file());
}

/// A repository holds symlinks out of itself that no artefact ships — the
/// links gitscale plants for its own dependencies. `publish` ignores them,
/// unless a pattern selects one: then it refuses, since such a link would
/// point at nothing on the consumer's machine, or at something else.
#[test]
fn edge_054_publish_ships_no_link_out_of_the_repository_unless_selected() {
    let env = TestEnv::new("artefact_publish_planted_link");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let producer = producer_with_build(&env, &bare);
    std::fs::create_dir_all(producer.join("libs")).unwrap();
    std::os::unix::fs::symlink("../../elsewhere", producer.join("libs/dep")).unwrap();
    std::fs::write(producer.join(".git/info/exclude"), "/dist/\n/libs/\n").unwrap();

    let out = env.run_in(&producer, &["artefact", "publish", "--dry-run"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("libs/dep"), "{}", out.stdout);

    std::fs::write(
        producer.join(".gitscale.toml"),
        format!(
            "{}[artefact]\ninclude = [\"dist/**\", \"libs/**\"]\n",
            env.registries()
        ),
    )
    .unwrap();
    let out = env.run_in(&producer, &["artefact", "publish"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("libs/dep is a symlink to ../../elsewhere, outside the repository"),
        "{}",
        out.stderr
    );
    assert!(env.registry().tags(&env.image(&bare)).is_empty());
}

/// A dry run where there is no commit and no remote still lists every file,
/// and says what it could not work out — the commit, the image, and so the
/// policy — rather than failing or pretending it checked.
#[test]
fn edge_055_a_dry_run_outside_git_says_what_it_could_not_check() {
    let env = TestEnv::new("artefact_dry_run_no_git");
    let loose = env.repos_remote.join("loose");
    std::fs::create_dir_all(loose.join("dist")).unwrap();
    std::fs::write(loose.join("dist/a.txt"), "a").unwrap();
    std::fs::write(
        loose.join(".gitscale.toml"),
        format!("{}[artefact]\ninclude = [\"dist/**\"]\n", env.registries()),
    )
    .unwrap();
    // The test tree itself sits inside a repository: keep git from finding it.
    let ceiling = env.repos_remote.display().to_string();
    let out = run_bin_in(
        &env,
        &loose,
        &[("GIT_CEILING_DIRECTORIES", &ceiling)],
        &["artefact", "publish", "--dry-run"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("Would publish:"), "{}", out.stdout);
    assert!(out.stdout.contains("  tags: unknown ("), "{}", out.stdout);
    assert!(out.stdout.contains("  image: unknown ("), "{}", out.stdout);
    assert!(
        out.stdout.contains("  policy: not checked ("),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("    dist/a.txt"), "{}", out.stdout);
    assert!(env.registry().log().is_empty());
}

/// A forced re-publish that changes the image's `.gitscale.toml` changes
/// what resolution reads: after a fetch says the image changed, the sync
/// that installs the new files also brings the dependencies the new config
/// declares. Files from one build and dependencies from another would be a
/// checkout nobody published.
#[test]
#[ignore = "bug: resolution reads the config in the manifest held for the release, not the republished one"]
fn edge_057_a_forced_republish_brings_the_dependencies_it_declares() {
    let env = TestEnv::new("artefact_republish_config");
    let dep = tagged(&env, "dep", &[("v1.0.0", "")]);
    let art = env.create_bare_repo("art", "main", &[("README.md", "art")]);
    run_git_pub(&art, &["tag", "v1.0.0", "main"]);
    let plain = "[artefact]\ninclude = [\"dist/**\"]\n";
    let (out, _) = env.publish_with(&art, "main", plain, &[("app.bin", "first")], &[]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    env.prefer(&art, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[("meta/app", &art, ", revision = \"v1.0.0\"")])
    ));
    assert!(env.run(&["sync"]).success);

    let with_dep = format!(
        "{}\n{}",
        plain,
        repos(&[("vendor/dep", &dep, ", revision = \"v1.0.0\"")])
    );
    let (out, _) = env.publish_with(
        &art,
        "main",
        &with_dep,
        &[("app.bin", "second")],
        &["--force"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.run(&["fetch"]).success);
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "second");
    let link = env.playground.join("meta/app/vendor/dep");
    assert!(
        link.is_symlink(),
        "the republished config's dependency is missing: {}",
        out.stdout
    );
    assert_eq!(
        git_stdout(&env.playground.join("imports/dep"), &["rev-parse", "HEAD"]),
        tag_commit(&dep, "v1.0.0")
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_028_a_different_build_of_a_published_commit_needs_force() {
    let env = TestEnv::new("art_republish_force");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");

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
fn error_029_a_group_that_matches_nothing_fails_the_publish() {
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
fn error_030_publishing_needs_an_artefact_table() {
    let env = TestEnv::new("art_no_table");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let producer = env.producer(&bare, "main");
    std::fs::write(producer.join(".gitscale.toml"), env.registries()).unwrap();
    let out = env.run_in(&producer, &["artefact", "publish"]);
    assert!(!out.success);
    assert!(out.stderr.contains("no [artefact] table"), "{}", out.stderr);
}

/// Every file an image ships is either the commit's own, unmodified, or
/// ignored build output: anything else could not be laid over a checkout of
/// that commit, and `publish` refuses it — a dry run too.
#[test]
fn error_031_publish_refuses_files_that_break_the_artefact_policy() {
    let env = TestEnv::new("art_policy");
    let bare = env.create_bare_repo(
        "app",
        "main",
        &[("README.md", "app"), ("src/lib.rs", "lib")],
    );
    let producer = env.producer(&bare, "main");
    std::fs::write(producer.join("src/lib.rs"), "changed").unwrap();
    std::fs::create_dir_all(producer.join("out")).unwrap();
    std::fs::write(producer.join("out/app.bin"), "built").unwrap();
    std::fs::write(
        producer.join(".gitscale.toml"),
        format!(
            "{}[artefact]\ninclude = [\"src/**\", \"out/**\", \"README.md\"]\n",
            env.registries()
        ),
    )
    .unwrap();
    for args in [
        &["artefact", "publish"][..],
        &["artefact", "publish", "--dry-run"],
    ] {
        let out = env.run_in(&producer, args);
        assert!(!out.success, "{:?}: {}", args, out.stdout);
        assert!(
            out.stderr.contains("2 files break the artefact policy"),
            "{}",
            out.stderr
        );
        assert!(
            out.stderr.contains("src/lib.rs") && out.stderr.contains("tracked, modified"),
            "{}",
            out.stderr
        );
        assert!(
            out.stderr.contains("out/app.bin")
                && out
                    .stderr
                    .contains("untracked, not ignored — add out/ to .gitignore"),
            "{}",
            out.stderr
        );
    }
    assert!(env.registry().tags(&env.image(&bare)).is_empty());
    // A tracked file shipped as it is, and ignored build output, pass.
    support::run_git_pub(&producer, &["checkout", "--", "src/lib.rs"]);
    std::fs::write(producer.join(".git/info/exclude"), "/out/\n").unwrap();
    let out = env.run_in(&producer, &["artefact", "publish"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
}

/// An abbreviated commit is taken for a name the remote does not have, and
/// the failure says to give the full SHA.
#[test]
fn error_032_an_abbreviated_sha_is_refused_with_directions() {
    let env = TestEnv::new("art_short_sha");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    let short = &git_stdout(&bare, &["rev-parse", "main"])[..9];
    env.write_config(&entry_config(&env, &bare, short));

    let out = env.run(&["sync"]);
    assert!(!out.success);
    assert!(out.stderr.contains("full SHA"), "{}", out.stderr);
    assert!(
        out.stderr.contains(&format!("git rev-parse {}", short)),
        "{}",
        out.stderr
    );
}

#[test]
fn error_033_sync_without_a_registry_says_how_to_add_one() {
    let env = TestEnv::new("pull_no_registry");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    env.prefer(&bare, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        "[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        bare.display()
    ));

    let out = env.run(&["sync"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("no registry is known") && out.stderr.contains("[registries]"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn error_034_sync_fails_when_nothing_is_published() {
    let env = TestEnv::new("pull_artefact_no_data");
    // A release whose pipeline has published nothing.
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));

    let out = env.run(&["sync"]);
    assert!(!out.success, "a missing artefact is an error, not a skip");
    assert!(
        out.stderr.contains("no artefact for v1.0.0 ("),
        "stderr: {}",
        out.stderr
    );
    assert!(!env.playground.join("meta/app").exists());
}

/// An archive that cannot be unpacked leaves nothing that later passes for
/// the artefact: running sync again tries again, rather than taking the
/// half-made directory as done.
#[test]
fn error_035_a_corrupt_artefact_is_not_taken_as_current() {
    let env = TestEnv::new("corrupt_artefact");
    let bare = env.artefact_repo("app", &[("app.bin", "v1-content")]);
    env.registry().corrupt_blobs();
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));

    let first = env.run(&["sync"]);
    assert!(!first.success, "the blob does not match its digest");
    assert!(
        first.stderr.contains("does not match its digest"),
        "{}",
        first.stderr
    );
    let again = env.run(&["sync"]);
    assert!(
        !again.success,
        "pull took the failed one as up to date:\n{}",
        again.stdout
    );
    assert!(!env.playground.join("meta/app/dist/app.bin").exists());
    // Nothing is recorded as installed.
    assert_eq!(installed_tag(&env), "");
}

#[test]
fn error_036_show_reports_an_entry_it_cannot_look_up_and_fails() {
    let env = TestEnv::new("art_show_error");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    // No [registries]: nothing maps this repository to a registry.
    env.prefer(&bare, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        "[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
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
        out.stderr.contains("1 image could not be looked up"),
        "{}",
        out.stderr
    );
}

#[test]
fn normal_037_show_and_list_name_nothing_while_nothing_is_taken_as_an_artefact() {
    let env = TestEnv::new("art_show_git");
    let lib = env.create_bare_repo("lib", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        lib.display()
    ));
    for command in ["show", "list"] {
        let all = env.run(&["artefact", command]);
        assert!(all.success, "{}", all.stderr);
        assert!(
            all.stdout.contains("No checkout is taken as an artefact."),
            "{}",
            all.stdout
        );
    }
}

/// A registry that serves a damaged layer during an update leaves the
/// installed version alone: everything is downloaded and checked before the
/// old files go, as the docs promise. The installed files, the record of
/// them and `ls` all still say the old release.
#[test]
fn error_058_a_registry_failing_mid_update_keeps_the_installed_version() {
    let env = TestEnv::new("artefact_mid_update_failure");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);

    env.push_commit(&bare, "main", "README.md", "v2");
    layered(&env, &bare, "v1.1.0", "app v2");
    // The vendor layer is the same as before; the app layer is new, and the
    // one the registry damages.
    let app_layer = layer_digests(&env, &bare, "v1.1.0")[1].clone();
    env.registry().corrupt_blob(&app_layer);
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("does not match its digest"),
        "{}",
        out.stderr
    );
    assert_eq!(read(&env, "meta/app/dist/app.js"), "app v1");
    assert_eq!(read(&env, "meta/app/dist/vendor/lib.js"), "vendor v1");
    assert_eq!(installed_tag(&env), "v1.0.0");
}

/// A tag that names an image index — what `docker buildx` pushes with
/// provenance, or a multi-platform image — is not an image with no layers.
/// Taking it for one would wipe the installed files and record an empty
/// checkout as the artefact, and every later sync would succeed doing it
/// again.
#[test]
#[ignore = "bug: a manifest without layers (an OCI index) is installed as an empty artefact"]
fn error_059_an_image_index_is_refused_rather_than_installed_empty() {
    let env = TestEnv::new("artefact_index_refused");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);

    env.push_commit(&bare, "main", "README.md", "v2");
    run_git_pub(&bare, &["tag", "v1.1.0", "main"]);
    let index = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": support::registry::INDEX_TYPE,
        "manifests": [{
            "mediaType": support::registry::MANIFEST_TYPE,
            "digest": format!("sha256:{}", "a".repeat(64)),
            "size": 100,
            "platform": { "architecture": "amd64", "os": "linux" },
        }],
    }))
    .unwrap();
    env.registry().put_manifest(
        &env.image(&bare),
        "v1.1.0",
        &index,
        support::registry::INDEX_TYPE,
    );
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success, "an index was installed: {}", out.stdout);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "v1");
    assert_eq!(installed_tag(&env), "v1.0.0");
}

/// Nothing in an archive may land outside the checkout or be anything but a
/// file, a directory or a symlink inside it: a `..` path, an absolute path,
/// a hard link, a FIFO, and symlinks out — relative or absolute — each fail
/// the whole install, and leave no half-unpacked checkout behind.
#[test]
fn error_060_archive_entries_that_leave_the_checkout_are_refused() {
    let env = TestEnv::new("artefact_hostile_entries");
    let ok_file = || Tar::new().file("dist/ok.txt", "ok");
    let cases: Vec<(&str, Tar, &str)> = vec![
        (
            "dotdot",
            ok_file().file("../escape.txt", "x"),
            "unsafe path: ../escape.txt",
        ),
        (
            "absolute",
            ok_file().file("/abs-escape.txt", "x"),
            "unsafe path: /abs-escape.txt",
        ),
        (
            "hardlink",
            ok_file().hardlink("hard", "/etc/passwd"),
            "unsupported entry hard",
        ),
        ("fifo", ok_file().fifo("pipe"), "unsupported entry pipe"),
        (
            "linkout",
            ok_file().symlink("escape", "../../outside"),
            "symlink leaving the checkout: escape -> ../../outside",
        ),
        (
            "linkabsolute",
            ok_file().symlink("abs", "/etc"),
            "symlink leaving the checkout: abs -> /etc",
        ),
    ];
    for (name, tar, why) in cases {
        let bare = env.create_bare_repo(name, "main", &[("README.md", name)]);
        run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
        push_image(&env, &bare, "v1.0.0", &[gzip_layer("app", tar)]);
        env.write_config(&entry_config(&env, &bare, "v1.0.0"));
        let out = env.run(&["sync"]);
        assert!(!out.success, "{}: {}", name, out.stdout);
        assert!(
            out.stderr.contains(why) || out.stdout.contains(why),
            "{}: {}{}",
            name,
            out.stdout,
            out.stderr
        );
        assert!(
            !env.playground.join("meta/app/dist/ok.txt").exists(),
            "{}: a half-unpacked checkout was left",
            name
        );
        assert_eq!(installed_tag(&env), "", "{}", name);
        assert!(!env.playground.join("meta/escape.txt").exists(), "{}", name);
        assert!(
            !std::path::Path::new("/abs-escape.txt").exists(),
            "{}",
            name
        );
    }
}

/// A symlink is judged by where it really resolves, not by its path in the
/// archive: with `a/b -> ..`, an entry `a/b/l -> ../x` passes a check on its
/// path (two levels down, one up) but lands at the top of the checkout,
/// pointing out of it. The docs promise no symlink pointing out of the
/// directory.
#[test]
#[ignore = "bug: symlink targets are checked against the entry's path, not where it resolves"]
fn error_061_a_symlink_through_a_symlink_cannot_point_out_of_the_checkout() {
    let env = TestEnv::new("artefact_symlink_chain");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    push_image(
        &env,
        &bare,
        "v1.0.0",
        &[gzip_layer(
            "app",
            Tar::new()
                .file("dist/ok.txt", "ok")
                .dir("a", 0o755)
                .symlink("a/b", "..")
                .symlink("a/b/l", "../escaped"),
        )],
    );
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    let escaped = env.playground.join("meta/app/l");
    assert!(
        std::fs::symlink_metadata(&escaped).is_err(),
        "{} -> {:?} was installed",
        escaped.display(),
        std::fs::read_link(&escaped)
    );
    assert!(!out.success, "{}", out.stdout);
}

/// A layer of a kind gitscale cannot unpack — a Helm chart, say — fails the
/// install with the media type named, rather than being unpacked as a tar
/// or skipped.
#[test]
fn error_063_an_unknown_layer_media_type_is_refused() {
    let env = TestEnv::new("artefact_unknown_media_type");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    push_image(
        &env,
        &bare,
        "v1.0.0",
        &[Layer {
            media_type: "application/vnd.cncf.helm.chart.content.v1.tar+gzip",
            bytes: Tar::new().file("dist/chart.yaml", "x").gzip(),
            title: "chart",
        }],
    );
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains(
            "has a media type gitscale cannot unpack: application/vnd.cncf.helm.chart.content.v1.tar+gzip"
        ),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("meta/app/dist/chart.yaml").exists());
}

/// A manifest whose layer digest is not a well-formed sha256 digest is
/// refused before the digest is ever used as a file name in the image
/// store: `sha256:../../escape` would otherwise name a path outside it.
#[test]
fn error_064_a_manifest_naming_an_invalid_digest_is_refused() {
    let env = TestEnv::new("artefact_invalid_layer_digest");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": support::registry::MANIFEST_TYPE,
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": format!("sha256:{}", "b".repeat(64)),
            "size": 2,
        },
        "layers": [{
            "mediaType": support::registry::LAYER_GZIP,
            "digest": "sha256:../../escape",
            "size": 3,
        }],
    }))
    .unwrap();
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    env.registry().put_manifest(
        &env.image(&bare),
        "v1.0.0",
        &manifest,
        support::registry::MANIFEST_TYPE,
    );
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("the image manifest names an invalid layer digest: sha256:../../escape"),
        "{}",
        out.stderr
    );
    assert_eq!(env.registry().count("GET", "/blobs/"), 0);
}

/// `ls --fetch` that cannot reach the registry still prints the table,
/// says the entry's row is what the last successful fetch saw, and does not
/// pass that off as fresh.
#[test]
fn error_066_ls_fetch_without_the_registry_shows_the_last_fetched_state() {
    let env = TestEnv::new("artefact_status_fetch_refused");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);
    env.registry().refuse_with(500);

    let out = env.run(&["ls", "--fetch"]);
    assert!(
        out.stderr.contains("fetch meta/app:")
            && out.stderr.contains("(showing the last fetched state)"),
        "{}",
        out.stderr
    );
    assert!(
        support::strip_ansi(&out.stdout)
            .lines()
            .any(
                |l| l.split_whitespace().nth(1) == Some("meta/app") && l.trim_end().ends_with("ok")
            ),
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

#[test]
fn perf_038_an_identical_republish_uploads_nothing() {
    let env = TestEnv::new("art_republish_same");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
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
fn perf_039_a_sync_downloads_only_the_layer_that_changed() {
    let env = TestEnv::new("art_layer_pull");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);
    // Vendor and app: resolution reads the config from the manifest.
    assert_eq!(blob_downloads(&env), 2);

    env.push_commit(&bare, "main", "README.md", "v2");
    layered(&env, &bare, "v1.1.0", "app v2");
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    env.registry().clear_log();
    let out = env.run(&["sync"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.js"), "app v2");
    assert_eq!(read(&env, "meta/app/dist/vendor/lib.js"), "vendor v1");
    assert_eq!(blob_downloads(&env), 1, "{:?}", env.registry().log());
}

#[test]
fn perf_040_an_up_to_date_sync_asks_the_registry_nothing() {
    let env = TestEnv::new("pull_artefact_up_to_date");
    let bare = env.artefact_repo("app", &[("app.bin", "content")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);
    env.registry().clear_log();

    // Sync again: the release is installed, so the registry is not asked
    // anything.
    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("sync_artefact_up_to_date_stdout", out.stdout);
    assert!(
        env.registry().log().is_empty(),
        "{:?}",
        env.registry().log()
    );
}

/// The root's image store is shared by its worktrees: a second one of the
/// root installs from it, downloading nothing.
#[test]
fn perf_041_a_second_root_worktree_downloads_nothing() {
    let env = TestEnv::new("art_second_worktree");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    env.init_playground_git();
    run_git_pub(&env.playground, &["add", ".gitscale.toml"]);
    run_git_pub(&env.playground, &["commit", "-q", "-m", "config"]);
    assert!(env.run(&["sync"]).success);

    let other = env.repos_remote.join("second-worktree");
    run_git_pub(
        &env.playground,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            other.to_str().unwrap(),
        ],
    );
    env.registry().clear_log();
    let out = env.run_in(&other, &["sync"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(
        std::fs::read_to_string(other.join("meta/app/dist/app.js")).unwrap(),
        "app v1"
    );
    assert_eq!(blob_downloads(&env), 0, "{:?}", env.registry().log());
    assert_eq!(env.registry().count("GET", "/manifests/"), 0);
}

/// A CI job without the cache keeps nothing: every layer is downloaded again,
/// two blobs a sync.
#[test]
fn perf_042_in_ci_without_the_cache_every_layer_is_downloaded() {
    let env = TestEnv::new("art_ci_no_cache");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    for _ in 0..2 {
        let _ = std::fs::remove_dir_all(env.playground.join("meta"));
        let out = env.run_with_env(&[("CI", "true")], &["sync", "--no-cache"]);
        assert!(out.success, "{}{}", out.stdout, out.stderr);
    }
    // Each time, the two layers the install unpacks.
    assert_eq!(blob_downloads(&env), 4);
    assert!(env.cache_entries("images").is_empty());
}

/// N cold CI jobs on one runner, sharing its cache, download each blob once.
#[test]
fn perf_043_parallel_cold_ci_jobs_download_each_blob_once() {
    let env = TestEnv::new("art_parallel");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    let workspaces: Vec<PathBuf> = (0..4)
        .map(|i| env.repos_remote.join(format!("ws{}", i)))
        .collect();
    for ws in &workspaces {
        std::fs::create_dir_all(ws).unwrap();
        run_git_pub(ws, &["init", "-q"]);
        std::fs::write(
            ws.join(".gitscale.toml"),
            entry_config(&env, &bare, "v1.0.0"),
        )
        .unwrap();
        crate::support::prefer(ws, &bare, gitscale::prefer::Form::Artefact);
    }
    std::thread::scope(|scope| {
        for ws in &workspaces {
            let cache = env.cache.clone();
            scope.spawn(move || {
                let out = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"))
                    .args(["sync", "-C", ws.to_str().unwrap()])
                    .env("CI", "true")
                    .env("GITSCALE_CACHE_DIR", &cache)
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
            });
        }
    });
    for ws in &workspaces {
        assert_eq!(
            std::fs::read_to_string(ws.join("meta/app/dist/app.js")).unwrap(),
            "app v1"
        );
    }
    assert_eq!(blob_downloads(&env), 2, "{:?}", env.registry().log());
}

/// A release named for a commit already published goes on its image,
/// uploading nothing.
#[test]
fn normal_067_publish_releases_an_image_already_there() {
    let env = TestEnv::new("art_version_tags");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let (out, _) = env.publish_with(
        &bare,
        "main",
        LAYERED,
        &[("vendor/lib.js", "vendor v1"), ("app.js", "app v1")],
        &[],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(hash_tags(env.registry().tags(&env.image(&bare))).len(), 1);
    assert_eq!(
        env.registry().tags(&env.image(&bare)).len(),
        1,
        "no release yet"
    );

    env.registry().clear_log();
    layered(&env, &bare, "v1-2026.10.05-1", "app v1");
    let hash = hash_tags(env.registry().tags(&env.image(&bare)))
        .pop()
        .unwrap();
    assert_eq!(
        env.registry()
            .manifest(&env.image(&bare), "v1-2026.10.05-1"),
        env.registry().manifest(&env.image(&bare), &hash)
    );
    assert_eq!(env.registry().count("POST", "/blobs/uploads/"), 0);
}

/// A squash merge that changes no file gives `main` a new commit with the
/// branch build's sources: `publish --reuse` releases that build's image, as
/// it is, packing and uploading nothing.
#[test]
fn normal_068_reuse_releases_the_image_of_the_same_sources() {
    let env = TestEnv::new("art_reuse");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["branch", "feat/x", "main"]);
    let branch = env.push_commit(&bare, "feat/x", "lib.txt", "the change");
    let built = [("vendor/lib.js", "vendor"), ("app.js", "built")];
    let (out, _) = env.publish_with(&bare, "feat/x", LAYERED, &built, &[]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let image = env.image(&bare);
    let hash = hash_tags(env.registry().tags(&image)).pop().unwrap();

    let tree = git_stdout(&bare, &["rev-parse", &format!("{}^{{tree}}", branch)]);
    let squashed = git_stdout(&bare, &["commit-tree", &tree, "-p", "main", "-m", "squash"]);
    run_git_pub(&bare, &["update-ref", "refs/heads/main", &squashed]);
    env.registry().clear_log();

    let (out, _) = env.publish_with(&bare, "main", LAYERED, &[], &["--reuse", "v1-2026.10.05-1"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let released = env.registry().manifest(&image, "v1-2026.10.05-1").unwrap();
    assert_eq!(released, env.registry().manifest(&image, &hash).unwrap());
    assert_eq!(
        released["annotations"]["org.opencontainers.image.revision"],
        branch.as_str(),
        "the build it is"
    );
    assert_eq!(env.registry().count("POST", "/blobs/uploads/"), 0);
}

/// With no image of these sources there is nothing to reuse: `--reuse`
/// fails, a dry run as well, so a release pipeline stops before it tags.
#[test]
fn error_069_reuse_fails_without_an_image_of_these_sources() {
    let env = TestEnv::new("art_reuse_none");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    env.push_commit(&bare, "main", "lib.txt", "changed sources");
    for args in [
        &["--reuse", "--dry-run", "v1.1.0"][..],
        &["--reuse", "v1.1.0"][..],
    ] {
        let (out, _) = env.publish_with(&bare, "main", LAYERED, &[], args);
        assert!(!out.success, "{:?}: {}", args, out.stdout);
        assert!(
            out.stderr.contains("no image of these sources"),
            "{:?}: {}",
            args,
            out.stderr
        );
    }
    assert_eq!(hash_tags(env.registry().tags(&env.image(&bare))).len(), 1);
}

/// A repository published as `app`, at a release, whose sources then become
/// unreadable — moved where its URL no longer reaches. Its image declares a
/// dependency in its config. Returns the URL the root asks for.
fn unreadable_release(env: &TestEnv) -> PathBuf {
    let dep = tagged(env, "dep", &[("v1.0.0", "")]);
    let app = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let producer = format!(
        "[artefact]\ninclude = [\"dist/**\"]\n\n{}",
        repos(&[("vendor/dep", &dep, ", revision = \"v1.0.0\"")])
    );
    let (out, _) = env.publish_with(
        &app,
        "main",
        &producer,
        &[("app.bin", "built")],
        &["v1-2026.10.05-1"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    std::fs::rename(&app, env.repos_remote.join("moved.git")).unwrap();
    app
}

/// A repository whose sources cannot be read is its artefact, with nothing to
/// configure: its release found in its registry, its dependencies in its
/// image's manifest, and its source hash from the tree its image records.
#[test]
fn normal_070_without_access_to_its_sources_a_checkout_is_its_artefact() {
    let env = TestEnv::new("art_no_access");
    let app = unreadable_release(&env);
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[("meta/app", &app, ", revision = \"v1-2026.10.05-1\"")])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let dest = env.playground.join("meta/app");
    assert!(dest.join("dist/app.bin").is_file());
    assert!(!dest.join(".git").exists());
    assert!(
        dest.join("vendor/dep").is_symlink(),
        "its dependency, linked"
    );
    assert!(env.playground.join("imports/dep/VERSION").is_file());

    let row = crate::support::status_clean::json_row(&env, "meta/app");
    assert_eq!(row["as"], "artefact");
    assert_eq!(row["as_reason"], "no-access");
    let out = env.run(&["hash", "meta/app"]);
    assert!(out.success, "{}", out.stderr);

    // On a topic it stays at its release: no branch of it can be seen.
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        crate::support::status_clean::json_row(&env, "meta/app")["topic"],
        serde_json::Value::Null
    );
}

/// Without its sources, a branch cannot be resolved: only released versions.
#[test]
fn error_071_without_access_a_branch_revision_needs_the_sources() {
    let env = TestEnv::new("art_no_access_branch");
    let app = unreadable_release(&env);
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[("meta/app", &app, ", revision = \"main\"")])
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("main needs access to its sources"),
        "{}",
        out.stderr
    );
}

/// A remote that cannot be reached is not one that refused: its registry is
/// not asked instead, and the fetch error is what is said.
#[test]
fn edge_072_an_unreachable_remote_is_not_taken_for_one_without_access() {
    let env = TestEnv::new("art_unreachable");
    env.write_config(&format!(
        "[registries]\n\"http://127.0.0.1:1/\" = \"{}\"\n\n{}",
        env.registry_addr(),
        "[repos]\n\"meta/app\" = { url = \"http://127.0.0.1:1/app.git\", revision = \"v1-2026.10.05-1\" }\n"
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("cannot fetch"), "{}", out.stderr);
    assert!(!out.stderr.contains("registry"), "{}", out.stderr);
}

/// A release tag already naming an image stays where it is: `--reuse`
/// refuses to move it, as a release is published once.
#[test]
fn error_073_reuse_leaves_a_release_already_published_alone() {
    let env = TestEnv::new("art_reuse_taken");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let (out, _) = env.publish_with(
        &bare,
        "main",
        LAYERED,
        &[("vendor/lib.js", "v"), ("app.js", "a")],
        &[],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    push_image(
        &env,
        &bare,
        "v1.0.0",
        &[gzip_layer(
            "app",
            Tar::new().file("dist/other.txt", "other"),
        )],
    );
    let image = env.image(&bare);
    let published = env.registry().manifest(&image, "v1.0.0");

    let (out, _) = env.publish_with(&bare, "main", LAYERED, &[], &["--reuse", "v1.0.0"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains(":v1.0.0 already names another image"),
        "{}",
        out.stderr
    );
    assert_eq!(env.registry().manifest(&image, "v1.0.0"), published);
}

/// A dependency that does not resolve leaves the source hash unknown: a dry
/// run says so and lists the layers, a publish fails, as `--reuse` could not
/// find the image it would push.
#[test]
fn error_074_publish_fails_on_a_source_hash_it_cannot_take() {
    let env = TestEnv::new("art_hash_unknown");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let missing = env.repos_remote.join("missing.git");
    let producer = format!(
        "{}\n{}",
        LAYERED,
        repos(&[("vendor/dep", &missing, ", revision = \"v1.0.0\"")])
    );
    let built = [("vendor/lib.js", "vendor"), ("app.js", "built")];
    let (out, _) = env.publish_with(&bare, "main", &producer, &built, &["--dry-run"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("  tags: unknown ("), "{}", out.stdout);
    assert!(out.stdout.contains("    dist/app.js"), "{}", out.stdout);

    let (out, _) = env.publish_with(&bare, "main", &producer, &built, &[]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("cannot hash the sources"),
        "{}",
        out.stderr
    );
    assert!(env.registry().tags(&env.image(&bare)).is_empty());
}

/// Without access to its sources, a raise takes the newest version its
/// registry has: releases are cut in order on the producer's release
/// branches, so the newest holds every one before it.
#[test]
fn normal_075_without_access_a_raise_takes_the_newest_registry_version() {
    let env = TestEnv::new("art_no_access_raise");
    let app = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let producer = "[artefact]\ninclude = [\"dist/**\"]\n";
    let (out, _) = env.publish_with(
        &app,
        "main",
        producer,
        &[("app.bin", "one")],
        &["v1-2026.10.05-1"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    env.push_commit(&app, "main", "lib.txt", "two");
    let (out, _) = env.publish_with(
        &app,
        "main",
        producer,
        &[("app.bin", "two")],
        &["v1-2026.10.06-1"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    std::fs::rename(&app, env.repos_remote.join("moved.git")).unwrap();
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[("meta/app", &app, ", revision = \"v1-2026.10.05-1\"")])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);

    let out = env.run(&["upgrade", "meta/app"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("meta/app   v1-2026.10.05-1 → v1-2026.10.06-1"),
        "{}",
        out.stdout
    );
    assert!(
        std::fs::read_to_string(env.playground.join(".gitscale.toml"))
            .unwrap()
            .contains("revision = \"v1-2026.10.06-1\""),
    );
}

/// A checkout taken as an artefact downloads none of its history: its refs
/// are listed, its config read from its image, and nothing of git is kept
/// for it — someone without access, or not wanting the sources, gets only
/// the build.
#[test]
fn perf_076_an_artefact_downloads_none_of_its_history() {
    let env = TestEnv::new("art_no_history");
    env.init_playground_git();
    let bare = env.artefact_repo("app", &[("app.bin", "built")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "built");
    let stores = env.playground.join(".git/gitscale/repos");
    assert!(
        !stores.exists() || std::fs::read_dir(&stores).unwrap().next().is_none(),
        "a store was made: {:?}",
        all_names(&stores)
    );
}

/// A release is named with a version; anything else is refused before
/// anything is packed.
#[test]
fn error_077_a_release_must_be_a_version() {
    let env = TestEnv::new("art_release_name");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let (out, _) = env.publish_with(&bare, "main", LAYERED, &[], &["release-1"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("release-1 is not a version: a release is named like v1.4.0"),
        "{}",
        out.stderr
    );
    assert!(env.registry().log().is_empty());
}

/// A release already made of another commit is never taken over: refused
/// before anything is packed or pushed.
#[test]
fn error_078_a_release_of_another_commit_is_refused() {
    let env = TestEnv::new("art_release_taken");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let first = layered(&env, &bare, "v1.0.0", "app v1");
    env.push_commit(&bare, "main", "README.md", "v2");
    env.registry().clear_log();
    let (out, _) = env.publish_with(
        &bare,
        "main",
        LAYERED,
        &[("vendor/lib.js", "vendor v1"), ("app.js", "app v2")],
        &["v1.0.0"],
    );
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains(&format!("v1.0.0 is already the release of {}", &first[..7])),
        "{}",
        out.stderr
    );
    assert!(env.registry().log().is_empty());
    assert_eq!(tag_commit(&bare, "v1.0.0"), first);
}

/// A branch build of a repository whose sources cannot be read, published
/// under its source hash: the build pinned as `hash:<hash>`, and its
/// dependencies.
fn unreadable_build(env: &TestEnv) -> (PathBuf, String) {
    let dep = tagged(env, "dep", &[("v1.0.0", "")]);
    let app = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let producer = format!(
        "[artefact]\ninclude = [\"dist/**\"]\n\n{}",
        repos(&[("vendor/dep", &dep, ", revision = \"v1.0.0\"")])
    );
    let built = [("app.bin", "release")];
    let (out, _) = env.publish_with(&app, "main", &producer, &built, &["v1-2026.10.05-1"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    run_git_pub(&app, &["branch", "feat/x", "main"]);
    env.push_commit(&app, "feat/x", "lib.txt", "the change");
    let built = [("app.bin", "branch build")];
    let (out, _) = env.publish_with(&app, "feat/x", &producer, &built, &[]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let hash = hash_tags(env.registry().tags(&env.image(&app)))
        .into_iter()
        .find(|h| {
            env.registry().manifest(&env.image(&app), h).unwrap()["annotations"]
                ["org.opencontainers.image.revision"]
                == git_stdout(&app, &["rev-parse", "feat/x"]).as_str()
        })
        .unwrap();
    std::fs::rename(&app, env.repos_remote.join("moved.git")).unwrap();
    (app, hash)
}

/// A build pinned by its source hash is taken as it is — here by a
/// workspace that cannot read the sources, which no branch would reach:
/// its files, its dependencies, and `build` as its revision's kind.
#[test]
fn normal_080_a_build_pinned_by_its_source_hash_is_taken() {
    let env = TestEnv::new("art_build_pin");
    let (app, hash) = unreadable_build(&env);
    let pin = format!(", revision = \"hash:{}\"", hash);
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[("meta/app", &app, &pin)])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "branch build");
    assert!(env.playground.join("meta/app/vendor/dep").is_symlink());
    let row = crate::support::status_clean::json_row(&env, "meta/app");
    assert_eq!(row["revision_kind"], "build", "{}", row);
    assert_eq!(row["as_reason"], "no-access", "{}", row);
}

/// A build pin is for trying a build out, never for shipping: the merge
/// gate refuses it, naming the config that holds it.
#[test]
fn error_081_check_refuses_a_build_pin() {
    let env = TestEnv::new("art_build_pin_gate");
    let (app, hash) = unreadable_build(&env);
    let pin = format!(", revision = \"hash:{}\"", hash);
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[("meta/app", &app, &pin)])
    ));
    let out = env.run(&["check"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains(&format!(
            "meta/app is pinned to a build (hash:{}) in .gitscale.toml, not to a release",
            hash
        )),
        "{}",
        out.stderr
    );
}

/// A source hash that names no build fails, saying so; a malformed one too.
#[test]
fn error_082_a_build_pin_needs_a_build() {
    let env = TestEnv::new("art_build_pin_missing");
    let (app, _) = unreadable_build(&env);
    for (revision, said) in [
        (
            format!("hash:{}", "a".repeat(64)),
            "no build of these sources (aaaaaaaaaaaa)",
        ),
        (
            "hash:abc".to_string(),
            "hash: takes a source hash, 64 hex digits",
        ),
    ] {
        let pin = format!(", revision = \"{}\"", revision);
        env.write_config(&format!(
            "{}{}{}",
            env.registries(),
            allow(&env),
            repos(&[("meta/app", &app, &pin)])
        ));
        let out = env.run(&["sync"]);
        assert!(!out.success, "{}: {}", revision, out.stdout);
        assert!(out.stderr.contains(said), "{}: {}", revision, out.stderr);
    }
}

/// With access to the sources, a build pin is a checkout of the commit the
/// build was made from.
#[test]
fn normal_083_with_access_a_build_pin_is_its_commit() {
    let env = TestEnv::new("art_build_pin_sources");
    let app = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&app, &["branch", "feat/x", "main"]);
    let built = env.push_commit(&app, "feat/x", "lib.txt", "the change");
    env.publish(&app, "feat/x", &[("app.bin", "branch build")]);
    let hash = hash_tags(env.registry().tags(&env.image(&app)))
        .pop()
        .unwrap();
    env.write_config(&format!(
        "{}{}",
        env.registries(),
        repos(&[("meta/app", &app, &format!(", revision = \"hash:{}\"", hash))])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let dest = env.playground.join("meta/app");
    assert_eq!(git_stdout(&dest, &["rev-parse", "HEAD"]), built);
    assert!(dest.join("lib.txt").is_file());
}
