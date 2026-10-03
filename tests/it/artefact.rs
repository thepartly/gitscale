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

#[test]
fn normal_001_publish_tags_the_commit_and_annotates_the_image() {
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
    // The repository's own .gitscale.toml first, for consumers resolving
    // its dependencies; then the groups, in order.
    assert_eq!(titles, vec!["gitscale", "vendor", "app"]);
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
fn normal_003_an_annotated_tag_resolves_to_its_commit() {
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

    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "tagged");
    assert_eq!(installed_commit(&env), commit);
}

#[test]
fn normal_004_a_full_sha_is_used_as_given() {
    let env = TestEnv::new("art_full_sha");
    let bare = env.artefact_repo("app", &[("app.bin", "pinned")]);
    let commit = git_stdout(&bare, &["rev-parse", "main"]);
    // Main moves on, unpublished: the pinned commit is unaffected.
    env.push_commit(&bare, "main", "README.md", "later");
    env.write_config(&entry_config(&env, &bare, &commit));

    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "pinned");
}

#[test]
fn normal_005_no_revision_follows_the_default_branch() {
    let env = TestEnv::new("art_default_branch");
    let bare = env.create_bare_repo("app", "trunk", &[("README.md", "app")]);
    env.publish(&bare, "trunk", &[("app.bin", "from trunk")]);
    env.write_config(&format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", artefact = \"replace\" }}\n",
        env.registries(),
        bare.display()
    ));

    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "from trunk");
}

#[test]
fn normal_006_pull_installs_a_replace_image() {
    let env = TestEnv::new("pull_artefact_replace");
    let bare = env.artefact_repo("app", &[("app.bin", "binary-content")]);

    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "main", artefact = "replace" }}
"#,
        env.registries(),
        bare.display(),
    ));

    let out = env.run(&["pull"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("pull_artefact_replace_stdout", redact_shas(&out.stdout));

    // Image paths are the repository's own.
    let file = env.playground.join("meta/app/dist/app.bin");
    assert_eq!(std::fs::read_to_string(file).unwrap(), "binary-content");
    assert!(!env.playground.join("meta/app/.git").exists());
}

#[test]
fn normal_007_pull_takes_a_newly_published_image() {
    let env = TestEnv::new("pull_artefact_local");
    let bare = env.artefact_repo("app", &[("app.bin", "v1-content")]);

    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "main", artefact = "replace" }}
"#,
        env.registries(),
        bare.display(),
    ));

    let out1 = env.run(&["pull"]);
    assert!(out1.success, "first pull stderr: {}", out1.stderr);

    // A new commit on main, and its pipeline's artefact.
    env.push_commit(&bare, "main", "README.md", "v2");
    env.publish(&bare, "main", &[("app.bin", "v2-content")]);

    // Pull should download newer version
    let out2 = env.run(&["pull"]);
    assert!(out2.success, "pull stderr: {}", out2.stderr);
    insta::assert_snapshot!("pull_artefact_local_stdout", redact_shas(&out2.stdout));

    let content = std::fs::read_to_string(env.playground.join("meta/app/dist/app.bin")).unwrap();
    assert_eq!(content, "v2-content");
}

#[test]
fn normal_008_fetch_records_what_the_registry_holds_and_installs_nothing() {
    let env = TestEnv::new("fetch_artefact_local");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0", "main"]);
    env.publish(&bare, "v1.0", &[("app.bin", "v1-content")]);

    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "v1.0", artefact = "replace" }}
"#,
        env.registries(),
        bare.display(),
    ));

    let out = env.run(&["fetch"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("fetch_artefact_local_stdout", redact_shas(&out.stdout));

    // What the fetch saw is recorded outside the checkout; nothing is
    // downloaded, and no directory is made for it.
    assert!(!env.playground.join("meta/app").exists());
}

#[test]
fn normal_009_status_says_behind_and_missing_after_a_fetch() {
    let env = TestEnv::new("art_status_behind");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);
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
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "v2");
}

#[test]
fn normal_010_a_republished_commit_shows_as_changed_and_pull_takes_it() {
    let env = TestEnv::new("art_status_changed");
    let bare = env.artefact_repo("app", &[("app.bin", "first build")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);

    let artefact = "[artefact]\ninclude = [\"dist/**\"]\n";
    let (out, _) = env.publish_with(
        &bare,
        "main",
        artefact,
        &[("app.bin", "second build")],
        &["--force"],
    );
    assert!(out.success, "{}", out.stderr);
    // A pull alone asks the registry nothing: the commit has not moved.
    env.registry().clear_log();
    assert!(env.run(&["pull"]).success);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "first build");
    assert!(
        env.registry().log().is_empty(),
        "a pull of an unmoved commit asked the registry: {:?}",
        env.registry().log()
    );

    assert!(env.run(&["fetch"]).success);
    assert!(
        status_line(&env).ends_with("changed"),
        "{}",
        status_line(&env)
    );
    assert!(env.run(&["pull"]).success);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "second build");
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));
}

