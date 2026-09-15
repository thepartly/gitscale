// The object cache: one bare entry per remote URL, in the user's own data
// directory, standing between every workspace on the machine and the network.
//
// The rule these tests hold to is the one the design turns on: the cache talks
// to the remote, the workspace talks to the cache. So most of them assert on
// two things at once — that the entry was brought up to date, and that the
// checkout was built from it rather than from another trip to the remote.
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

/// Run a `gitscale cache <action>`: `TestEnv::run` inserts `-C` after the
/// first word, which a two-word subcommand cannot take.
fn cache_cmd(env: &TestEnv, args: &[&str]) -> helpers::CliOutput {
    let mut full: Vec<String> = vec!["gitscale".to_string(), "cache".to_string()];
    full.extend(args.iter().map(|a| a.to_string()));
    full.push("-C".to_string());
    full.push(env.playground.to_str().unwrap().to_string());
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    gitscale::run_cli_with(&refs, false)
}

/// Point a fixture's HEAD at the branch its content is on.
///
/// `create_bare_repo` inits with git's default branch name and pushes to
/// another, which no real remote looks like — and a clone of it checks nothing
/// out. Only the tests that clone a repository without naming a revision care.
fn set_head(bare: &Path, branch: &str) {
    git(
        bare,
        &["symbolic-ref", "HEAD", &format!("refs/heads/{}", branch)],
    );
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

/// The STATUS flags of a `gitscale status` row, by repo directory. Colour is
/// stripped first: the icon is wrapped in escapes, and the reset sequence
/// would otherwise split off as a field of its own.
fn status_flags(out: &helpers::CliOutput, repo: &str) -> String {
    let plain = helpers::strip_ansi(&out.stdout);
    let row = plain
        .lines()
        .find(|l| l.contains(repo))
        .unwrap_or_else(|| panic!("no status row for {}: {}", repo, plain));
    // Icon, REPO, PATH, MODE, REF, EXPECTED, then STATUS — the last column and
    // the only one that can hold spaces. Every repo here declares a revision,
    // so EXPECTED is never blank.
    row.split_whitespace().skip(6).collect::<Vec<_>>().join(" ")
}

/// What `gitscale status --format json` says the cache is doing for a repo.
fn cache_state(env: &TestEnv, repo: &str) -> String {
    let out = env.run(&["status", "--format", "json"]);
    assert!(out.success, "{:?}", out.stderr);
    let rows: serde_json::Value = serde_json::from_str(&out.stdout).expect("valid JSON");
    rows.as_array()
        .expect("an array")
        .iter()
        .find(|row| row["directory"] == repo)
        .unwrap_or_else(|| panic!("no row for {}: {}", repo, out.stdout))["cache"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn config_for(url: &str) -> String {
    format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        url
    )
}

// ---------------------------------------------------------------------------
// Mirrors — the developer's side
// ---------------------------------------------------------------------------

#[test]
fn a_clone_lands_in_the_cache_and_borrows_from_it() {
    let env = TestEnv::new("cache_clone");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = bare.display().to_string();
    env.write_config(&config_for(&url));

    let out = env.run(&["clone"]);
    assert!(out.success, "{:?}", out.stderr);

    let entry = env.cache_entry("mirror", &url);
    assert_eq!(
        git(&entry, &["rev-parse", "refs/heads/main"]),
        git(&bare, &["rev-parse", "main"]),
        "the entry should hold what the remote holds"
    );

    let checkout = env.playground.join("libs/core");
    let alternates = alternates_of(&checkout).expect("the clone should borrow from the cache");
    assert_eq!(
        Path::new(&alternates).canonicalize().unwrap(),
        entry.join("objects").canonicalize().unwrap(),
        "it should borrow from the entry, not from anywhere else"
    );
    assert_eq!(
        git(&checkout, &["remote", "get-url", "origin"]),
        url,
        "origin must stay the real URL, so pushes go where they always did"
    );
}

#[test]
fn a_second_workspace_shares_the_one_entry() {
    let env = TestEnv::new("cache_second_ws");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = bare.display().to_string();
    env.write_config(&config_for(&url));
    assert!(env.run(&["clone"]).success);

    // A second workspace of the same project, with no relationship to the
    // first beyond the machine they share.
    let other = env.playground.with_extension("two");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join(".gitscale.toml"),
        format!(
            "[cache]\ndir = \"{}\"\n\n{}",
            env.cache.display(),
            config_for(&url)
        ),
    )
    .unwrap();
    let out = gitscale::run_cli_with(&["gitscale", "clone", "-C", other.to_str().unwrap()], false);
    assert!(out.success, "{:?}", out.stderr);

    assert_eq!(
        env.cache_entries("mirror").len(),
        1,
        "both workspaces should read one entry, not one each"
    );
    assert!(other.join("libs/core/a.txt").is_file());
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn a_pull_updates_the_entry_before_the_workspace() {
    let env = TestEnv::new("cache_pull");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "v1")]);
    let url = bare.display().to_string();
    env.write_config(&config_for(&url));
    assert!(env.run(&["clone"]).success);

    let moved = commit_to_bare(&bare, "main", "a.txt", "v2");

    let out = env.run(&["pull"]);
    assert!(out.success, "{:?}", out.stderr);

    let entry = env.cache_entry("mirror", &url);
    assert_eq!(
        git(&entry, &["rev-parse", "refs/heads/main"]),
        moved,
        "the entry must be brought up to date first — that is the one network call"
    );
    let checkout = env.playground.join("libs/core");
    assert_eq!(
        git(&checkout, &["rev-parse", "HEAD"]),
        moved,
        "and the workspace updated from it"
    );
    assert_eq!(
        std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "v2"
    );
}

