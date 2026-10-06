//! The CI cache: exact commits CI jobs asked for, kept in the user's own data
//! directory so the next job on the runner downloads nothing.
//!
//! A developer machine keeps no cache — its stores live in the root's own git
//! directory, see `stores` — so every test here that syncs does it as a CI
//! job, and asserts on two things at once: that the cache holds the commit,
//! and that the checkout was built from it.

use crate::support;
use crate::support::artefacts::*;
use crate::support::ci_cache::*;
use crate::support::ci_cache::{age, ci_pull};
use crate::support::resolution::*;
use crate::support::TestEnv;

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_ci_takes_a_pinned_commit_out_of_a_snapshot_entry() {
    let env = TestEnv::new("cache_ci_pin");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    // file:// so git honours --depth; it ignores depth for a plain local path.
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    let head = git(&bare, &["rev-parse", "main"]);

    let out = ci_pull(&env);
    assert!(out.success, "{:?}", out.stderr);

    let entry = env.cache_entry("snapshots", &url);
    assert_eq!(
        git(&entry, &["rev-parse", &format!("refs/heads/pin/{}", head)]),
        head,
        "the pinned commit should be a ref inside the entry"
    );

    let checkout = env.playground.join("libs/core");
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&checkout, &["rev-parse", "--is-shallow-repository"]),
        "true",
        "a snapshot entry is shallow by design, and so is what comes out of it"
    );
    assert!(
        !git_ok(&checkout, &["symbolic-ref", "-q", "HEAD"]),
        "a CI checkout is detached at the commit, as a workspace's is"
    );
    assert_eq!(
        git(&checkout, &["remote", "get-url", "origin"]),
        url,
        "origin is repointed at the real URL once the local clone is made"
    );
    assert_eq!(
        alternates_of(&checkout),
        None,
        "a job copies what it takes, so nothing it produces depends on the entry"
    );
    assert!(
        !env.playground.join(".git/gitscale/repos").exists(),
        "CI keeps no store"
    );
}

/// Outside CI, `sync` never touches the cache.
#[test]
fn normal_002_a_developer_sync_touches_no_cache() {
    let env = TestEnv::new("cache_dev_untouched");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare.display().to_string()));
    let out = env.run_with_env(&[], &["sync"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(!env.cache.exists(), "{}", env.cache.display());
}

/// `--no-cache` in CI talks to the remote: a depth-1 fetch of the one commit.
#[test]
fn normal_003_no_cache_in_ci_fetches_the_commit_from_the_remote() {
    let env = TestEnv::new("cache_ci_off");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    let out = env.run_with_env(&[("CI", "1")], &["sync", "--no-cache"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(!env.cache.exists());
    let checkout = env.playground.join("libs/core");
    assert_eq!(
        git(&checkout, &["rev-parse", "HEAD"]),
        git(&bare, &["rev-parse", "main"])
    );
    assert_eq!(
        git(&checkout, &["rev-parse", "--is-shallow-repository"]),
        "true"
    );
}

#[test]
fn normal_004_a_second_job_re_pins_when_the_head_has_moved() {
    let env = TestEnv::new("cache_ci_moved");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "v1")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    assert!(ci_pull(&env).success);

    let moved = commit_to_bare(&bare, "main", "a.txt", "v2");
    assert!(ci_pull(&env).success);

    let entry = env.cache_entry("snapshots", &url);
    assert_eq!(
        git(&entry, &["rev-parse", &format!("refs/heads/pin/{}", moved)]),
        moved,
        "the new head should be pinned alongside the old one"
    );
    let checkout = env.playground.join("libs/core");
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), moved);
    assert_eq!(
        std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "v2"
    );
}

