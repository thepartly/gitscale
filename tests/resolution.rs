//! Transitive version resolution, end to end: real repositories, real
//! commands.

#[allow(dead_code)]
mod helpers;

use helpers::{git_stdout, run_git_pub, strip_ansi, TestEnv};
use std::path::{Path, PathBuf};

/// A repository whose `main` gains one commit per `(tag, config)`, each
/// tagged; a non-empty config becomes that commit's `.gitscale.toml`.
fn tagged(env: &TestEnv, name: &str, versions: &[(&str, &str)]) -> PathBuf {
    let bare = env.create_bare_repo(name, "main", &[("README.md", name)]);
    for (tag, config) in versions {
        let mut commit = env.push_commit(&bare, "main", "VERSION", tag);
        if !config.is_empty() {
            commit = env.push_commit(&bare, "main", ".gitscale.toml", config);
        }
        run_git_pub(&bare, &["tag", tag, &commit]);
    }
    bare
}

/// A `[repos]` table: `(directory, repository, extra keys)`.
fn repos(entries: &[(&str, &Path, &str)]) -> String {
    let mut text = String::from("[repos]\n");
    for (dir, bare, extra) in entries {
        text.push_str(&format!(
            "\"{}\" = {{ url = \"{}\"{} }}\n",
            dir,
            bare.display(),
            extra
        ));
    }
    text
}

/// Lets implicit dependencies come from this environment's repositories,
/// which are local paths — never allowed without saying so.
fn allow(env: &TestEnv) -> String {
    format!(
        "[resolve]\nallow = [\"{}/*\"]\n\n",
        env.repos_remote.display()
    )
}

fn tag_commit(bare: &Path, tag: &str) -> String {
    git_stdout(bare, &["rev-parse", &format!("{}^{{commit}}", tag)])
}

fn head(env: &TestEnv, dir: &str) -> String {
    git_stdout(&env.playground.join(dir), &["rev-parse", "HEAD"])
}

fn status_row(env: &TestEnv, dir: &str) -> String {
    let out = env.run(&["status"]);
    assert!(out.success, "{}", out.stderr);
    strip_ansi(&out.stdout)
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(dir))
        .unwrap_or_default()
        .to_string()
}

/// D at v1.2.0, v1.5.0 and v2.0.0; B v1.0.0 needs D v1.5.0.
fn diamond(env: &TestEnv) -> (PathBuf, PathBuf) {
    let d = tagged(env, "d", &[("v1.2.0", ""), ("v1.5.0", ""), ("v2.0.0", "")]);
    let b = tagged(
        env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.5.0\"")]),
        )],
    );
    (b, d)
}

#[test]
fn a_dependency_raises_the_root_and_status_says_why() {
    let env = TestEnv::new("res_raise");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));

    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    let link = env.playground.join("imports/b/libs/d");
    assert_eq!(std::fs::read_link(&link).unwrap(), PathBuf::from("../../d"));

    let row = status_row(&env, "imports/d");
    assert!(row.contains("v1.5.0"), "{}", row);
    // STATUS keeps the checkout's own state; RESOLUTION says how the
    // revision was chosen, and how many asked.
    assert!(row.contains("   ok   "), "{}", row);
    assert!(
        row.ends_with("raised from v1.2.0 by imports/b, 2 requests"),
        "{}",
        row
    );
    let table = strip_ansi(&env.run(&["status"]).stdout);
    assert!(
        table.lines().next().unwrap().ends_with("RESOLUTION"),
        "{}",
        table
    );
    assert!(
        table.contains("hint: gitscale status --why <dir> lists every request"),
        "{}",
        table
    );

    let why = env.run(&["status", "--why", "imports/d"]);
    assert!(why.success, "{}", why.stderr);
    assert!(why.stdout.contains("selected  v1.5.0"), "{}", why.stdout);
    assert!(why.stdout.contains("highest semver"), "{}", why.stdout);
    assert!(
        why.stdout.contains("root → imports/b@v1.0.0"),
        "{}",
        why.stdout
    );

    let json = env.run(&["status", "--format", "json"]);
    let rows: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "imports/d")
        .unwrap();
    assert_eq!(row["declared_ref"], "v1.2.0");
    assert_eq!(row["resolved_ref"], "v1.5.0");
    assert_eq!(row["resolution"], "raised");
    assert_eq!(row["revision_kind"], "tag");
    assert_eq!(row["class"], "1");
    assert_eq!(row["requests"].as_array().unwrap().len(), 2);
}

#[test]
fn an_override_at_the_root_holds_a_dependency_down() {
    let env = TestEnv::new("res_override");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\", override = true"),
    ]));

    assert!(env.run(&["pull"]).success);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.2.0"));
    let row = status_row(&env, "imports/d");
    assert!(row.starts_with('↧'), "{}", row);
    assert!(row.contains("   override   "), "{}", row);
    assert!(
        row.ends_with("held at v1.2.0, imports/b wants v1.5.0, 2 requests"),
        "{}",
        row
    );
}