#[test]
fn a_fetch_updates_the_entry_without_touching_the_tree() {
    let env = TestEnv::new("cache_fetch");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "v1")]);
    let url = bare.display().to_string();
    env.write_config(&config_for(&url));
    assert!(env.run(&["clone"]).success);

    let moved = commit_to_bare(&bare, "main", "a.txt", "v2");
    assert!(env.run(&["fetch"]).success);

    let entry = env.cache_entry("mirror", &url);
    assert_eq!(git(&entry, &["rev-parse", "refs/heads/main"]), moved);
    let checkout = env.playground.join("libs/core");
    assert_eq!(
        std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "v1",
        "a fetch updates refs, never the working tree"
    );
    assert_eq!(
        git(&checkout, &["rev-parse", "refs/remotes/origin/main"]),
        moved,
        "the tracking ref should have advanced, from the cache"
    );
}

#[test]
fn no_cache_leaves_the_cache_untouched() {
    let env = TestEnv::new("cache_off");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare.display().to_string()));

    assert!(env.run(&["clone", "--no-cache"]).success);
    assert!(
        env.cache_entries("mirror").is_empty(),
        "--no-cache must not create entries"
    );
    assert_eq!(
        alternates_of(&env.playground.join("libs/core")),
        None,
        "and the clone should own its objects"
    );
}

#[test]
fn the_config_can_turn_it_off_too() {
    let env = TestEnv::new("cache_disabled");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[cache]\nenabled = false\ndir = \"{}\"\n\n{}",
        env.cache.display(),
        config_for(&bare.display().to_string())
    ));

    assert!(env.run(&["clone"]).success);
    assert!(env.cache_entries("mirror").is_empty());
}

#[test]
fn an_artefact_never_gets_an_entry() {
    let env = TestEnv::new("cache_artefact");
    let repo_url = "https://github.com/org/app.git";
    env.create_artefact(repo_url, "main", &[("app.bin", "binary")]);
    env.write_config(&format!(
        "[storage]\nurl = \"{}\"\n\n[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"main\", mode = \"artefact\" }}\n",
        env.storage_url(),
        repo_url
    ));

    assert!(env.run(&["clone"]).success);
    assert!(
        env.cache_entries("mirror").is_empty() && env.cache_entries("snapshots").is_empty(),
        "an unpacked archive has no object store to cache"
    );
}