/// `cache update` works anywhere, `CI` set or not: it adds the pin a job of
/// this workspace would take.
#[test]
fn normal_005_update_warms_a_repo_nobody_has_placed() {
    let env = TestEnv::new("cache_update");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    let head = git(&bare, &["rev-parse", "main"]);

    let out = cache_cmd(&env, &["update"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(
        out.stdout.contains("libs/core (pinned at "),
        "{}",
        out.stdout
    );
    assert_eq!(
        git(
            &env.cache_entry("snapshots", &url),
            &["rev-parse", &format!("refs/heads/pin/{}", head)]
        ),
        head
    );
    assert!(
        !env.playground.join("libs/core").exists(),
        "warming the cache should check nothing out"
    );
}

/// `cache update` works anywhere, CI or not: it adds what a job would take,
/// so a runner image can be warmed ahead of time.
#[test]
fn normal_006_update_warms_images_without_a_checkout() {
    let env = TestEnv::new("art_cache_update");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));

    let out = env.run_with_env(&[], &["cache", "update"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!env.playground.join("meta/app").exists());
    cached_images(&env);

    env.registry().clear_log();
    support::artefacts::ci_pull(&env);
    assert_eq!(blob_downloads(&env), 0);
}

/// `cache update` warms the checkouts resolution settles on, the implicit
/// ones included.
#[test]
fn normal_007_update_covers_implicit_dependencies() {
    let env = TestEnv::new("res_cache_update");
    let (_, d) = implicit_d(&env);
    let out = env.run_with_env(&[], &["cache", "update"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("imports/d"), "{}", out.stdout);
    let snapshot = env.cache_entry("snapshots", d.to_str().unwrap());
    assert!(snapshot.join("HEAD").is_file(), "{}", snapshot.display());
}

#[test]
fn normal_008_compact_evicts_what_nothing_has_used() {
    let env = TestEnv::new("cache_compact");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    assert!(ci_pull(&env).success);
    assert_eq!(env.cache_entries("snapshots").len(), 1);

    let entry = env.cache_entry("snapshots", &url);
    age(&entry.join("gitscale-last-used"), "2 hours ago");

    let out = cache_cmd(&env, &["compact", "--keep-recent", "1h"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(out.stdout.contains("1 entry evicted"), "{}", out.stdout);
    assert!(env.cache_entries("snapshots").is_empty());
}

#[test]
fn normal_009_compact_drops_stale_pins_from_an_entry_it_keeps() {
    let env = TestEnv::new("cache_compact_pins");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "v1")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    assert!(ci_pull(&env).success);
    let first = git(&bare, &["rev-parse", "main"]);

    // The entry is still in use; the pin inside it is what has gone stale, so
    // only the pin should go.
    let entry = env.cache_entry("snapshots", &url);
    age(&entry.join("gitscale-pins").join(&first), "2 hours ago");

    let out = cache_cmd(&env, &["compact", "--keep-recent", "1h"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(out.stdout.contains("1 pin dropped"), "{}", out.stdout);
    assert!(entry.is_dir(), "the entry itself should have been kept");
    assert!(
        !git_ok(
            &entry,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/pin/{}", first)
            ]
        ),
        "the stale pin should be gone"
    );
}

#[test]
fn normal_010_compact_drops_cold_images_and_their_blobs() {
    let env = TestEnv::new("art_cache_compact");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    let old = "v1.0.0".to_string();
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    support::artefacts::ci_pull(&env);
    env.push_commit(&bare, "main", "README.md", "v2");
    layered(&env, &bare, "v1.1.0", "app v2");
    let new = "v1.1.0".to_string();
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    support::artefacts::ci_pull(&env);

    let entry = cached_images(&env);
    let blobs = || {
        std::fs::read_dir(entry.join("blobs/sha256"))
            .unwrap()
            .count()
    };
    let before = blobs();
    support::artefacts::age(&entry, &old);

    let out = env.run_with_env(&[], &["cache", "compact", "--keep-recent", "30d"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("1 pin dropped"), "{}", out.stdout);
    let index = std::fs::read_to_string(entry.join("index.json")).unwrap();
    assert!(!index.contains(&old) && index.contains(&new), "{}", index);
    // The old manifest and its app layer go; the shared vendor layer stays.
    assert_eq!(blobs(), before - 2);

    std::fs::remove_dir_all(env.playground.join("meta")).unwrap();
    env.registry().clear_log();
    support::artefacts::ci_pull(&env);
    assert_eq!(blob_downloads(&env), 0);
}

#[test]
fn normal_011_status_names_entries_after_the_repos_that_declare_them() {
    let env = TestEnv::new("cache_status_cmd");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&format!("file://{}", bare.display())));

    let empty = cache_cmd(&env, &["status"]);
    assert!(empty.success, "{:?}", empty.stderr);
    assert!(
        empty.stdout.contains("Nothing cached yet"),
        "{}",
        empty.stdout
    );

    assert!(ci_pull(&env).success);
    let out = cache_cmd(&env, &["status"]);
    assert!(out.success, "{:?}", out.stderr);
    let row = out
        .stdout
        .lines()
        .find(|l| l.contains("libs/core"))
        .expect("the entry should be named after the repo that declares it");
    assert!(
        row.contains("KiB") || row.contains(" B"),
        "the row should carry what the snapshot costs: {}",
        row
    );
}

#[test]
fn normal_012_status_lists_every_revision_a_snapshot_holds() {
    let env = TestEnv::new("cache_status_pins");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "v1")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    assert!(ci_pull(&env).success);
    let first = git(&bare, &["rev-parse", "main"]);

    // A second job on a moved head: the entry now holds two commits, and which
    // of them is still wanted is what `compact` decides on.
    let second = commit_to_bare(&bare, "main", "a.txt", "v2");
    assert!(ci_pull(&env).success);

    let out = cache_cmd(&env, &["status"]);
    assert!(out.success, "{:?}", out.stderr);
    let row = out
        .stdout
        .lines()
        .find(|l| l.contains("libs/core"))
        .expect("a row for the repo");
    let fields: Vec<&str> = row.split_whitespace().collect();
    // REPO, SNAPSHOTS (two words), IMAGES, TOTAL (two words), REVS…
    assert_eq!(
        fields[3], "-",
        "no image entry for a git repository: {}",
        row
    );
    assert_eq!(fields[6], "2", "two pins: {}", row);
    for sha in [&first, &second] {
        let short = gitscale::git::short_sha(sha);
        assert!(
            out.stdout.contains(short),
            "every cached revision should be listed, missing {}: {}",
            short,
            out.stdout
        );
    }
}

#[test]
fn normal_013_status_has_an_images_column() {
    let env = TestEnv::new("art_cache_status");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    support::artefacts::ci_pull(&env);
    assert!(!env.playground.join(".git/gitscale/images").exists());

    let out = env.run_with_env(&[], &["cache", "status"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("IMAGES"), "{}", out.stdout);
    let row = out.stdout.lines().find(|l| l.contains("meta/app")).unwrap();
    let fields: Vec<&str> = row.split_whitespace().collect();
    // REPO, SNAPSHOTS, IMAGES (two words), …: no snapshot, as nothing of
    // its git is read, and the image.
    assert_eq!(fields[1], "-", "{}", row);
    assert_ne!(fields[2], "-", "{}", row);
    // The release is listed under the row.
    assert!(out.stdout.contains("      v1.0.0"), "{}", out.stdout);
}

/// An image entry nothing has used within the period goes whole, blobs and
/// all, and `compact` counts it as an evicted entry.
#[test]
fn normal_021_compact_evicts_a_cold_image_entry_whole() {
    let env = TestEnv::new("cache_compact_image_entry");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    support::artefacts::ci_pull(&env);
    let entry = cached_images(&env);
    age(&entry.join("gitscale-last-used"), "2000-01-01");

    let out = cache_cmd(&env, &["compact", "--keep-recent", "30d"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("1 entry evicted"), "{}", out.stdout);
    assert!(!entry.exists());
    assert!(env.cache_entries("images").is_empty());
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// An annotated tag is an object of its own: what CI pins, and checks out,
/// must be the commit it points at, not the tag object.
#[test]
fn edge_014_ci_pins_an_annotated_tag_at_its_commit() {
    let env = TestEnv::new("cache_ci_annotated");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "tagged")]);
    git(
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
            "v1",
            "main",
        ],
    );
    let tagged = git(&bare, &["rev-parse", "v1^{commit}"]);
    // Main moves on, so the tag and the branch head differ.
    commit_to_bare(&bare, "main", "a.txt", "later");
    let url = format!("file://{}", bare.display());
    env.write_config(&format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"v1\" }}\n",
        url
    ));

    let out = ci_pull(&env);
    assert!(out.success, "{:?}", out.stderr);
    let checkout = env.playground.join("libs/core");
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), tagged);
    assert_eq!(
        std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "tagged"
    );
    let entry = env.cache_entry("snapshots", &url);
    assert_eq!(
        git(
            &entry,
            &["for-each-ref", "--format=%(refname)", "refs/heads/pin/"]
        ),
        format!("refs/heads/pin/{}", tagged),
        "the pin should name the commit, not the tag object"
    );
}