#[test]
fn upgrade_resolved_records_the_resolved_revisions_and_keeps_comments() {
    let env = TestEnv::new("res_write");
    let (b, d) = diamond(&env);
    env.write_config(&format!(
        "# the workspace\n[repos]\n\"imports/b\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n\
         \"imports/d\" = {{ url = \"{}\", revision = \"v1.2.0\" }} # raised by b\n",
        b.display(),
        d.display()
    ));

    let out = env.run(&["upgrade", "--resolved", "--dry-run"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("v1.2.0 → v1.5.0"), "{}", out.stdout);
    assert!(
        out.stdout.contains("1 entry would change"),
        "{}",
        out.stdout
    );

    let out = env.run(&["upgrade", "--resolved"]);
    assert!(out.success, "{}", out.stderr);
    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    assert!(config.contains("revision = \"v1.5.0\""), "{}", config);
    assert!(config.contains("# the workspace"), "{}", config);
    assert!(config.contains("# raised by b"), "{}", config);

    let out = env.run(&["upgrade", "--resolved"]);
    assert!(out.stdout.contains("already declares"), "{}", out.stdout);
}

#[test]
fn two_majors_get_a_checkout_each_unless_one_is_a_singleton() {
    let env = TestEnv::new("res_majors");
    let d = tagged(&env, "d", &[("v1.5.0", ""), ("v2.0.0", "")]);
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.5.0\"")]),
        )],
    );
    let c = tagged(
        &env,
        "c",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v2.0.0\"")]),
        )],
    );
    let config = format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    );
    env.write_config(&config);
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    assert_eq!(head(&env, "imports/d_v2"), tag_commit(&d, "v2.0.0"));
    assert!(status_row(&env, "imports/d_v2").contains("2 majors"));
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/c/libs/d")).unwrap(),
        PathBuf::from("../../d_v2")
    );

    // Declared at the root as a singleton: one checkout or none.
    let fresh = TestEnv::new("res_singleton");
    let d = tagged(&fresh, "d", &[("v1.5.0", ""), ("v2.0.0", "")]);
    let b = tagged(
        &fresh,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.5.0\"")]),
        )],
    );
    let c = tagged(
        &fresh,
        "c",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v2.0.0\"")]),
        )],
    );
    fresh.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", singleton = true"),
    ]));
    let out = fresh.run(&["pull"]);
    assert!(!out.success);
    assert!(out.stderr.contains("singleton"), "{}", out.stderr);
}

#[test]
fn a_cycle_fails_before_anything_is_cloned() {
    let env = TestEnv::new("res_cycle");
    let b_url = env.repos_remote.join("b.git");
    let c = tagged(
        &env,
        "c",
        &[(
            "v1.0.0",
            &repos(&[("libs/b", &b_url, ", revision = \"v1.0.0\"")]),
        )],
    );
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/c", &c, ", revision = \"v1.0.0\"")]),
        )],
    );
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
    ]));
    let out = env.run(&["pull"]);
    assert!(!out.success);
    assert!(out.stderr.contains("cycle"), "{}", out.stderr);
    assert!(!env.playground.join("imports").exists());
}

#[test]
fn pull_moves_a_checkout_only_when_nothing_can_be_lost() {
    let env = TestEnv::new("res_safe_move");
    let d = tagged(&env, "d", &[("v1.2.0", ""), ("v1.5.0", "")]);
    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.2.0\"")]));
    assert!(env.run(&["pull"]).success);
    let dest = env.playground.join("imports/d");
    helpers::edit(&dest.join("README.md"), "local edit");

    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.5.0\"")]));
    let out = env.run(&["pull"]);
    assert!(!out.success);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(text.contains("not moved: uncommitted changes"), "{}", text);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.2.0"));
    assert_eq!(
        std::fs::read_to_string(dest.join("README.md")).unwrap(),
        "local edit"
    );

    run_git_pub(&dest, &["checkout", "--", "README.md"]);
    assert!(env.run(&["pull"]).success);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
}

#[test]
fn calendar_versions_order_by_date_then_modifier() {
    let env = TestEnv::new("res_calver");
    let d = tagged(
        &env,
        "d",
        &[
            ("v2026.09.30-rc1", ""),
            ("v2026.09.30", ""),
            ("v2026.09.30-2", ""),
            ("v2026.09.30-11", ""),
        ],
    );
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v2026.09.30-11\"")]),
        )],
    );
    let c = tagged(
        &env,
        "c",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v2026.09.30-2\"")]),
        )],
    );
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v2026.09.30\""),
    ]));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v2026.09.30-11"));
}