#[test]
fn normal_011_status_json_carries_the_installed_commit_and_digest() {
    let env = TestEnv::new("art_status_json");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    let commit = git_stdout(&bare, &["rev-parse", "main"]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);

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
fn normal_012_changing_the_configured_revision_is_a_ref_mismatch() {
    let env = TestEnv::new("art_ref_mismatch");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    run_git_pub(&bare, &["tag", "v1", "main"]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);
    env.write_config(&entry_config(&env, &bare, "v1"));
    assert!(
        status_line(&env).ends_with("ref-mismatch"),
        "{}",
        status_line(&env)
    );
    assert!(env.run(&["pull"]).success);
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));
}

/// What gitscale records about a checkout lives in the workspace's git
/// directory, never in the checkout, whose every file is the artefact's.
#[test]
fn normal_013_records_live_in_the_git_directory_not_the_checkout() {
    let env = TestEnv::new("art_records");
    env.init_playground_git();
    let bare = env.artefact_repo("app", &[("app.bin", "x"), (".env", "y")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);

    let listed = |dir: PathBuf| {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    // The producer's .gitscale.toml arrives with its config layer.
    assert_eq!(
        listed(env.playground.join("meta/app")),
        vec![".gitscale.toml", "dist"]
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
        shown(&out.stdout, "layers").starts_with("gitscale "),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("app  "), "{}", out.stdout);
    assert_eq!(shown(&out.stdout, "installed"), "nothing");
    assert_eq!(shown(&out.stdout, "status"), "not installed");

    assert!(env.run(&["pull"]).success);
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
fn normal_015_list_shows_every_published_commit_with_its_refs() {
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
    assert!(env.run(&["pull"]).success);

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

/// `pull` prunes the image store by itself — at most once a day, so the cost
/// on every other pull is one stat.
#[test]
fn normal_016_pull_prunes_cold_images_once_a_day() {
    let env = TestEnv::new("art_daily_prune");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let old = layered(&env, &bare, "app v1");
    env.write_config(&format!(
        "[clean]\nkeep_recent = \"30d\"\n\n{}",
        entry_config(&env, &bare, "main")
    ));
    assert!(env.run(&["pull"]).success);
    env.push_commit(&bare, "main", "README.md", "v2");
    layered(&env, &bare, "app v2");
    let entry = image_store(&env);
    let held = || std::fs::read_to_string(entry.join("index.json")).unwrap();

    // Pruned today already: nothing goes.
    age(&entry, &old);
    assert!(env.run(&["pull"]).success);
    assert!(held().contains(&old));

    // A day later, it does.
    let marker = env.playground.join(".git/gitscale/images/gitscale-pruned");
    let touched = std::process::Command::new("touch")
        .args(["-d", "2000-01-01", marker.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(touched.success());
    assert!(env.run(&["pull"]).success);
    assert!(!held().contains(&old), "{}", held());
}

/// On the topic, an artefact that replaces its checkout is the image of the
/// branch tip when one is published, and that tip's source when not.
#[test]
fn normal_017_an_artefact_on_a_topic_is_the_tips_image_or_its_source() {
    let env = TestEnv::new("wt_artefact_topic");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, "replace");
    ok(&gs(&ws, &["pull"]));
    let dest = ws.join("meta/app");
    assert!(dest.join("dist/app.bin").is_file());

    // A branch of the topic, published.
    run_git_pub(&app, &["branch", "feat/x", "main"]);
    env.push_commit(&app, "feat/x", "README.md", "feature");
    env.publish(&app, "feat/x", &[("app.bin", "feature build")]);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "feature build"
    );
    assert!(!dest.join(".git").exists());

    // A newer tip with no image yet: its source, detached.
    let unpublished = env.push_commit(&app, "feat/x", "README.md", "newer");
    let out = gs(&ws, &["pull"]);
    ok(&out);
    assert!(dest.join(".git").is_file(), "{}", out.stdout);
    assert_eq!(head(&dest), unpublished);
    assert_eq!(branch(&dest), None);

    // Developed: the source, on its branch.
    ok(&gs(&ws, &["develop", "meta/app"]));
    assert_eq!(branch(&dest).as_deref(), Some("feat/x"));

    // Back on main: the image again.
    run_git_pub(&dest, &["checkout", "-q", "--detach"]);
    run_git_pub(&dest, &["branch", "-q", "-D", "feat/x"]);
    run_git_pub(&ws, &["switch", "-q", "main"]);
    run_git_pub(&app, &["branch", "-D", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    assert!(!dest.join(".git").exists());
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "main build"
    );
}

/// An overlay is the source checkout with the image's untracked files laid
/// over it: tracked files are the checkout's own, and the next overlay
/// removes what this one wrote before laying its own. `clean` keeps them.
#[test]
fn normal_018_an_overlay_lays_the_build_over_the_source() {
    let env = TestEnv::new("wt_overlay");
    // Build output is ignored, as the artefact policy has it.
    let app = env.create_bare_repo(
        "app",
        "main",
        &[
            ("README.md", "app"),
            ("src.txt", "source"),
            (".gitignore", "/dist/\n"),
        ],
    );
    let producer_files = [("app.bin", "v1 build"), ("old.bin", "goes away")];
    env.publish(&app, "main", &producer_files);
    let ws = artefact_workspace(&env, &app, "overlay");
    ok(&gs(&ws, &["pull"]));
    let dest = ws.join("meta/app");
    assert!(dest.join(".git").is_file(), "a source worktree");
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "v1 build"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("src.txt")).unwrap(),
        "source"
    );
    assert!(
        git(&dest, &["status", "--porcelain"]).is_empty(),
        "the overlay is ignored output"
    );

    // A clean keeps the overlay's files.
    ok(&gs(&ws, &["clean", "-f"]));
    assert!(dest.join("dist/app.bin").is_file());

    // A new commit with a smaller build: the old file goes.
    env.push_commit(&app, "main", "README.md", "v2");
    env.publish(&app, "main", &[("app.bin", "v2 build")]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "v2 build"
    );
    assert!(!dest.join("dist/old.bin").exists());
}

/// Images other tools publish unpack too: a Docker-typed gzip layer and an
/// uncompressed OCI tar layer, in order, so a later layer's file replaces an
/// earlier one's. gitscale's own publisher writes neither, but an artefact
/// is an ordinary image, and the docs say any registry and tool works.
#[test]
fn normal_044_foreign_layer_media_types_unpack_in_order() {
    let env = TestEnv::new("artefact_foreign_media_types");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let commit = tip(&bare, "main");
    push_image(
        &env,
        &bare,
        &commit,
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
    env.write_config(&entry_config(&env, &bare, "main"));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/a.txt"), "from docker");
    assert_eq!(read(&env, "meta/app/dist/b.txt"), "from plain tar");
    assert_eq!(read(&env, "meta/app/dist/shared.txt"), "second");
    assert_eq!(installed_commit(&env), commit);
}

/// In a GitLab job, `publish` tags the job's commit (`CI_COMMIT_SHA`) — not
/// whatever `HEAD` is — under the job's project (`CI_PROJECT_URL`), which
/// is the URL consumers declare. A wrong commit or image name would publish
/// an artefact no consumer ever finds, or one under another commit's tag.
#[test]
fn normal_045_publish_in_a_gitlab_job_tags_the_jobs_commit_under_its_project() {
    let env = TestEnv::new("artefact_publish_gitlab_job");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let job_commit = tip(&bare, "main");
    // HEAD moves on with a change the image does not ship.
    env.push_commit(&bare, "main", "README.md", "later");
    let producer = producer_with_build(&env, &bare);
    let project = format!("{}/project.git", env.repos_remote.display());

    let out = run_bin_in(
        &env,
        &producer,
        &[
            ("GITLAB_CI", "true"),
            ("CI_COMMIT_SHA", &job_commit),
            ("CI_PROJECT_URL", &project),
        ],
        &["artefact", "publish"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        env.registry().tags("project/gitscale"),
        vec![job_commit.clone()]
    );
    assert!(env.registry().tags("app/gitscale").is_empty());
    let manifest = env
        .registry()
        .manifest("project/gitscale", &job_commit)
        .unwrap();
    assert_eq!(
        manifest["annotations"]["org.opencontainers.image.source"],
        project.as_str()
    );
    assert_eq!(
        manifest["annotations"]["org.opencontainers.image.revision"],
        job_commit.as_str()
    );
}

/// The same in a GitHub workflow: `GITHUB_SHA` is the commit, and
/// `GITHUB_SERVER_URL`/`GITHUB_REPOSITORY` name the repository.
#[test]
fn normal_046_publish_in_a_github_workflow_tags_its_commit_under_its_repository() {
    let env = TestEnv::new("artefact_publish_github_workflow");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let workflow_commit = tip(&bare, "main");
    env.push_commit(&bare, "main", "README.md", "later");
    let producer = producer_with_build(&env, &bare);
    let server = env.repos_remote.display().to_string();

    let out = run_bin_in(
        &env,
        &producer,
        &[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", &workflow_commit),
            ("GITHUB_SERVER_URL", &server),
            ("GITHUB_REPOSITORY", "org/proj"),
        ],
        &["artefact", "publish"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        env.registry().tags("org/proj/gitscale"),
        vec![workflow_commit.clone()]
    );
    let manifest = env
        .registry()
        .manifest("org/proj/gitscale", &workflow_commit)
        .unwrap();
    assert_eq!(
        manifest["annotations"]["org.opencontainers.image.source"],
        format!("{}/org/proj", server).as_str()
    );
}

/// An overlay's files are read-only on a detached checkout, as the rest of
/// it is, and writable once the checkout is on a topic branch somebody
/// develops in: a build there has to be able to replace them.
#[test]
fn normal_047_an_overlay_is_readonly_off_a_topic_and_writable_on_its_branch() {
    let env = TestEnv::new("artefact_overlay_modes");
    let app = env.create_bare_repo(
        "app",
        "main",
        &[("README.md", "app"), (".gitignore", "/dist/\n")],
    );
    env.publish(&app, "main", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, "overlay");
    ok(&gs(&ws, &["pull"]));
    let dest = ws.join("meta/app");
    assert!(!writable(&dest.join("dist/app.bin")));

    run_git_pub(&app, &["branch", "feat/x", "main"]);
    env.push_commit(&app, "feat/x", "README.md", "feature");
    env.publish(&app, "feat/x", &[("app.bin", "feature build")]);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&dest).as_deref(), Some("feat/x"));
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "feature build"
    );
    assert!(writable(&dest.join("dist/app.bin")));
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

#[test]
fn edge_020_an_all_hex_tag_name_is_a_tag() {
    let env = TestEnv::new("art_hex_tag");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "20241001", "main"]);
    env.publish(&bare, "20241001", &[("app.bin", "dated")]);
    env.write_config(&entry_config(&env, &bare, "20241001"));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "dated");
}

#[test]
fn edge_021_a_branch_name_matches_exactly() {
    let env = TestEnv::new("art_exact_branch");
    let bare = env.artefact_repo("app", &[("app.bin", "main")]);
    // `ls-remote <url> main` would also list this one.
    run_git_pub(&bare, &["branch", "feature/main", "main"]);
    env.push_commit(&bare, "feature/main", "README.md", "f");
    env.write_config(&entry_config(&env, &bare, "main"));

    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "main");
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
    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "main", artefact = "replace" }}