/// `git ls-remote <url> main` lists every ref whose name ends in `main`. The
/// branch CI pins is the one called exactly that.
#[test]
fn edge_015_ci_pins_the_branch_named_and_not_one_ending_in_the_name() {
    let env = TestEnv::new("cache_ci_exact_branch");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "main")]);
    let main = git(&bare, &["rev-parse", "main"]);
    git(&bare, &["branch", "zzz/main", "main"]);
    commit_to_bare(&bare, "zzz/main", "a.txt", "not main");
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));

    let out = ci_pull(&env);
    assert!(out.success, "{:?}", out.stderr);
    let checkout = env.playground.join("libs/core");
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), main);
    assert_eq!(
        std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "main"
    );
}

#[test]
fn edge_016_a_job_is_unaffected_by_the_entry_being_deleted() {
    let env = TestEnv::new("cache_ci_evicted");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    assert!(ci_pull(&env).success);

    std::fs::remove_dir_all(&env.cache).unwrap();

    let checkout = env.playground.join("libs/core");
    assert!(
        git_ok(&checkout, &["fsck", "--no-progress"]),
        "the checkout should still be whole"
    );
    assert!(!git(&checkout, &["log", "--oneline"]).is_empty());
    assert_eq!(
        std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "a"
    );
}

#[test]
fn edge_017_a_sha_pinned_entry_needs_no_ref_advertisement() {
    let env = TestEnv::new("cache_ci_sha");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = format!("file://{}", bare.display());
    let sha = git(&bare, &["rev-parse", "main"]);
    env.write_config(&format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"{}\" }}\n",
        url, sha
    ));

    assert!(ci_pull(&env).success);

    let entry = env.cache_entry("snapshots", &url);
    assert_eq!(
        git(&entry, &["rev-parse", &format!("refs/heads/pin/{}", sha)]),
        sha
    );
    let checkout = env.playground.join("libs/core");
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), sha);
}