/// In CI without the cache, resolution reads refs from `ls-remote` and each
/// config from one commit fetched at depth 1: no history crosses the wire.
#[test]
fn resolving_in_ci_without_the_cache_fetches_no_history() {
    let env = TestEnv::new("res_no_history");
    env.init_playground_git();
    let (b, _d) = diamond(&env);
    // Plenty of history behind the commit resolution reads.
    for i in 0..5 {
        env.push_commit(&b, "main", "noise.txt", &i.to_string());
    }
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")])
    ));
    let out = env.run_with_env(&[("CI", "true")], &["pull", "--no-cache"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&_d, "v1.5.0"));

    let stores = env.playground.join(".git/gitscale/resolve");
    let store = std::fs::read_dir(&stores)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().contains("-b-"))
        .expect("a store for b");
    assert!(store.join("gitscale-refs.json").is_file());
    let objects = git_stdout(
        &store,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objecttype)",
        ],
    );
    let held = objects.lines().filter(|t| *t == "commit").count();
    let total: usize = git_stdout(&b, &["rev-list", "--all", "--count"])
        .parse()
        .unwrap();
    assert!(
        held == 1 && total > 5,
        "the store holds {} of {} commits",
        held,
        total
    );

    // Offline afterwards: status reads what is on disk.
    let out = env.run_with_env(&[("CI", "true")], &["status"]);
    let table = strip_ansi(&out.stdout);
    let row = table.lines().find(|l| l.contains("imports/d")).unwrap();
    assert!(row.contains("implicit via imports/b"), "{}", row);
}

/// An artefact's dependencies travel in its image's config layer: resolved
/// before anything is installed, and linked inside the artefact checkout.
#[test]
fn an_artefact_brings_its_dependencies_in_its_config_layer() {
    let env = TestEnv::new("res_artefact_deps");
    let dep = tagged(&env, "dep", &[("v1.0.0", "")]);
    let art = env.create_bare_repo("art", "main", &[("README.md", "art")]);
    let producer = format!(
        "[artefact]\ninclude = [\"dist/**\"]\n\n{}",
        repos(&[("vendor/dep", &dep, ", revision = \"v1.0.0\"")])
    );
    let (published, _) = env.publish_with(&art, "main", &producer, &[("app.bin", "x")], &[]);
    assert!(
        published.success,
        "{}{}",
        published.stdout, published.stderr
    );

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
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/dep"), tag_commit(&dep, "v1.0.0"));
    let link = env.playground.join("meta/app/vendor/dep");
    assert!(link.is_symlink(), "linked inside the artefact");
    assert!(link.join("VERSION").is_file());
}

/// An implicit checkout nothing asks for any more is removed by `sync` when
/// that loses nothing; a clone made by hand under the hoist directory never.
#[test]
fn sync_removes_an_implicit_checkout_left_behind() {
    let env = TestEnv::new("res_left_behind");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = tagged(
        &env,
        "b",
        &[
            (
                "v1.0.0",
                &repos(&[("libs/d", &d, ", revision = \"v1.0.0\"")]),
            ),
            ("v1.1.0", "[repos]\n"),
        ],
    );
    let config = |rev: &str| {
        format!(
            "{}{}",
            allow(&env),
            repos(&[("imports/b", &b, &format!(", revision = \"{}\"", rev))])
        )
    };
    env.write_config(&config("v1.0.0"));
    assert!(env.run(&["pull"]).success);
    assert!(env.playground.join("imports/d").is_dir());
    // Someone's own clone, which gitscale did not make.
    run_git_pub(
        &env.playground,
        &["clone", "--quiet", d.to_str().unwrap(), "imports/mine"],
    );

    env.write_config(&config("v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("remove  imports/d (no longer needed)"),
        "{}",
        out.stdout
    );
    assert!(!env.playground.join("imports/d").exists());
    assert!(env.playground.join("imports/mine").is_dir());
}

/// One entry that cannot be brought up to date — an artefact whose pipeline
/// has not published yet — fails the command, but only after everything else
/// is done: the rest pulled and linked.
#[test]
fn one_failing_entry_fails_the_command_after_the_rest_is_done() {
    let env = TestEnv::new("res_fail_at_end");
    let (b, d) = diamond(&env);
    let art = env.artefact_repo("art", &[("app.bin", "v1")]);
    env.write_config(&format!(
        "{}{}",
        env.registries(),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/d", &d, ", revision = \"v1.2.0\""),
            (
                "meta/art",
                &art,
                ", revision = \"main\", artefact = \"replace\""
            ),
        ])
    ));
    // A commit whose image does not exist yet, before anything is cloned.
    env.push_commit(&art, "main", "README.md", "unpublished");

    for command in ["pull", "sync"] {
        let _ = std::fs::remove_dir_all(env.playground.join("imports"));
        let out = env.run(&[command]);
        let text = format!("{}{}", out.stdout, out.stderr);
        assert!(!out.success, "{} should fail: {}", command, text);
        assert!(
            text.contains("may not have published yet"),
            "{}: {}",
            command,
            text
        );
        assert_eq!(
            head(&env, "imports/d"),
            tag_commit(&d, "v1.5.0"),
            "{}: the rest is still pulled",
            command
        );
        assert!(
            env.playground.join("imports/b/libs/d").is_symlink(),
            "{}: and linked",
            command
        );
    }
}