"#,
        env.registries(),
        bare.display(),
    ));
    let root = env.playground.join("meta/app");
    let dest = root.join("dist");

    let out = env.run(&["pull"]);
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
    // The producer's .gitscale.toml arrives with its config layer.
    assert_eq!(listed(&root), vec![".gitscale.toml", "dist"]);
    assert_eq!(
        listed(&dest),
        vec![".config", ".env", "app.bin", "bin", "share"]
    );

    // An update has to get past the read-only files it replaces.
    env.push_commit(&bare, "main", "README.md", "v2");
    env.publish(&bare, "main", &files("v2"));
    let out = env.run(&["pull"]);
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
/// downloaded by the pull that follows.
#[test]
fn edge_023_pull_after_fetch_still_downloads_the_artefact() {
    let env = TestEnv::new("pull_after_fetch_artefact");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    run_git_pub(&bare, &["tag", "v1.0", "main"]);
    env.publish(&bare, "v1.0", &[("app.bin", "v1-content")]);
    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "v1.0", artefact = "replace" }}
"#,
        env.registries(),
        bare.display(),
    ));

    assert!(env.run(&["fetch"]).success);
    let out = env.run(&["pull"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join("meta/app/dist/app.bin").is_file(),
        "the artefact was never downloaded; pull said:\n{}",
        out.stdout
    );
}