#[test]
fn a_source_workspace_still_wins_over_the_cache() {
    let env = TestEnv::new("cache_source_ws");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = bare.display().to_string();
    env.write_config(&config_for(&url));
    env.init_playground_git();
    assert!(env.run(&["clone"]).success);

    // A worktree of the workspace: its clone should borrow from the main
    // worktree's copy, while the entry is still kept current for whoever falls
    // back to it later.
    let wt = env.playground.with_extension("wt");
    let _ = std::fs::remove_dir_all(&wt);
    git(
        &env.playground,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat",
            wt.to_str().unwrap(),
        ],
    );
    let out = gitscale::run_cli_with(&["gitscale", "clone", "-C", wt.to_str().unwrap()], false);
    assert!(out.success, "{:?}", out.stderr);

    let alternates = alternates_of(&wt.join("libs/core")).expect("it should borrow");
    assert_eq!(
        Path::new(&alternates).canonicalize().unwrap(),
        env.playground
            .join("libs/core/.git/objects")
            .canonicalize()
            .unwrap(),
        "the source workspace has this repo, so it is what gets borrowed from"
    );
    assert!(
        env.cache_entry("mirror", &url).is_dir(),
        "the entry is still updated, for the workspace that has to fall back to it"
    );
    let _ = std::fs::remove_dir_all(&wt);
    git(&env.playground, &["worktree", "prune"]);
}

// ---------------------------------------------------------------------------
// Snapshots — the CI side
// ---------------------------------------------------------------------------

#[test]
fn ci_takes_a_pinned_commit_out_of_a_snapshot_entry() {
    let env = TestEnv::new("cache_ci_pin");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    // file:// so git honours --depth; it ignores depth for a plain local path.
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    let head = git(&bare, &["rev-parse", "main"]);

    let out = env.run_with_env(&[("CI", "1")], &["clone"]);
    assert!(out.success, "{:?}", out.stderr);

    let entry = env.cache_entry("snapshots", &url);
    assert_eq!(
        git(&entry, &["rev-parse", &format!("refs/heads/pin/{}", head)]),
        head,
        "the pinned commit should be a ref inside the entry"
    );
    assert!(
        env.cache_entries("mirror").is_empty(),
        "CI takes snapshots, not mirrors"
    );

    let checkout = env.playground.join("libs/core");
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&checkout, &["rev-parse", "--is-shallow-repository"]),
        "true",
        "a snapshot entry is shallow by design, and so is what comes out of it"
    );
    assert_eq!(
        git(&checkout, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main",
        "a branch revision should land on that branch, as a --branch clone would"
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
}

#[test]
fn a_job_is_unaffected_by_the_entry_being_deleted() {
    let env = TestEnv::new("cache_ci_evicted");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    assert!(env.run_with_env(&[("CI", "1")], &["clone"]).success);

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
    assert!(env.run_with_env(&[("CI", "1")], &["clone"]).success);

    let moved = commit_to_bare(&bare, "main", "a.txt", "v2");
    assert!(env.run_with_env(&[("CI", "1")], &["pull"]).success);

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

    assert!(env.run_with_env(&[("CI", "1")], &["clone"]).success);

    let entry = env.cache_entry("snapshots", &url);
    assert_eq!(
        git(&entry, &["rev-parse", &format!("refs/heads/pin/{}", sha)]),
        sha
    );
    let checkout = env.playground.join("libs/core");
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), sha);
    assert!(
        !git_ok(&checkout, &["symbolic-ref", "HEAD"]),
        "a commit revision leaves HEAD detached, as it always has"
    );
}

// ---------------------------------------------------------------------------
// The commands that own the cache
// ---------------------------------------------------------------------------

#[test]
fn cache_update_warms_a_repo_nobody_has_pulled() {
    let env = TestEnv::new("cache_update");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = bare.display().to_string();
    env.write_config(&config_for(&url));

    let out = cache_cmd(&env, &["update"]);
    assert!(out.success, "{:?}", out.stderr);
    assert_eq!(
        git(
            &env.cache_entry("mirror", &url),
            &["rev-parse", "refs/heads/main"]
        ),
        git(&bare, &["rev-parse", "main"])
    );
    assert!(
        !env.playground.join("libs/core").exists(),
        "warming the cache should check nothing out"
    );
}