/// A checkout moves with a plain checkout, so an untracked file in the way of
/// the new revision stops the move rather than being overwritten.
#[test]
fn a_move_never_overwrites_an_untracked_file() {
    let env = TestEnv::new("res_untracked_in_the_way");
    let d = env.create_bare_repo("d", "main", &[("README.md", "d")]);
    let v1 = env.push_commit(&d, "main", "VERSION", "1");
    run_git_pub(&d, &["tag", "v1", &v1]);
    let v2 = env.push_commit(&d, "main", "new.txt", "from v2");
    run_git_pub(&d, &["tag", "v2", &v2]);
    let url = format!("file://{}", d.display());
    let entry = |rev: &str| {
        format!(
            "[repos]\n\"imports/d\" = {{ url = \"{}\", revision = \"{}\" }}\n",
            url, rev
        )
    };
    env.write_config(&entry("v1"));
    assert!(env.run(&["pull"]).success);
    let dest = env.playground.join("imports/d");
    std::fs::write(dest.join("new.txt"), "mine").unwrap();

    env.write_config(&entry("v2"));
    let out = env.run(&["pull"]);
    assert!(!out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        std::fs::read_to_string(dest.join("new.txt")).unwrap(),
        "mine"
    );
    assert_eq!(head(&env, "imports/d"), v1);
}

/// The links gitscale plants in a checkout are not its owner's work: they do
/// not make it dirty, but where they are not ignored, status says so and how
/// to fix it.
#[test]
fn links_a_repository_does_not_ignore_are_flagged_apart_from_dirty() {
    let env = TestEnv::new("res_untracked_links");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["pull"]).success);

    let out = env.run(&["status"]);
    let text = strip_ansi(&out.stdout);
    let row = text.lines().find(|l| l.contains("imports/b")).unwrap();
    assert!(row.contains("untracked-links"), "{}", text);
    assert!(!row.contains("dirty"), "{}", text);
    assert!(
        text.contains("hint: imports/b: ") && text.contains("add /libs/ to its .gitignore"),
        "{}",
        text
    );

    // Real work in the same checkout is still dirty.
    helpers::edit(&env.playground.join("imports/b/README.md"), "edited");
    let row = status_row(&env, "imports/b");
    assert!(row.contains("dirty, untracked-links"), "{}", row);

    // Ignored, the links are nobody's business.
    let checkout = env.playground.join("imports/b");
    run_git_pub(&checkout, &["checkout", "--", "README.md"]);
    // A worktree's exclude file is its store's, shared by every worktree.
    let exclude = git_stdout(&checkout, &["rev-parse", "--git-path", "info/exclude"]);
    let exclude = checkout.join(exclude);
    std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    std::fs::write(&exclude, "/libs/\n").unwrap();
    let row = status_row(&env, "imports/b");
    assert!(
        !row.contains("untracked-links") && !row.contains("dirty"),
        "{}",
        row
    );
}

/// A left-behind implicit checkout holding work is kept, and the sync fails
/// at the end saying so; `--force` removes it.
#[test]
fn sync_keeps_a_left_behind_checkout_with_work_and_fails() {
    let env = TestEnv::new("res_left_behind_work");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = tagged(
        &env,
        "b",
        &[
            (
                "v1.0.0",
                &repos(&[("libs/d", &d, ", revision = \"v1.0.0\"")]),
            ),
            ("v1.1.0", "[repos]\n"),
        ],
    );
    let config = |rev: &str| {
        format!(
            "{}{}",
            allow(&env),
            repos(&[("imports/b", &b, &format!(", revision = \"{}\"", rev))])
        )
    };
    env.write_config(&config("v1.0.0"));
    assert!(env.run(&["pull"]).success);
    std::fs::write(env.playground.join("imports/d/notes.txt"), "mine").unwrap();

    env.write_config(&config("v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("skip  imports/d (no longer needed, but modified"),
        "{}",
        out.stdout
    );
    assert!(out.stderr.contains("use --force"), "{}", out.stderr);
    assert!(env.playground.join("imports/d/notes.txt").is_file());

    let out = env.run(&["sync", "--force"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!env.playground.join("imports/d").exists());
}

/// A dependency pinned to a revision its repository does not have fails
/// resolution — unless the root overrides that dependency, which is how it
/// works around a broken pin it cannot edit.
#[test]
fn an_override_from_above_works_around_a_missing_revision() {
    let env = TestEnv::new("res_missing_rev");
    let d = tagged(&env, "d", &[("v1.2.0", "")]);
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"develop\"")]),
        )],
    );
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    let out = env.run(&["pull"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("'develop' is not a branch, tag or commit"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());

    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\", override = true"),
    ]));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.2.0"));
    let row = status_row(&env, "imports/d");
    assert!(
        row.ends_with("held at v1.2.0, imports/b wants develop (no such revision), 2 requests"),
        "{}",
        row
    );
}