/// A directory gitscale has no record of installing — one from an older
/// gitscale, or made by hand — is replaced by `pull`, dot files and all.
#[test]
fn edge_024_an_old_style_checkout_is_replaced_on_pull() {
    let env = TestEnv::new("art_legacy");
    let bare = env.artefact_repo("app", &[("app.bin", "new")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    let dest = env.playground.join("meta/app");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join(".etag"), "\"abc\"").unwrap();
    std::fs::write(dest.join("stale.bin"), "old").unwrap();

    let out = env.run(&["pull"]);
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
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);
    std::fs::remove_dir_all(env.playground.join("meta/app")).unwrap();
    assert!(
        status_line(&env).ends_with("missed"),
        "{}",
        status_line(&env)
    );
    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert!(!out.stdout.contains("up to date"), "{}", out.stdout);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "x");
}

#[test]
fn edge_026_list_of_a_repository_nothing_published_for() {
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

/// A blob that no longer matches its digest is never used: it is deleted and
/// downloaded again.
#[test]
fn edge_027_a_damaged_blob_is_downloaded_again() {
    let env = TestEnv::new("art_damaged_blob");
    let bare = env.artefact_repo("app", &[("app.bin", "intact")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);

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
    assert!(env.run(&["pull"]).success);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "intact");
    // The damaged one, and only that — manifest or layer, whichever it was.
    let gets = env.registry().count("GET", "");
    assert_eq!(gets, 1, "{:?}", env.registry().log());
}

/// An overlay lays only what the checkout does not track: a tracked file is
/// the commit's own, and the image's `.gitscale.toml` is for consumers to
/// read, never laid over the checkout's. An image that ships either with
/// other content — from another tool, or a forced publish — must not change
/// the source the checkout holds.
#[test]
fn edge_048_an_overlay_never_replaces_a_tracked_file_or_the_config() {
    let env = TestEnv::new("artefact_overlay_tracked");
    let app = env.create_bare_repo(
        "app",
        "main",
        &[
            ("README.md", "app"),
            ("src.txt", "source"),
            (".gitignore", "/dist/\n"),
            (".gitscale.toml", "# the source's own\n"),
        ],
    );
    let commit = tip(&app, "main");
    push_image(
        &env,
        &app,
        &commit,
        &[gzip_layer(
            "app",
            Tar::new()
                .file("dist/app.bin", "built")
                .file("src.txt", "from the image")
                .file(".gitscale.toml", "# from the image\n"),
        )],
    );
    env.write_config(&overlay_config(&env, &app, "main"));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "built");
    assert_eq!(read(&env, "meta/app/src.txt"), "source");
    assert_eq!(
        read(&env, "meta/app/.gitscale.toml"),
        "# the source's own\n"
    );
    assert!(git(&env.playground.join("meta/app"), &["status", "--porcelain"]).is_empty());
}