#[test]
fn cache_repair_brings_back_a_deleted_entry() {
    let env = TestEnv::new("cache_repair");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = bare.display().to_string();
    env.write_config(&config_for(&url));
    assert!(env.run(&["clone"]).success);

    // What a deleted entry does to its borrowers: nothing, until something
    // tries to read an object.
    let entry = env.cache_entry("mirror", &url);
    std::fs::remove_dir_all(&entry).unwrap();

    let out = cache_cmd(&env, &["repair"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(out.stdout.contains("repair"), "{}", out.stdout);
    assert!(entry.is_dir(), "the entry should be back");
    assert!(
        git_ok(&env.playground.join("libs/core"), &["log", "--oneline"]),
        "and the workspace able to read its history again"
    );
}

#[test]
fn cache_repair_leaves_a_healthy_entry_alone() {
    let env = TestEnv::new("cache_repair_ok");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare.display().to_string()));
    assert!(env.run(&["clone"]).success);

    let out = cache_cmd(&env, &["repair"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(out.stdout.contains("Repaired 0 entries"), "{}", out.stdout);
}

#[test]
fn cache_compact_evicts_what_nothing_has_used() {
    let env = TestEnv::new("cache_compact");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare.display().to_string()));
    assert!(env.run(&["clone"]).success);
    assert_eq!(env.cache_entries("mirror").len(), 1);

    let entry = env.cache_entry("mirror", &bare.display().to_string());
    age(&entry.join("gitscale-last-used"), "2 hours ago");

    let out = cache_cmd(&env, &["compact", "--keep-recent", "1h"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(out.stdout.contains("1 entry evicted"), "{}", out.stdout);
    assert!(env.cache_entries("mirror").is_empty());
}

#[test]
fn cache_compact_drops_stale_pins_from_an_entry_it_keeps() {
    let env = TestEnv::new("cache_compact_pins");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "v1")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    assert!(env.run_with_env(&[("CI", "1")], &["clone"]).success);
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
    let out = gitscale::run_cli_with(
        &[
            "gitscale",
            "cache",
            "compact",
            "-C",
            env.playground.to_str().unwrap(),
        ],
        false,
    );
    assert!(out.success, "{:?}", out.stderr);
    assert!(out.stdout.contains("Compacted"), "{}", out.stdout);
}

#[test]
fn an_unknown_period_is_refused_before_anything_is_deleted() {
    let env = TestEnv::new("cache_compact_period");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare.display().to_string()));
    assert!(env.run(&["clone"]).success);

    let out = cache_cmd(&env, &["compact", "--keep-recent", "soon"]);
    assert!(!out.success);
    assert_eq!(
        env.cache_entries("mirror").len(),
        1,
        "a period it could not read must not evict anything"
    );
}

// ---------------------------------------------------------------------------
// Bootstrapping and adoption
// ---------------------------------------------------------------------------

#[test]
fn clone_from_a_url_bootstraps_a_whole_workspace() {
    let env = TestEnv::new("cache_bootstrap");
    let core = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let root = env.create_bare_repo(
        "root",
        "main",
        &[(
            ".gitscale.toml",
            &format!(
                "[cache]\ndir = \"{}\"\n\n{}",
                env.cache.display(),
                config_for(&core.display().to_string())
            ),
        )],
    );

    set_head(&root, "main");

    // Bootstrapping happens before there is a config to read, so the cache is
    // whatever the environment says — which is also how CI points a runner at
    // a cache that outlives the job.
    let dest = env.playground.join("ws");
    let out = env.run_with_env(
        &[("GITSCALE_CACHE_DIR", env.cache.to_str().unwrap())],
        &["clone", root.to_str().unwrap(), "ws"],
    );
    assert!(out.success, "{:?}", out.stderr);

    assert!(
        dest.join(".gitscale.toml").is_file(),
        "the root should be cloned"
    );
    assert!(
        dest.join("libs/core/a.txt").is_file(),
        "and everything it declares cloned too"
    );
    assert!(
        alternates_of(&dest).is_some(),
        "the root repository goes through the cache like any other"
    );
    assert_eq!(
        env.cache_entries("mirror").len(),
        2,
        "one entry for the root, one for the repo it declares"
    );
}