/// An entry removed from the root config goes with the next sync, as long as
/// that loses nothing; renamed, the old checkout goes and the new one comes.
#[test]
fn sync_removes_a_checkout_whose_entry_was_removed() {
    let env = TestEnv::new("res_removed_entry");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let e = tagged(&env, "e", &[("v1.0.0", "")]);
    env.write_config(&repos(&[
        ("imports/d", &d, ", revision = \"v1.0.0\""),
        ("imports/e", &e, ", revision = \"v1.0.0\""),
    ]));
    assert!(env.run(&["sync"]).success);

    // Removed, and renamed.
    env.write_config(&repos(&[("libs/e", &e, ", revision = \"v1.0.0\"")]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("remove  imports/d (no longer needed)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("remove  imports/e (no longer needed)"),
        "{}",
        out.stdout
    );
    assert!(!env.playground.join("imports/d").exists());
    assert!(!env.playground.join("imports/e").exists());
    assert_eq!(head(&env, "libs/e"), tag_commit(&e, "v1.0.0"));
}

/// An implicit artefact nothing asks for any more is removed too, with what
/// gitscale recorded about installing it.
#[test]
fn sync_removes_a_left_behind_implicit_artefact() {
    let env = TestEnv::new("res_left_behind_artefact");
    let art = env.artefact_repo("art", &[("app.bin", "x")]);
    run_git_pub(&art, &["tag", "v1.0.0", "main"]);
    let b = tagged(
        &env,
        "b",
        &[
            (
                "v1.0.0",
                &repos(&[(
                    "libs/art",
                    &art,
                    ", revision = \"v1.0.0\", artefact = \"replace\"",
                )]),
            ),
            ("v1.1.0", "[repos]\n"),
        ],
    );
    env.init_playground_git();
    let config = |rev: &str| {
        format!(
            "{}{}{}",
            env.registries(),
            allow(&env),
            repos(&[("imports/b", &b, &format!(", revision = \"{}\"", rev))])
        )
    };
    env.write_config(&config("v1.0.0"));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/art/dist/app.bin").is_file());
    let records = env.playground.join(".git/gitscale/artefacts");
    assert!(std::fs::read_dir(&records).unwrap().count() > 0);

    env.write_config(&config("v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("remove  imports/art (no longer needed)"),
        "{}",
        out.stdout
    );
    assert!(!env.playground.join("imports/art").exists());
    assert_eq!(std::fs::read_dir(&records).unwrap().count(), 0);
}

// ---------------------------------------------------------------------------
// Safety: what a command must never delete or move
// ---------------------------------------------------------------------------

/// A b whose v1.0.0 needs d at v1.5.0, with nothing declaring d at the root:
/// d is an implicit checkout.
fn implicit_d(env: &TestEnv) -> (PathBuf, PathBuf) {
    let (b, d) = diamond(env);
    env.write_config(&format!(
        "{}{}",
        allow(env),
        repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")])
    ));
    (b, d)
}

/// The root's clean keeps the implicit checkouts, which are untracked
/// directories to it like any declared one, and a dependency's clean keeps
/// the links planted in it.
#[test]
fn clean_keeps_implicit_checkouts_and_their_links() {
    let env = TestEnv::new("res_clean_implicit");
    env.init_playground_git();
    implicit_d(&env);
    assert!(env.run(&["pull"]).success);
    std::fs::write(env.playground.join("stray.txt"), "x").unwrap();

    let out = env.run(&["clean", "-f"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!env.playground.join("stray.txt").exists(), "the clean ran");
    assert!(env.playground.join("imports/d/VERSION").is_file());
    assert!(env.playground.join("imports/b/libs/d").is_symlink());
}

/// A detached HEAD with a commit no branch holds is not moved: switching away
/// would leave that commit reachable only through the reflog.
#[test]
fn pull_does_not_move_a_detached_head_with_commits_on_no_branch() {
    let env = TestEnv::new("res_detached_work");
    let d = tagged(&env, "d", &[("v1.2.0", ""), ("v1.5.0", "")]);
    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.2.0\"")]));
    assert!(env.run(&["pull"]).success);
    let dest = env.playground.join("imports/d");
    run_git_pub(
        &dest,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "--allow-empty",
            "-m",
            "work on no branch",
        ],
    );
    let work = head(&env, "imports/d");

    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.5.0\"")]));
    let out = env.run(&["pull"]);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(!out.success, "{}", text);
    assert!(text.contains("is on no branch"), "{}", text);
    assert_eq!(head(&env, "imports/d"), work);
}