/// An overlay checkout deleted by hand — or by a CI runner's `git clean`
/// of the root — and cloned again at the same commit gets its build again.
/// What gitscale recorded about the old checkout says nothing about the new
/// one: a source tree without its build is what the overlay exists to
/// prevent.
#[test]
#[ignore = "bug: overlay is skipped when its record names the commit, though the files are gone"]
fn edge_049_a_recreated_overlay_checkout_gets_its_build_again() {
    let env = TestEnv::new("artefact_overlay_recreated");
    let app = env.create_bare_repo(
        "app",
        "main",
        &[("README.md", "app"), (".gitignore", "/dist/\n")],
    );
    env.publish(&app, "main", &[("app.bin", "built")]);
    env.write_config(&overlay_config(&env, &app, "main"));
    assert!(env.run(&["pull"]).success);
    let dest = env.playground.join("meta/app");
    assert!(dest.join("dist/app.bin").is_file());

    make_tree_writable(&dest);
    std::fs::remove_dir_all(&dest).unwrap();
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(dest.join(".git").is_file(), "cloned again");
    assert!(
        dest.join("dist/app.bin").is_file(),
        "the overlay was not laid again: {}",
        out.stdout
    );
}

/// On a topic, a branch tip with no image yet keeps the overlay that is
/// there: the checkout moves to the tip, and the previous build stays, as
/// the topics table in the docs says.
#[test]
fn edge_050_an_overlay_on_a_topic_without_an_image_keeps_the_previous_build() {
    let env = TestEnv::new("artefact_overlay_topic_unpublished");
    let app = env.create_bare_repo(
        "app",
        "main",
        &[("README.md", "app"), (".gitignore", "/dist/\n")],
    );
    env.publish(&app, "main", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, "overlay");
    ok(&gs(&ws, &["pull"]));
    let dest = ws.join("meta/app");

    run_git_pub(&app, &["branch", "feat/x", "main"]);
    let unpublished = env.push_commit(&app, "feat/x", "README.md", "feature");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["pull"]);
    ok(&out);
    assert_eq!(head(&dest), unpublished);
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "main build"
    );
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
    let first = tip(&bare, "main");
    push_image(
        &env,
        &bare,
        &first,
        &[gzip_layer(
            "app",
            Tar::new().dir("ro", 0o555).file("ro/f.txt", "v1"),
        )],
    );
    env.write_config(&entry_config(&env, &bare, "main"));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/ro/f.txt"), "v1");

    let second = env.push_commit(&bare, "main", "README.md", "v2");
    push_image(
        &env,
        &bare,
        &second,
        &[gzip_layer(
            "app",
            Tar::new().dir("ro", 0o555).file("ro/f.txt", "v2"),
        )],
    );
    let out = env.run(&["pull"]);
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
    let commit = tip(&bare, "main");
    push_image(
        &env,
        &bare,
        &commit,
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
    env.write_config(&entry_config(&env, &bare, "main"));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let dist = env.playground.join("meta/app/dist");
    assert!(dist.join("keep.txt").is_file());
    assert!(dist.join("old.txt").is_file());
    assert!(dist.join(".wh.old.txt").is_file());
}

/// `publish --commit` checks the artefact policy against the commit named,
/// not `HEAD`: a file tracked there that the working tree has changed since
/// breaks it. A short SHA is refused: an image is tagged with a full one.
#[test]
fn edge_053_publish_commit_checks_the_policy_against_that_commit() {
    let env = TestEnv::new("artefact_publish_commit_flag");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app"), ("src/lib.rs", "v1")]);
    let first = tip(&bare, "main");
    let second = env.push_commit(&bare, "main", "src/lib.rs", "v2");
    let producer = producer_with_build(&env, &bare);
    std::fs::write(
        producer.join(".gitscale.toml"),
        format!(
            "{}[artefact]\ninclude = [\"dist/**\", \"src/**\"]\n",
            env.registries()
        ),
    )
    .unwrap();

    let out = env.run_in(&producer, &["artefact", "publish", "--commit", &first]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains(&format!("tracked, modified since {}", &first[..7])),
        "{}",
        out.stderr
    );
    let out = env.run_in(&producer, &["artefact", "publish", "--commit", &first[..9]]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("not a full commit SHA"),
        "{}",
        out.stderr
    );
    assert!(env.registry().tags(&env.image(&bare)).is_empty());

    let out = env.run_in(&producer, &["artefact", "publish", "--commit", &second]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(env.registry().tags(&env.image(&bare)), vec![second]);
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
    assert!(out.stdout.contains("  commit: unknown ("), "{}", out.stdout);
    assert!(out.stdout.contains("  image: unknown ("), "{}", out.stdout);
    assert!(
        out.stdout.contains("  policy: not checked ("),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("    dist/a.txt"), "{}", out.stdout);
    assert!(env.registry().log().is_empty());
}

/// On a topic, a `replace` entry installed from the branch tip's image is
/// what the topic asks for: `status --fetch` calls it ok, not behind the
/// pinned revision.
#[test]
fn edge_056_status_fetch_on_a_topic_calls_the_installed_tip_ok() {
    let env = TestEnv::new("artefact_topic_status_fetch");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, "replace");
    run_git_pub(&app, &["branch", "feat/x", "main"]);
    env.push_commit(&app, "feat/x", "README.md", "feature");
    env.publish(&app, "feat/x", &[("app.bin", "feature build")]);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["pull"]));

    let out = gs(&ws, &["status", "--fetch"]);
    ok(&out);
    let row = support::strip_ansi(&out.stdout)
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some("meta/app"))
        .unwrap_or_default()
        .to_string();
    assert!(row.contains(" ok "), "{}", row);
    assert!(!row.contains("behind"), "{}", row);
}