#[test]
fn edge_018_compact_works_from_outside_any_workspace() {
    let env = TestEnv::new("cache_compact_bare");
    // No config at all: the cache belongs to the user, not to a workspace.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(["cache", "compact"])
        .current_dir(&env.repos_remote)
        .env("GITSCALE_CACHE_DIR", &env.cache)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Compacted"));
}

/// What resolution read into the cache seconds ago — an artefact's manifest,
/// read by a CI `fetch` for its config — is recently used: `compact` with a
/// period of a month keeps it.
#[test]
#[ignore = "bug: manifests resolution reads are recorded without a use marker, so compact drops them at once"]
fn edge_022_compact_keeps_the_images_resolution_just_read() {
    let env = TestEnv::new("cache_compact_config_layer");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    let commit = tip(&bare, "main");
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run_with_env(&[("CI", "true")], &["fetch"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let entry = cached_images(&env);
    assert!(
        std::fs::read_to_string(entry.join("index.json"))
            .unwrap()
            .contains(&commit),
        "the fetch read nothing into the cache"
    );

    let out = cache_cmd(&env, &["compact", "--keep-recent", "30d"]);
    assert!(out.success, "{}", out.stderr);
    assert!(entry.is_dir(), "{}", out.stdout);
    assert!(std::fs::read_to_string(entry.join("index.json"))
        .unwrap()
        .contains(&commit));
}

/// A download a killed job left half-written is garbage: `compact` removes
/// it from an entry it keeps, and leaves every blob a held image needs.
#[test]
fn edge_023_compact_removes_half_written_downloads() {
    let env = TestEnv::new("cache_compact_partial");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    support::artefacts::ci_pull(&env);
    let entry = cached_images(&env);
    let blobs = entry.join("blobs/sha256");
    let held = std::fs::read_dir(&blobs).unwrap().count();
    let partial = blobs.join(format!(".{}.partial-99999", "d".repeat(64)));
    std::fs::write(&partial, "half").unwrap();

    let out = cache_cmd(&env, &["compact", "--keep-recent", "30d"]);
    assert!(out.success, "{}", out.stderr);
    assert!(!partial.exists());
    assert_eq!(std::fs::read_dir(&blobs).unwrap().count(), held);

    std::fs::remove_dir_all(env.playground.join("meta")).unwrap();
    env.registry().clear_log();
    support::artefacts::ci_pull(&env);
    assert_eq!(blob_downloads(&env), 0);
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_019_an_unknown_period_is_refused_before_anything_is_deleted() {
    let env = TestEnv::new("cache_compact_period");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&format!("file://{}", bare.display())));
    assert!(ci_pull(&env).success);

    // `6m` reads as six minutes to humantime and as six months to a person.
    for period in ["soon", "6m"] {
        let out = cache_cmd(&env, &["compact", "--keep-recent", period]);
        assert!(!out.success, "{}", period);
        assert_eq!(
            env.cache_entries("snapshots").len(),
            1,
            "a period it could not read must not evict anything: {}",
            period
        );
    }
}

/// `cache update` for an artefact whose release has no image fails, and
/// says so, rather than warming something else.
#[test]
fn error_024_update_fails_for_an_unpublished_artefact_and_says_why() {
    let env = TestEnv::new("cache_update_unpublished");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    support::run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = cache_cmd(&env, &["update"]);
    assert!(!out.success, "{}", out.stdout);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(text.contains("no artefact for"), "{}", text);
    assert!(env.cache_entries("images").is_empty());
}

/// `cache status` does not need a workspace, but one whose config does not
/// parse is an error, not a reason to guess at names.
#[test]
fn error_025_status_refuses_a_workspace_config_that_does_not_parse() {
    let env = TestEnv::new("cache_status_bad_config");
    env.write_config("[repos\nbroken = ");
    let out = cache_cmd(&env, &["status"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains(".gitscale.toml"), "{}", out.stderr);
}

/// With no `GITSCALE_CACHE_DIR`, `XDG_DATA_HOME` or `HOME` there is nowhere
/// to keep a cache: the cache commands say which variable to set.
#[test]
fn error_026_without_a_cache_location_the_commands_say_how_to_set_one() {
    let env = TestEnv::new("cache_no_location");
    for command in ["status", "compact"] {
        let out = env.run_with_env(
            &[
                ("GITSCALE_CACHE_DIR", ""),
                ("XDG_DATA_HOME", ""),
                ("HOME", ""),
            ],
            &["cache", command],
        );
        assert!(!out.success, "{}: {}", command, out.stdout);
        assert!(
            out.stderr
                .contains("no cache location: set GITSCALE_CACHE_DIR, XDG_DATA_HOME or HOME"),
            "{}: {}",
            command,
            out.stderr
        );
    }
}