#[test]
fn a_url_clone_names_its_directory_after_the_repository() {
    let env = TestEnv::new("cache_bootstrap_default_dir");
    let root = env.create_bare_repo("root", "main", &[("README.md", "hi")]);
    set_head(&root, "main");

    let out = env.run_with_env(
        &[("GITSCALE_CACHE_DIR", env.cache.to_str().unwrap())],
        &["clone", root.to_str().unwrap()],
    );
    assert!(out.success, "{:?}", out.stderr);
    assert!(
        env.playground.join("root/README.md").is_file(),
        "it should clone into ./root, the way git would"
    );
    assert!(
        out.stdout.contains("nothing further to do"),
        "a repository with no config is just a clone: {}",
        out.stdout
    );
}

#[test]
fn adopting_relinks_a_root_that_was_cloned_by_hand() {
    let env = TestEnv::new("cache_adopt");
    let core = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let root = env.create_bare_repo(
        "root",
        "main",
        &[(
            ".gitscale.toml",
            &format!(
                "[cache]\ndir = \"{}\"\nadopt_root = true\n\n{}",
                env.cache.display(),
                config_for(&core.display().to_string())
            ),
        )],
    );

    set_head(&root, "main");

    // Cloned by plain git, so it holds its own full copy and shares nothing.
    let ws = env.playground.join("ws");
    git(
        &env.playground,
        &[
            "clone",
            "--quiet",
            root.to_str().unwrap(),
            ws.to_str().unwrap(),
        ],
    );
    assert_eq!(alternates_of(&ws), None);

    let out = gitscale::run_cli_with(&["gitscale", "pull", "-C", ws.to_str().unwrap()], false);
    assert!(out.success, "{:?}", out.stderr);

    let alternates = alternates_of(&ws).expect("the root should now borrow from the cache");
    assert!(
        Path::new(&alternates).starts_with(env.cache.canonicalize().unwrap()),
        "it should point into the cache: {}",
        alternates
    );
    assert!(
        git_ok(&ws, &["fsck", "--no-progress"]),
        "and the repository still be whole"
    );
    assert!(!git(&ws, &["log", "--oneline"]).is_empty());
}

#[test]
fn adopting_is_off_unless_the_config_asks_for_it() {
    let env = TestEnv::new("cache_adopt_off");
    let core = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let root = env.create_bare_repo(
        "root",
        "main",
        &[(
            ".gitscale.toml",
            &format!(
                "[cache]\ndir = \"{}\"\n\n{}",
                env.cache.display(),
                config_for(&core.display().to_string())
            ),
        )],
    );

    set_head(&root, "main");

    let ws = env.playground.join("ws");
    git(
        &env.playground,
        &[
            "clone",
            "--quiet",
            root.to_str().unwrap(),
            ws.to_str().unwrap(),
        ],
    );
    let out = gitscale::run_cli_with(&["gitscale", "pull", "-C", ws.to_str().unwrap()], false);
    assert!(out.success, "{:?}", out.stderr);
    assert_eq!(
        alternates_of(&ws),
        None,
        "a repository that stood on its own keeps standing on its own"
    );
}

#[test]
fn a_checkout_that_is_already_shallow_stays_shallow() {
    let env = TestEnv::new("cache_shallow_upgrade");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "v1")]);
    let url = format!("file://{}", bare.display());
    // readonly, so it is cloned shallow — and cloned before this machine had a
    // cache, which is what every existing workspace looks like.
    env.write_config(&format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\", mode = \"readonly\" }}\n",
        url
    ));
    assert!(env.run(&["clone", "--no-cache"]).success);
    let checkout = env.playground.join("libs/core");
    assert_eq!(
        git(&checkout, &["rev-parse", "--is-shallow-repository"]),
        "true"
    );

    let moved = commit_to_bare(&bare, "main", "a.txt", "v2");
    assert!(env.run(&["pull"]).success);

    assert_eq!(
        git(&checkout, &["rev-parse", "HEAD"]),
        moved,
        "it should still be brought up to date, through the cache"
    );
    assert_eq!(
        git(&checkout, &["rev-parse", "--is-shallow-repository"]),
        "true",
        "and deepening it behind the user's back is not the cache's business"
    );
}