/// A forced re-publish that changes the image's `.gitscale.toml` changes
/// what resolution reads: after a fetch says the image changed, the pull
/// that installs the new files also brings the dependencies the new config
/// declares. Files from one build and dependencies from another would be a
/// checkout nobody published.
#[test]
#[ignore = "bug: resolution reads the config layer of the image held for the commit, not the republished one"]
fn edge_057_a_forced_republish_brings_the_dependencies_it_declares() {
    let env = TestEnv::new("artefact_republish_config");
    let dep = tagged(&env, "dep", &[("v1.0.0", "")]);
    let art = env.create_bare_repo("art", "main", &[("README.md", "art")]);
    let plain = "[artefact]\ninclude = [\"dist/**\"]\n";
    let (out, _) = env.publish_with(&art, "main", plain, &[("app.bin", "first")], &[]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[(
            "meta/app",
            &art,
            ", revision = \"main\", artefact = \"replace\""
        )])
    ));
    assert!(env.run(&["pull"]).success);

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
    let out = env.run(&["pull"]);
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

    let out = env.run(&["pull"]);
    assert!(!out.success);
    assert!(out.stderr.contains("full SHA"), "{}", out.stderr);
    assert!(
        out.stderr.contains(&format!("git rev-parse {}", short)),
        "{}",
        out.stderr
    );
}

#[test]
fn error_033_pull_without_a_registry_says_how_to_add_one() {
    let env = TestEnv::new("pull_no_registry");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);

    env.write_config(&format!(
        r#"[repos]
"meta/app" = {{ url = "{}", revision = "main", artefact = "replace" }}
"#,
        bare.display()
    ));

    let out = env.run(&["pull"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("no registry is known") && out.stderr.contains("[registries]"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn error_034_pull_fails_when_nothing_is_published() {
    let env = TestEnv::new("pull_artefact_no_data");
    // A repository whose pipeline has published nothing.
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);

    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "main", artefact = "replace" }}
"#,
        env.registries(),
        bare.display(),
    ));

    let out = env.run(&["pull"]);
    assert!(!out.success, "a missing artefact is an error, not a skip");
    assert!(
        out.stderr.contains("no artefact for") && out.stderr.contains("(main)"),
        "stderr: {}",
        out.stderr
    );
    assert!(!env.playground.join("meta/app").exists());
}

/// An archive that cannot be unpacked leaves nothing that later passes for
/// the artefact: running pull again tries again, rather than taking the
/// half-made directory as done.
#[test]
fn error_035_a_corrupt_artefact_is_not_taken_as_current() {
    let env = TestEnv::new("corrupt_artefact");
    let bare = env.artefact_repo("app", &[("app.bin", "v1-content")]);
    env.registry().corrupt_blobs();
    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "main", artefact = "replace" }}
"#,
        env.registries(),
        bare.display(),
    ));

    let first = env.run(&["pull"]);
    assert!(!first.success, "the blob does not match its digest");
    assert!(
        first.stderr.contains("does not match its digest"),
        "{}",
        first.stderr
    );
    let again = env.run(&["pull"]);
    assert!(
        !again.success,
        "pull took the failed one as up to date:\n{}",
        again.stdout
    );
    assert!(!env.playground.join("meta/app/dist/app.bin").exists());
    // Nothing is recorded as installed. (The failure here comes from the
    // config layer resolution reads; artefact::error_058 has one in the
    // install itself.)
    assert_eq!(installed_commit(&env), "");
}