/// A recorded checkout nothing needs is still kept while it holds a
/// checkout that is wanted.
#[test]
fn sync_never_removes_a_directory_holding_a_wanted_checkout() {
    let env = TestEnv::new("res_ledger_nested");
    let outer = tagged(&env, "outer", &[("v1.0.0", "")]);
    let inner = tagged(&env, "inner", &[("v1.0.0", "")]);
    env.write_config(&repos(&[
        ("deps", &outer, ", revision = \"v1.0.0\""),
        ("deps/inner", &inner, ", revision = \"v1.0.0\""),
    ]));
    assert!(env.run(&["sync"]).success);

    env.write_config(&repos(&[("deps/inner", &inner, ", revision = \"v1.0.0\"")]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("remove  deps "), "{}", out.stdout);
    assert!(env.playground.join("deps/inner/VERSION").is_file());
}

/// In CI, resolution reads each config from the commit the checkout is built
/// from, with no history fetched; and a move is never refused.
#[test]
fn resolution_and_moves_in_ci() {
    for cache in [true, false] {
        let env = TestEnv::new(&format!("res_ci_{}", cache));
        let (b, d) = implicit_d(&env);
        let mut args = vec!["pull"];
        if !cache {
            args.push("--no-cache");
        }
        let out = env.run_with_env(&[("CI", "true")], &args);
        assert!(out.success, "cache {}: {}{}", cache, out.stdout, out.stderr);
        assert_eq!(
            head(&env, "imports/d"),
            tag_commit(&d, "v1.5.0"),
            "cache {}",
            cache
        );
        assert!(env.playground.join("imports/b/libs/d").is_symlink());

        // A tracked change left by the last job does not stop the move.
        let _ = b;
        env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.2.0\"")]));
        let out = env.run_with_env(&[("CI", "true")], &["pull"]);
        assert!(out.success, "{}{}", out.stdout, out.stderr);
        let dest = env.playground.join("imports/d");
        set_writable(&dest.join("README.md"));
        std::fs::write(dest.join("README.md"), "left by the last job").unwrap();
        env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.5.0\"")]));
        let out = env.run_with_env(&[("CI", "true")], &["pull"]);
        assert!(out.success, "cache {}: {}{}", cache, out.stdout, out.stderr);
        assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    }
}

fn set_writable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(perms.mode() | 0o200);
    std::fs::set_permissions(path, perms).unwrap();
}

/// A graph that does not resolve still gets its declared entries fetched,
/// and the fetch fails afterwards with the reason.
#[test]
fn fetch_fetches_the_declared_entries_when_resolution_fails() {
    let env = TestEnv::new("res_fetch_fallback");
    let d = tagged(&env, "d", &[("v1.5.0", "")]);
    // main moves on past the tag, so the two requests name different commits.
    env.push_commit(&d, "main", "VERSION", "next");
    let b = tagged(
        &env,
        "b",
        &[("v1.0.0", &repos(&[("libs/d", &d, ", revision = \"main\"")]))],
    );
    let c = tagged(
        &env,
        "c",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.5.0\"")]),
        )],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")])
    ));
    assert!(env.run(&["pull"]).success);
    let fresh = env.push_commit(&b, "main", "news.txt", "new");

    // c's request for d cannot be ordered against b's: neither is above.
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ""),
    ]));
    let out = env.run(&["fetch"]);
    assert!(!out.success);
    assert!(out.stderr.contains("cannot order"), "{}", out.stderr);
    assert_eq!(
        git_stdout(
            &env.playground.join("imports/b"),
            &["rev-parse", "origin/main"]
        ),
        fresh,
        "the declared entry was fetched anyway"
    );
}

// ---------------------------------------------------------------------------
// Behaviour the docs promise
// ---------------------------------------------------------------------------

/// An override in a dependency: it wins over what that dependency is above,
/// and must agree with what it is not.
#[test]
fn an_override_in_a_dependency_reaches_only_what_it_is_above() {
    let env = TestEnv::new("res_child_override");
    let d = tagged(&env, "d", &[("v1.3.1", ""), ("v1.5.0", ""), ("v1.6.0", "")]);
    let e = tagged(
        &env,
        "e",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.6.0\"")]),
        )],
    );
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[
                ("libs/d", &d, ", revision = \"v1.5.0\", override = true"),
                ("libs/e", &e, ", revision = \"v1.0.0\""),
            ]),
        )],
    );
    let c_with = |wants: &str| {
        let name = format!("c{}", wants.replace('.', ""));
        tagged(
            &env,
            &name,
            &[(
                "v1.0.0",
                &repos(&[("libs/d", &d, &format!(", revision = \"{}\"", wants))]),
            )],
        )
    };
    let config = |c: &Path| {
        format!(
            "{}{}",
            allow(&env),
            repos(&[
                ("imports/b", &b, ", revision = \"v1.0.0\""),
                ("imports/c", c, ", revision = \"v1.0.0\""),
            ])
        )
    };

    // e is below b: b's override wins over it. c asks for less: it agrees.
    env.write_config(&config(&c_with("v1.3.1")));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    let row = status_row(&env, "imports/d");
    assert!(row.contains("override"), "{}", row);
    assert!(
        row.contains("held at v1.5.0 by imports/b, imports/e wants v1.6.0"),
        "{}",
        row
    );

    // c asks for more, and b is not above c: no order without the root.
    std::fs::remove_dir_all(env.playground.join("imports")).unwrap();
    env.write_config(&config(&c_with("v1.6.0")));
    let out = env.run(&["pull"]);
    assert!(!out.success);
    assert!(out.stderr.contains("override conflict"), "{}", out.stderr);
}