#[test]
fn a_readonly_repo_is_cloned_whole_when_a_mirror_serves_it() {
    let env = TestEnv::new("cache_readonly_depth");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "v1")]);
    commit_to_bare(&bare, "main", "a.txt", "v2");
    env.write_config(&format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\", mode = \"readonly\" }}\n",
        bare.display()
    ));

    assert!(env.run(&["clone"]).success);
    let checkout = env.playground.join("libs/core");
    assert_eq!(
        git(&checkout, &["rev-parse", "--is-shallow-repository"]),
        "false",
        "shallow follows the entry kind: a mirror is borrowed from, so it must not force depth 1"
    );
    assert_eq!(
        git(&checkout, &["rev-list", "--count", "HEAD"]),
        "2",
        "the history is there to browse, at no extra cost"
    );
}

#[test]
fn adopting_leaves_a_repository_that_already_borrows_alone() {
    let env = TestEnv::new("cache_adopt_borrowing");
    let core = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let root = env.create_bare_repo(
        "root",
        "main",
        &[(
            ".gitscale.toml",
            &format!(
                "[cache]\ndir = \"{}\"\nadopt_root = true\n\n{}",
                env.cache.display(),
                config_for(&core.display().to_string())
            ),
        )],
    );
    set_head(&root, "main");

    // Two clones, the second borrowing from the first — what `--reference`
    // leaves behind, and what adoption must not overwrite.
    let first = env.playground.join("first");
    let second = env.playground.join("second");
    git(
        &env.playground,
        &[
            "clone",
            "--quiet",
            root.to_str().unwrap(),
            first.to_str().unwrap(),
        ],
    );
    git(
        &env.playground,
        &[
            "clone",
            "--quiet",
            "--reference",
            first.to_str().unwrap(),
            root.to_str().unwrap(),
            second.to_str().unwrap(),
        ],
    );
    let borrowed = alternates_of(&second).expect("the second clone should borrow from the first");

    let out = gitscale::run_cli_with(&["gitscale", "pull", "-C", second.to_str().unwrap()], false);
    assert!(out.success, "{:?}", out.stderr);

    assert_eq!(
        alternates_of(&second).as_deref(),
        Some(borrowed.as_str()),
        "adopting must not repoint a repository that borrows from somewhere else"
    );
    assert!(
        git_ok(&second, &["fsck", "--no-progress"]),
        "and certainly must not leave it unable to read its own objects"
    );
}

#[test]
fn status_flags_a_checkout_whose_entry_has_been_deleted() {
    let env = TestEnv::new("cache_status_broken");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = bare.display().to_string();
    env.write_config(&config_for(&url));
    assert!(env.run(&["clone"]).success);

    let out = env.run(&["status"]);
    assert!(out.success, "{:?}", out.stderr);
    assert_eq!(status_flags(&out, "libs/core"), "ok");
    assert_eq!(cache_state(&env, "libs/core"), "mirror");

    // Delete the entry it borrows from. Git says nothing until something tries
    // to read an object, so status is where this has to surface.
    std::fs::remove_dir_all(env.cache_entry("mirror", &url)).unwrap();

    let out = env.run(&["status"]);
    assert!(out.success, "{:?}", out.stderr);
    assert!(
        status_flags(&out, "libs/core").contains("cache-broken"),
        "a checkout that cannot read its own history should say so: {}",
        status_flags(&out, "libs/core")
    );
    assert_eq!(cache_state(&env, "libs/core"), "broken");

    // And it is the state `cache repair` exists for.
    assert!(cache_cmd(&env, &["repair"]).success);
    let out = env.run(&["status"]);
    assert_eq!(status_flags(&out, "libs/core"), "ok");
    assert_eq!(cache_state(&env, "libs/core"), "mirror");
}

#[test]
fn cache_status_names_entries_after_the_repos_that_declare_them() {
    let env = TestEnv::new("cache_status_cmd");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    env.write_config(&config_for(&bare.display().to_string()));

    let empty = cache_cmd(&env, &["status"]);
    assert!(empty.success, "{:?}", empty.stderr);
    assert!(
        empty.stdout.contains("Nothing cached yet"),
        "{}",
        empty.stdout
    );

    assert!(env.run(&["clone"]).success);
    let out = cache_cmd(&env, &["status"]);
    assert!(out.success, "{:?}", out.stderr);
    let row = out
        .stdout
        .lines()
        .find(|l| l.contains("libs/core"))
        .expect("the entry should be named after the repo that declares it");
    assert!(
        row.contains("KiB") || row.contains(" B"),
        "the row should carry what the mirror costs: {}",
        row
    );
}