#[test]
fn error_036_show_reports_an_entry_it_cannot_look_up_and_fails() {
    let env = TestEnv::new("art_show_error");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    // No [registries]: nothing maps this repository to a registry.
    env.write_config(&format!(
        "[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"main\", artefact = \"replace\" }}\n",
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
fn error_037_show_and_list_refuse_an_entry_that_is_not_an_artefact() {
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

/// A registry that serves a damaged layer during an update leaves the
/// installed version alone: everything is downloaded and checked before the
/// old files go, as the docs promise. The installed files, the record of
/// them and the status all still say the old commit.
#[test]
fn error_058_a_registry_failing_mid_update_keeps_the_installed_version() {
    let env = TestEnv::new("artefact_mid_update_failure");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let first = layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);

    env.push_commit(&bare, "main", "README.md", "v2");
    let second = layered(&env, &bare, "app v2");
    // The config and vendor layers are the same as before; the app layer is
    // new, and the one the registry damages.
    let app_layer = layer_digests(&env, &bare, &second)[2].clone();
    env.registry().corrupt_blob(&app_layer);
    let out = env.run(&["pull"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("does not match its digest"),
        "{}",
        out.stderr
    );
    assert_eq!(read(&env, "meta/app/dist/app.js"), "app v1");
    assert_eq!(read(&env, "meta/app/dist/vendor/lib.js"), "vendor v1");
    assert_eq!(installed_commit(&env), first);
    assert!(status_line(&env).ends_with("ok"), "{}", status_line(&env));
}

/// A tag that names an image index — what `docker buildx` pushes with
/// provenance, or a multi-platform image — is not an image with no layers.
/// Taking it for one would wipe the installed files and record an empty
/// checkout as the artefact, and every later pull would succeed doing it
/// again.
#[test]
#[ignore = "bug: a manifest without layers (an OCI index) is installed as an empty artefact"]
fn error_059_an_image_index_is_refused_rather_than_installed_empty() {
    let env = TestEnv::new("artefact_index_refused");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    let first = tip(&bare, "main");
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);

    let second = env.push_commit(&bare, "main", "README.md", "v2");
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
        &second,
        &index,
        support::registry::INDEX_TYPE,
    );
    let out = env.run(&["pull"]);
    assert!(!out.success, "an index was installed: {}", out.stdout);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "v1");
    assert_eq!(installed_commit(&env), first);
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
        push_image(&env, &bare, &tip(&bare, "main"), &[gzip_layer("app", tar)]);
        env.write_config(&entry_config(&env, &bare, "main"));
        let out = env.run(&["pull"]);
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
        assert_eq!(installed_commit(&env), "", "{}", name);
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
    push_image(
        &env,
        &bare,
        &tip(&bare, "main"),
        &[gzip_layer(
            "app",
            Tar::new()
                .file("dist/ok.txt", "ok")
                .dir("a", 0o755)
                .symlink("a/b", "..")
                .symlink("a/b/l", "../escaped"),
        )],
    );
    env.write_config(&entry_config(&env, &bare, "main"));
    let out = env.run(&["pull"]);
    let escaped = env.playground.join("meta/app/l");
    assert!(
        std::fs::symlink_metadata(&escaped).is_err(),
        "{} -> {:?} was installed",
        escaped.display(),
        std::fs::read_link(&escaped)
    );
    assert!(!out.success, "{}", out.stdout);
}

/// An overlay writes only inside its checkout. A path in the image under a
/// directory the checkout holds as a symlink — tracked, or a dependency
/// link gitscale planted — must not be followed: here `link -> ../../victim`
/// would have the overlay write into another directory of the workspace.
#[test]
#[ignore = "bug: the overlay copies files through symlinked directories of the checkout"]
fn error_062_an_overlay_never_writes_through_a_symlink_in_the_checkout() {
    let env = TestEnv::new("artefact_overlay_through_link");
    let victim = env.playground.join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    let app = env.create_bare_repo(
        "app",
        "main",
        &[("README.md", "app"), (".gitignore", "/dist/\n")],
    );
    let commit = commit_symlink(&env, &app, "main", "link", "../../victim");
    push_image(
        &env,
        &app,
        &commit,
        &[gzip_layer(
            "app",
            Tar::new()
                .file("dist/app.bin", "built")
                .file("link/evil.txt", "written through the link"),
        )],
    );
    env.write_config(&overlay_config(&env, &app, "main"));
    let _ = env.run(&["pull"]);
    assert!(
        !victim.join("evil.txt").exists(),
        "the overlay wrote outside its checkout"
    );
}

/// A layer of a kind gitscale cannot unpack — a Helm chart, say — fails the
/// install with the media type named, rather than being unpacked as a tar
/// or skipped.
#[test]
fn error_063_an_unknown_layer_media_type_is_refused() {
    let env = TestEnv::new("artefact_unknown_media_type");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    push_image(
        &env,
        &bare,
        &tip(&bare, "main"),
        &[Layer {
            media_type: "application/vnd.cncf.helm.chart.content.v1.tar+gzip",
            bytes: Tar::new().file("dist/chart.yaml", "x").gzip(),
            title: "chart",
        }],
    );
    env.write_config(&entry_config(&env, &bare, "main"));
    let out = env.run(&["pull"]);
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
    env.registry().put_manifest(
        &env.image(&bare),
        &tip(&bare, "main"),
        &manifest,
        support::registry::MANIFEST_TYPE,
    );
    env.write_config(&entry_config(&env, &bare, "main"));
    let out = env.run(&["pull"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("the image manifest names an invalid layer digest: sha256:../../escape"),
        "{}",
        out.stderr
    );
    assert_eq!(env.registry().count("GET", "/blobs/"), 0);
}

/// Off a topic, an overlay entry whose commit has no image fails, rather
/// than handing over the sources without their build.
#[test]
fn error_065_an_overlay_without_an_image_fails_off_a_topic() {
    let env = TestEnv::new("artefact_overlay_unpublished");
    let app = env.create_bare_repo(
        "app",
        "main",
        &[("README.md", "app"), (".gitignore", "/dist/\n")],
    );
    env.write_config(&overlay_config(&env, &app, "main"));
    let out = env.run(&["pull"]);
    assert!(!out.success, "{}", out.stdout);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(text.contains("no artefact for"), "{}", text);
    assert!(text.contains("may not have published yet"), "{}", text);
    assert!(!env.playground.join("meta/app/dist").exists());
}

/// `status --fetch` that cannot reach the registry still prints the table,
/// says the entry's row is what the last successful fetch saw, and does not
/// pass that off as fresh.
#[test]
fn error_066_status_fetch_without_the_registry_shows_the_last_fetched_state() {
    let env = TestEnv::new("artefact_status_fetch_refused");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);
    env.registry().refuse_with(500);

    let out = env.run(&["status", "--fetch"]);
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
fn perf_039_a_pull_downloads_only_the_layer_that_changed() {
    let env = TestEnv::new("art_layer_pull");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["pull"]).success);
    // The config layer, read by resolution and kept, then vendor and app.
    assert_eq!(blob_downloads(&env), 3);

    env.push_commit(&bare, "main", "README.md", "v2");
    layered(&env, &bare, "app v2");
    env.registry().clear_log();
    let out = env.run(&["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.js"), "app v2");
    assert_eq!(read(&env, "meta/app/dist/vendor/lib.js"), "vendor v1");
    assert_eq!(blob_downloads(&env), 1, "{:?}", env.registry().log());
}