/// Offline, status reads what is on this machine: a repository nothing has
/// fetched yet is unresolved until `status --fetch`. In CI, where checkouts
/// hold no history, that is the light stores resolution keeps.
#[test]
fn status_is_unresolved_until_fetched() {
    let env = TestEnv::new("res_offline");
    env.init_playground_git();
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    // Before anything: only missed.
    let row = status_row(&env, "imports/b");
    assert!(row.ends_with("missed"), "{}", row);

    let ci = [("CI", "true")];
    assert!(env.run_with_env(&ci, &["pull", "--no-cache"]).success);
    // The workspace's own stores go; the checkouts stay.
    std::fs::remove_dir_all(env.playground.join(".git/gitscale/resolve")).unwrap();
    let out = env.run_with_env(&ci, &["status"]);
    let table = strip_ansi(&out.stdout);
    let row = table.lines().find(|l| l.contains("imports/d")).unwrap();
    assert!(row.contains("unresolved"), "{}", table);
    assert!(
        row.contains("not fetched yet: run status --fetch"),
        "{}",
        table
    );

    let out = env.run_with_env(&ci, &["status", "--fetch", "--no-cache"]);
    let table = strip_ansi(&out.stdout);
    let row = table.lines().find(|l| l.contains("imports/d")).unwrap();
    assert!(!row.contains("unresolved"), "{}", table);
    assert!(row.contains("raised from v1.2.0 by imports/b"), "{}", table);
}

/// A dependency's checkout at the selected commit is read from disk, so an
/// edit to its `.gitscale.toml` not yet committed takes effect at once.
#[test]
fn an_uncommitted_config_edit_takes_effect() {
    let env = TestEnv::new("res_worktree_config");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["pull"]).success);
    assert!(status_row(&env, "imports/d").contains("v1.5.0"));

    // b now asks for no more than the root does.
    helpers::edit(
        &env.playground.join("imports/b/.gitscale.toml"),
        &repos(&[("libs/d", &d, ", revision = \"v1.2.0\"")]),
    );
    let row = status_row(&env, "imports/d");
    let expected = row.split_whitespace().nth(5).unwrap_or_default();
    assert_eq!(expected, "v1.2.0", "{}", row);
    assert!(row.contains("ref-mismatch"), "{}", row);
}

#[test]
fn why_shows_the_shared_checkouts_or_the_ones_named() {
    let env = TestEnv::new("res_why");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["pull"]).success);

    let out = env.run(&["status", "--why"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.starts_with("imports/d  "), "{}", out.stdout);
    assert!(
        !out.stdout.contains("imports/b  "),
        "only shared checkouts: {}",
        out.stdout
    );

    let out = env.run(&["status", "--why", "imports/b"]);
    assert!(out.stdout.starts_with("imports/b  "), "{}", out.stdout);
    assert!(out.stdout.contains("only request"), "{}", out.stdout);

    let out = env.run(&["status", "--why", "nowhere"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("no checkout at nowhere"),
        "{}",
        out.stderr
    );
}

#[test]
fn bad_resolution_config_is_refused_when_read() {
    let env = TestEnv::new("res_bad_config");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    for (config, message) in [
        (
            repos(&[("imports/d", &d, ", override = true")]),
            "override = true without a revision",
        ),
        (
            repos(&[("imports/d", &d, ""), ("libs/d", &d, "")]),
            "same repository with no revision",
        ),
        (
            format!(
                "[resolve]\nhoist_dir = \"../out\"\n\n{}",
                repos(&[("imports/d", &d, "")])
            ),
            "resolve.hoist_dir",
        ),
    ] {
        env.write_config(&config);
        let out = env.run(&["status"]);
        assert!(!out.success, "{}", config);
        assert!(out.stderr.contains(message), "{}: {}", message, out.stderr);
    }
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

/// A child of `name` at v1.0.0 whose config is `entries`.
fn dependant(env: &TestEnv, name: &str, entries: &[(&str, &Path, &str)]) -> PathBuf {
    tagged(env, name, &[("v1.0.0", &repos(entries))])
}

#[test]
fn placement_follows_the_hoist_dir_majors_kind_and_names() {
    // A hoist directory of our own, and 0.x majors side by side.
    let env = TestEnv::new("res_place_majors");
    let d = tagged(&env, "d", &[("v0.3.0", ""), ("v0.4.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v0.3.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v0.4.0\"")]);
    env.write_config(&format!(
        "[resolve]\nhoist_dir = \"vendor\"\nallow = [\"{}/*\"]\n\n{}",
        env.repos_remote.display(),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "vendor/d"), tag_commit(&d, "v0.3.0"));
    assert_eq!(head(&env, "vendor/d_v0.4"), tag_commit(&d, "v0.4.0"));

    // Dependants naming it differently: the repository's own name.
    let env = TestEnv::new("res_place_names");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/dee", &d, ", revision = \"v1.0.0\"")]);
    let c = dependant(&env, "c", &[("vendor/d2", &d, ", revision = \"v1.0.0\"")]);
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/d/VERSION").is_file());

    // Two repositories wanting one path.
    let env = TestEnv::new("res_place_clash");
    let x = tagged(&env, "x", &[("v1.0.0", "")]);
    let y = tagged(&env, "y", &[("v1.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/shared", &x, ", revision = \"v1.0.0\"")]);
    let c = dependant(&env, "c", &[("libs/shared", &y, ", revision = \"v1.0.0\"")]);
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["pull"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("two repositories want imports/shared"),
        "{}",
        out.stderr
    );
}