#[test]
fn cache_status_lists_every_revision_a_snapshot_holds() {
    let env = TestEnv::new("cache_status_pins");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "v1")]);
    let url = format!("file://{}", bare.display());
    env.write_config(&config_for(&url));
    assert!(env.run_with_env(&[("CI", "1")], &["clone"]).success);
    let first = git(&bare, &["rev-parse", "main"]);

    // A second job on a moved head: the entry now holds two commits, and which
    // of them is still wanted is what `compact` decides on.
    let second = commit_to_bare(&bare, "main", "a.txt", "v2");
    assert!(env.run_with_env(&[("CI", "1")], &["pull"]).success);

    let out = cache_cmd(&env, &["status"]);
    assert!(out.success, "{:?}", out.stderr);
    let row = out
        .stdout
        .lines()
        .find(|l| l.contains("libs/core"))
        .expect("a row for the repo");
    let fields: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(fields[1], "-", "nothing mirrored on a CI machine: {}", row);
    assert!(
        fields[3] == "KiB" || fields[2].ends_with("B"),
        "the snapshot entry should be sized: {}",
        row
    );
    for sha in [&first, &second] {
        let short: String = sha.chars().take(12).collect();
        assert!(
            out.stdout.contains(&short),
            "every cached revision should be listed, missing {}: {}",
            short,
            out.stdout
        );
    }
}

#[test]
fn a_checkout_made_before_the_cache_existed_says_copy() {
    let env = TestEnv::new("cache_copy");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    let url = bare.display().to_string();
    env.write_config(&config_for(&url));

    // Cloned with no cache, so it owns its objects — every checkout on a
    // machine that predates the cache looks like this.
    assert!(env.run(&["clone", "--no-cache"]).success);
    assert_eq!(
        cache_state(&env, "libs/core"),
        "-",
        "nothing is cached for it yet"
    );

    // A pull fills the entry in — cache-first updates it even though this
    // checkout cannot borrow from it, because every later clone can.
    assert!(env.run(&["pull"]).success);
    assert_eq!(
        cache_state(&env, "libs/core"),
        "copy",
        "the entry exists, but this checkout still owns its objects"
    );
    assert_eq!(
        alternates_of(&env.playground.join("libs/core")),
        None,
        "nothing re-links a clone after the fact"
    );
}

#[test]
fn cache_status_counts_every_revision_and_totals_the_entries() {
    let env = TestEnv::new("cache_status_totals");
    let bare = env.create_bare_repo("core", "main", &[("a.txt", "a")]);
    git(&bare, &["tag", "v1", "main"]);
    env.write_config(&config_for(&bare.display().to_string()));
    assert!(env.run(&["clone"]).success);

    let out = cache_cmd(&env, &["status"]);
    assert!(out.success, "{:?}", out.stderr);
    let row = out
        .stdout
        .lines()
        .find(|l| l.contains("libs/core"))
        .expect("a row for the repo");
    let fields: Vec<&str> = row.split_whitespace().collect();
    // REPO, MIRROR (two words), SNAPSHOTS, TOTAL (two words), REVS…
    assert_eq!(fields[3], "-", "no snapshot entry on a developer machine");
    assert_eq!(
        format!("{} {}", fields[1], fields[2]),
        format!("{} {}", fields[4], fields[5]),
        "with only one kind of entry, the total is that entry: {}",
        row
    );
    assert_eq!(
        fields[6], "2",
        "a branch and a tag are two cached revisions: {}",
        row
    );

    // Counted always, listed on request.
    assert!(!out.stdout.contains("\n      v1"), "{}", out.stdout);
    let verbose = gitscale::run_cli_with(
        &[
            "gitscale",
            "-v",
            "cache",
            "status",
            "-C",
            env.playground.to_str().unwrap(),
        ],
        false,
    );
    assert!(verbose.stdout.contains("v1"), "{}", verbose.stdout);
}