#[test]
fn perf_040_an_up_to_date_pull_asks_the_registry_nothing() {
    let env = TestEnv::new("pull_artefact_up_to_date");
    let bare = env.artefact_repo("app", &[("app.bin", "content")]);

    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "main", artefact = "replace" }}
"#,
        env.registries(),
        bare.display(),
    ));

    assert!(env.run(&["pull"]).success);
    env.registry().clear_log();

    // Pull again: the revision still names the installed commit, so the
    // registry is not asked anything.
    let out = env.run(&["pull"]);
    assert!(out.success, "stderr: {}", out.stderr);
    insta::assert_snapshot!("pull_artefact_up_to_date_stdout", redact_shas(&out.stdout));
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
    layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));
    env.init_playground_git();
    run_git_pub(&env.playground, &["add", ".gitscale.toml"]);
    run_git_pub(&env.playground, &["commit", "-q", "-m", "config"]);
    assert!(env.run(&["pull"]).success);

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
    let out = env.run_in(&other, &["pull"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(
        std::fs::read_to_string(other.join("meta/app/dist/app.js")).unwrap(),
        "app v1"
    );
    assert_eq!(blob_downloads(&env), 0, "{:?}", env.registry().log());
    assert_eq!(env.registry().count("GET", "/manifests/"), 0);
}

/// A CI job without the cache keeps nothing: the config layer resolution
/// reads is downloaded again by the install, four blobs a pull.
#[test]
fn perf_042_in_ci_without_the_cache_every_layer_is_downloaded() {
    let env = TestEnv::new("art_ci_no_cache");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));
    for _ in 0..2 {
        let _ = std::fs::remove_dir_all(env.playground.join("meta"));
        let out = env.run_with_env(&[("CI", "true")], &["pull", "--no-cache"]);
        assert!(out.success, "{}{}", out.stdout, out.stderr);
    }
    assert_eq!(blob_downloads(&env), 8);
    assert!(env.cache_entries("images").is_empty());
}

/// N cold CI jobs on one runner, sharing its cache, download each blob once.
#[test]
fn perf_043_parallel_cold_ci_jobs_download_each_blob_once() {
    let env = TestEnv::new("art_parallel");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "app v1");
    let workspaces: Vec<PathBuf> = (0..4)
        .map(|i| env.repos_remote.join(format!("ws{}", i)))
        .collect();
    for ws in &workspaces {
        std::fs::create_dir_all(ws).unwrap();
        run_git_pub(ws, &["init", "-q"]);
        std::fs::write(ws.join(".gitscale.toml"), entry_config(&env, &bare, "main")).unwrap();
    }
    std::thread::scope(|scope| {
        for ws in &workspaces {
            let cache = env.cache.clone();
            scope.spawn(move || {
                let out = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"))
                    .args(["pull", "-C", ws.to_str().unwrap()])
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
    assert_eq!(blob_downloads(&env), 3, "{:?}", env.registry().log());
}