/// The source and the built artefact of one repository are two checkouts.
#[test]
fn an_artefact_beside_the_source_of_one_repository() {
    let env = TestEnv::new("res_place_artefact");
    let d = env.artefact_repo("d", &[("d.bin", "built")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let c = dependant(
        &env,
        "c",
        &[(
            "libs/d",
            &d,
            ", revision = \"v1.0.0\", artefact = \"replace\"",
        )],
    );
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        env.playground.join("imports/d/README.md").is_file(),
        "the source"
    );
    assert!(
        env.playground
            .join("imports/d_artefact/dist/d.bin")
            .is_file(),
        "the build"
    );
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/c/libs/d")).unwrap(),
        PathBuf::from("../../d_artefact")
    );
}

/// `cache update` warms the checkouts resolution settles on, the implicit
/// ones included.
#[test]
fn cache_update_covers_implicit_dependencies() {
    let env = TestEnv::new("res_cache_update");
    let (_, d) = implicit_d(&env);
    let out = env.run_with_env(&[], &["cache", "update"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("imports/d"), "{}", out.stdout);
    let snapshot = env.cache_entry("snapshots", d.to_str().unwrap());
    assert!(snapshot.join("HEAD").is_file(), "{}", snapshot.display());
}

/// Where the root's store holds history, a winner by position that is behind
/// what a losing request asked for is flagged — and still wins.
#[test]
fn a_winner_behind_a_request_is_flagged_where_history_is_local() {
    let env = TestEnv::new("res_behind");
    let d = tagged(&env, "d", &[("v1.3.1", ""), ("v1.5.0", "")]);
    run_git_pub(&d, &["branch", "stable", "v1.3.1"]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    env.write_config(&repos(&[
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"stable\""),
    ]));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.3.1"));
    let row = status_row(&env, "imports/d");
    assert!(row.starts_with('↧'), "{}", row);
    assert!(row.contains("behind imports/c's v1.5.0"), "{}", row);
}

/// `upgrade --resolved` edits table-style entries as well as inline ones.
#[test]
fn upgrade_resolved_edits_table_style_entries() {
    let env = TestEnv::new("res_write_table");
    let (b, d) = diamond(&env);
    env.write_config(&format!(
        "[repos.\"imports/b\"]\nurl = \"{}\"\nrevision = \"v1.0.0\"\n\n\
         [repos.\"imports/d\"]\nurl = \"{}\"\nrevision = \"v1.2.0\"  # held back?\n",
        b.display(),
        d.display()
    ));
    let out = env.run(&["upgrade", "--resolved"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    assert!(
        config.contains("revision = \"v1.5.0\"  # held back?"),
        "{}",
        config
    );
    assert!(config.contains("[repos.\"imports/d\"]"), "{}", config);
}

// ---------------------------------------------------------------------------
// Found by the doc review
// ---------------------------------------------------------------------------

/// A checkout nothing needs, whose only change is the links gitscale planted
/// in it, holds nothing to lose and goes.
#[test]
fn sync_removes_a_checkout_whose_only_change_is_its_links() {
    let env = TestEnv::new("res_remove_with_links");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["sync"]).success);
    assert!(env.playground.join("imports/b/libs/d").is_symlink());

    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.5.0\"")]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!env.playground.join("imports/b").exists(), "{}", out.stdout);
}

/// `sync` takes the name of an implicit checkout, as `pull` does.
#[test]
fn sync_takes_an_implicit_checkout_by_name() {
    let env = TestEnv::new("res_sync_implicit_name");
    implicit_d(&env);
    assert!(env.run(&["pull"]).success);
    let out = env.run(&["sync", "imports/d"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
}

/// A row with nothing on disk says `missed` and nothing more, even where
/// resolution has a story to tell.
#[test]
fn a_missed_row_has_no_resolution_text() {
    let env = TestEnv::new("res_missed_quiet");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["pull"]).success);
    std::fs::remove_dir_all(env.playground.join("imports/d")).unwrap();
    let row = status_row(&env, "imports/d");
    assert!(row.ends_with("missed"), "{}", row);
}

/// When an entry fails to fetch and resolution fails too, both are said.
#[test]
fn fetch_reports_both_failures() {
    let env = TestEnv::new("res_fetch_both");
    let d = tagged(&env, "d", &[("v1.5.0", "")]);
    env.push_commit(&d, "main", "VERSION", "next");
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"main\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    let gone = tagged(&env, "gone", &[("v1.0.0", "")]);
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/gone", &gone, ", revision = \"v1.0.0\""),
        ])
    ));
    assert!(env.run(&["pull"]).success);
    std::fs::remove_dir_all(&gone).unwrap();
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ""),
        ("imports/gone", &gone, ", revision = \"v1.0.0\""),
    ]));
    let out = env.run(&["fetch"]);
    assert!(!out.success);
    assert!(out.stderr.contains("failed to fetch"), "{}", out.stderr);
    assert!(out.stderr.contains("cannot resolve"), "{}", out.stderr);
}
