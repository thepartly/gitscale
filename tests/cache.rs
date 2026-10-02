// The CI cache: exact commits CI jobs asked for, kept in the user's own data
// directory so the next job on the runner downloads nothing.
//
// A developer machine keeps no cache — its stores live in the root's own git
// directory, see tests/worktrees.rs — so every test here that pulls does it as
// a CI job, and asserts on two things at once: that the cache holds the commit,
// and that the checkout was built from it.
#[allow(dead_code)]
mod helpers;

use helpers::TestEnv;
use std::path::Path;

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {:?} in {} failed:\n{}{}",
        args,
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_ok(cwd: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn alternates_of(repo: &Path) -> Option<String> {
    std::fs::read_to_string(repo.join(".git/objects/info/alternates"))
        .ok()
        .map(|s| s.trim().to_string())
}

/// Add a commit to a bare repo through a throwaway clone, and return its SHA.
fn commit_to_bare(bare: &Path, branch: &str, file: &str, content: &str) -> String {
    let tmp = bare.with_extension("edit");
    let _ = std::fs::remove_dir_all(&tmp);
    git(
        bare.parent().unwrap(),
        &[
            "clone",
            "--quiet",
            bare.to_str().unwrap(),
            tmp.to_str().unwrap(),
        ],
    );
    git(&tmp, &["config", "user.email", "t@t"]);
    git(&tmp, &["config", "user.name", "T"]);
    git(&tmp, &["checkout", "--quiet", branch]);
    std::fs::write(tmp.join(file), content).unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "--quiet", "-m", "another"]);
    git(&tmp, &["push", "--quiet", "origin", branch]);
    let sha = git(&tmp, &["rev-parse", "HEAD"]);
    let _ = std::fs::remove_dir_all(&tmp);
    sha
}

/// Backdate a file, so eviction can be tested without waiting for a month to
/// pass.
fn age(path: &Path, when: &str) {
    let ok = std::process::Command::new("touch")
        .args(["-d", when])
        .arg(path)
        .status()
        .expect("failed to run touch")
        .success();
    assert!(ok, "could not backdate {}", path.display());
}

fn config_for(url: &str) -> String {
    format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        url
    )
}

/// A pull as a CI job would run it, against this test's own cache.
fn ci_pull(env: &TestEnv) -> helpers::CliOutput {
    env.run_with_env(&[("CI", "1")], &["pull"])
}

/// A `gitscale cache …` command against this test's own cache, CI or not.
fn cache_cmd(env: &TestEnv, args: &[&str]) -> helpers::CliOutput {
    let mut full = vec!["cache"];
    full.extend_from_slice(args);
    env.run_with_env(&[], &full)
}

// ---------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------

#[test]
fn ci_takes_a_pinned_commit_out_of_a_snapshot_entry() {
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

/// Outside CI, `pull` never touches the cache.
#[test]
fn a_developer_pull_touches_no_cache() {
    let env = TestEnv::new("cache_dev_untouched");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare.display().to_string()));
    let out = env.run_with_env(&[], &["pull"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(!env.cache.exists(), "{}", env.cache.display());
}

/// `--no-cache` in CI talks to the remote: a depth-1 fetch of the one commit.
#[test]
fn no_cache_in_ci_fetches_the_commit_from_the_remote() {
    let env = TestEnv::new("cache_ci_off");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    let out = env.run_with_env(&[("CI", "1")], &["pull", "--no-cache"]);
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

/// An annotated tag is an object of its own: what CI pins, and checks out,
/// must be the commit it points at, not the tag object.
#[test]
fn ci_pins_an_annotated_tag_at_its_commit() {
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
fn ci_pins_the_branch_named_and_not_one_ending_in_the_name() {
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
fn a_job_is_unaffected_by_the_entry_being_deleted() {
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
fn a_second_job_re_pins_when_the_head_has_moved() {
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

#[test]
fn a_sha_pinned_entry_needs_no_ref_advertisement() {
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

// ---------------------------------------------------------------------------
// The commands that own the cache
// ---------------------------------------------------------------------------

/// `cache update` works anywhere, `CI` set or not: it adds the pin a job of
/// this workspace would take.
#[test]
fn cache_update_warms_a_repo_nobody_has_pulled() {
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

#[test]
fn cache_compact_evicts_what_nothing_has_used() {
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
fn cache_compact_drops_stale_pins_from_an_entry_it_keeps() {
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
fn compact_works_from_outside_any_workspace() {
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

#[test]
fn an_unknown_period_is_refused_before_anything_is_deleted() {
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

#[test]
fn cache_status_names_entries_after_the_repos_that_declare_them() {
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
fn cache_status_lists_every_revision_a_snapshot_holds() {
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
