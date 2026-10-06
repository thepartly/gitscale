//! Transitive version resolution, end to end: real repositories, real
//! commands.

use crate::support;
use crate::support::resolution::*;
use crate::support::workspace::*;
use crate::support::{git_stdout, run_git_pub, strip_ansi, TestEnv};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// A root entry without a revision gets the one a dependency asks for.
#[test]
fn normal_001_a_root_entry_without_a_revision_takes_a_dependencys() {
    let env = TestEnv::new("recursive_revision_from_dep");

    // Create B with two branches
    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "B main")]);
    // Add a develop branch
    {
        let tmp = env.repos_remote.join("repoB-checkout");
        let _ = std::fs::remove_dir_all(&tmp);
        support::run_git_pub(
            &env.repos_remote,
            &["clone", bare_b.to_str().unwrap(), tmp.to_str().unwrap()],
        );
        support::run_git_pub(&tmp, &["config", "user.email", "t@t.com"]);
        support::run_git_pub(&tmp, &["config", "user.name", "T"]);
        support::run_git_pub(&tmp, &["checkout", "-b", "develop"]);
        std::fs::write(tmp.join("b.txt"), "B develop").unwrap();
        support::run_git_pub(&tmp, &["add", "."]);
        support::run_git_pub(&tmp, &["commit", "-m", "dev"]);
        support::run_git_pub(&tmp, &["push", "origin", "develop"]);
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

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // B should be checked out on "develop"
    let b_txt = std::fs::read_to_string(env.playground.join("repoB/b.txt")).unwrap();
    assert_eq!(b_txt, "B develop");

    // Symlink should exist
    let link = env.playground.join("repoA/libs/b");
    assert!(link.is_symlink());
}

#[test]
fn normal_002_the_root_revision_wins() {
    let env = TestEnv::new("recursive_root_revision_wins");

    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "B main")]);
    bare_git_stdout(&bare_b, &["branch", "develop", "main"]);
    commit_to_bare(&bare_b, "develop", "b.txt", "B develop");

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

    // Root pins B at "main". Neither branch is a version, so position
    // decides: the root is above every repository, and wins.
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"repoB" = {{ url = "{}", revision = "main" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));

    let out = env.run(&["sync"]);
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

/// A tag a dependency asks for, for a root entry without a revision, is where
/// the checkout lands — not on the default branch's tip — and its files are
/// readonly.
#[test]
fn normal_003_a_tag_a_dependency_asks_for_lands_detached_and_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let env = TestEnv::new("dep_tag_readonly");
    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "B v1")]);
    let tagged = bare_git_stdout(&bare_b, &["rev-parse", "main"]);
    support::run_git_pub(&bare_b, &["tag", "v1", &tagged]);
    // The tip moves on: v1 is not what the default branch has.
    commit_to_bare(&bare_b, "main", "b.txt", "B v2");
    let url_b = format!("file://{}", bare_b.display());
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"v1\" }}\n",
                    url_b
                ),
            ),
        ],
    );
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"repoB" = {{ url = "{}" }}
"#,
        bare_a.display(),
        url_b,
    ));

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);

    let dest = env.playground.join("repoB");
    assert_eq!(git_stdout(&dest, &["rev-parse", "HEAD"]), tagged);
    let file = dest.join("b.txt");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "B v1");
    let mode = std::fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o222, 0, "b.txt should still be readonly");
}

/// A revision a child pins for a root entry that has none is applied by sync
/// too, as a fresh clone would — including one the sync itself brings in.
#[test]
fn normal_004_sync_moves_to_a_revision_a_dependency_starts_asking_for() {
    let env = TestEnv::new("pull_dep_asks_later");
    let bare_b = env.create_bare_repo("repoB", "main", &[("b.txt", "B main")]);
    bare_git_stdout(&bare_b, &["branch", "develop", "main"]);
    let develop_tip = commit_to_bare(&bare_b, "develop", "b.txt", "B develop");
    let bare_a = env.create_bare_repo("repoA", "main", &[("a.txt", "A")]);
    env.write_config(&format!(
        r#"[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"repoB" = {{ url = "{}" }}
"#,
        bare_a.display(),
        bare_b.display(),
    ));
    assert!(env.run(&["sync"]).success);
    let dest_b = env.playground.join("repoB");
    assert_eq!(
        std::fs::read_to_string(dest_b.join("b.txt")).unwrap(),
        "B main"
    );

    // repoA starts pinning B at develop; only syncing repoA reveals that.
    commit_to_bare(
        &bare_a,
        "main",
        ".gitscale.toml",
        &format!(
            "[repos]\n\"libs/b\" = {{ url = \"{}\", revision = \"develop\" }}\n",
            bare_b.display()
        ),
    );
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(git_stdout(&dest_b, &["rev-parse", "HEAD"]), develop_tip);
    assert!(env.playground.join("repoA/libs/b").is_symlink());
}

/// A dependency the root does not declare is checked out implicitly, under
/// `imports/` and readonly — but only from somewhere the allowlist covers.
/// The root's own entries allow their host and owner; a local path is never
/// allowed that way, only by `[resolve] allow`.
#[test]
fn normal_005_an_undeclared_dependency_is_checked_out_implicitly_where_allowed() {
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

    let out = env.run(&["sync"]);
    assert!(!out.success, "should fail when child dep is not allowed");
    assert!(
        out.stderr.contains("not on the allowlist"),
        "stderr: {}",
        out.stderr
    );
    assert!(
        !env.playground.join("repoA").exists(),
        "nothing is cloned before resolution succeeds"
    );

    env.write_config(&format!(
        r#"[resolve]
allow = ["{}/*"]

[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"#,
        env.repos_remote.display(),
        bare_a.display(),
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let implicit = env.playground.join("imports/b");
    assert_eq!(
        std::fs::read_to_string(implicit.join("b.txt")).unwrap(),
        "B"
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(implicit.join("b.txt"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o222, 0, "an implicit checkout is readonly");
    let link = env.playground.join("repoA/libs/b");
    assert!(link.is_symlink());
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        std::path::PathBuf::from("../../imports/b")
    );

    let status = strip_ansi(&env.run(&["ls"]).stdout);
    let row = status
        .lines()
        .find(|l| l.contains("imports/b"))
        .unwrap_or_default();
    assert!(row.contains("implicit via repoA"), "{}", status);
}

/// A checkout taken as an artefact has the dependencies its repository's
/// config declares, linked inside it like any checkout's.
#[test]
fn normal_006_an_artefacts_dependencies_are_linked_inside_it() {
    let env = TestEnv::new("recursive_artefact_config");
    let bare_dep = env.create_bare_repo("dep", "main", &[("dep.txt", "dep content")]);
    let bare_art = env.create_bare_repo("art", "main", &[("README.md", "art")]);
    let producer = format!(
        "[artefact]\ninclude = [\"dist/**\"]\n\n[repos]\n\"vendor/dep\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare_dep.display()
    );
    let released = env.push_commit(&bare_art, "main", ".gitscale.toml", &producer);
    run_git_pub(&bare_art, &["tag", "v1.0.0", &released]);
    let (published, _) = env.publish_with(
        &bare_art,
        "v1.0.0",
        &producer,
        &[("art.bin", "binary")],
        &["v1.0.0"],
    );
    assert!(
        published.success,
        "{}{}",
        published.stdout, published.stderr
    );

    // Root declares both artefact and the dep
    env.prefer(&bare_art, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        r#"{}[repos]
"meta/art" = {{ url = "{}", revision = "v1.0.0" }}
"dep" = {{ url = "{}", revision = "main" }}
"#,
        env.registries(),
        bare_art.display(),
        bare_dep.display(),
    ));

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // Symlink inside artefact dir
    let link = env.playground.join("meta/art/vendor/dep");
    assert!(link.is_symlink(), "expected symlink inside artefact");
    assert!(link.join("dep.txt").is_file());
}

#[test]
fn normal_007_a_dependency_raises_the_root_and_ls_and_explain_say_why() {
    let env = TestEnv::new("res_raise");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));

    let out = env.run(&["sync"]);
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
    let table = strip_ansi(&env.run(&["ls"]).stdout);
    assert!(
        table.lines().next().unwrap().ends_with("RESOLUTION"),
        "{}",
        table
    );
    assert!(
        table.contains("hint: git explain <dir> lists every request"),
        "{}",
        table
    );

    let why = env.run(&["explain", "imports/d"]);
    assert!(why.success, "{}", why.stderr);
    assert!(why.stdout.contains("selected  v1.5.0"), "{}", why.stdout);
    assert!(why.stdout.contains("highest semver"), "{}", why.stdout);
    assert!(
        why.stdout.contains("root → imports/b@v1.0.0"),
        "{}",
        why.stdout
    );

    let json = env.run(&["ls", "--format", "json"]);
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
fn normal_008_an_override_at_the_root_holds_a_dependency_down() {
    let env = TestEnv::new("res_override");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\", override = true"),
    ]));

    assert!(env.run(&["sync"]).success);
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
fn normal_009_two_majors_get_a_checkout_each_unless_one_is_a_singleton() {
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
    let out = env.run(&["sync"]);
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
    let out = fresh.run(&["sync"]);
    assert!(!out.success);
    assert!(out.stderr.contains("singleton"), "{}", out.stderr);
}

#[test]
fn normal_010_calendar_versions_order_by_date_then_modifier() {
    let env = TestEnv::new("res_calver");
    let d = tagged(
        &env,
        "d",
        &[
            ("v1-2026.09.30-rc1", ""),
            ("v1-2026.09.30", ""),
            ("v1-2026.09.30-2", ""),
            ("v1-2026.09.30-11", ""),
        ],
    );
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1-2026.09.30-11\"")]),
        )],
    );
    let c = tagged(
        &env,
        "c",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1-2026.09.30-2\"")]),
        )],
    );
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1-2026.09.30\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1-2026.09.30-11"));
    // The reason is the calendar order, not position: b and c are siblings.
    let why = env.run(&["explain", "imports/d"]);
    assert!(why.success, "{}", why.stderr);
    assert!(
        why.stdout.contains("highest calendar version"),
        "{}",
        why.stdout
    );
}

/// The dependencies of a checkout taken as an artefact are resolved from its
/// repository's config before anything is installed, checked out implicitly
/// and linked inside the artefact.
#[test]
fn normal_011_an_artefacts_dependencies_are_checked_out_implicitly() {
    let env = TestEnv::new("res_artefact_deps");
    let dep = tagged(&env, "dep", &[("v1.0.0", "")]);
    let art = env.create_bare_repo("art", "main", &[("README.md", "art")]);
    let producer = format!(
        "[artefact]\ninclude = [\"dist/**\"]\n\n{}",
        repos(&[("vendor/dep", &dep, ", revision = \"v1.0.0\"")])
    );
    env.push_commit(&art, "main", ".gitscale.toml", &producer);
    run_git_pub(&art, &["tag", "v1.0.0", "main"]);
    let (published, _) =
        env.publish_with(&art, "main", &producer, &[("app.bin", "x")], &["v1.0.0"]);
    assert!(
        published.success,
        "{}{}",
        published.stdout, published.stderr
    );

    env.prefer(&art, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[("meta/app", &art, ", revision = \"v1.0.0\"")])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/dep"), tag_commit(&dep, "v1.0.0"));
    let link = env.playground.join("meta/app/vendor/dep");
    assert!(link.is_symlink(), "linked inside the artefact");
    assert!(link.join("VERSION").is_file());
}

/// In CI, resolution reads each config from the commit the checkout is built
/// from, with no history fetched; and a move is never refused.
#[test]
fn normal_012_in_ci_resolution_reads_each_config_from_its_commit() {
    for cache in [true, false] {
        let env = TestEnv::new(&format!("res_ci_{}", cache));
        let (b, d) = implicit_d(&env);
        let mut args = vec!["sync"];
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
        let out = env.run_with_env(&[("CI", "true")], &["sync"]);
        assert!(out.success, "{}{}", out.stdout, out.stderr);
        let dest = env.playground.join("imports/d");
        set_writable(&dest.join("README.md"));
        std::fs::write(dest.join("README.md"), "left by the last job").unwrap();
        env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.5.0\"")]));
        let out = env.run_with_env(&[("CI", "true")], &["sync"]);
        assert!(out.success, "cache {}: {}{}", cache, out.stdout, out.stderr);
        assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    }
}

/// An override in a dependency: it wins over what that dependency is above,
/// and must agree with what it is not.
#[test]
fn normal_013_an_override_in_a_dependency_reaches_only_what_it_is_above() {
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
    let out = env.run(&["sync"]);
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
    let out = env.run(&["sync"]);
    assert!(!out.success);
    assert!(out.stderr.contains("override conflict"), "{}", out.stderr);
}

#[test]
fn normal_014_explain_shows_the_shared_checkouts_or_the_ones_named() {
    let env = TestEnv::new("res_why");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["sync"]).success);

    let out = env.run(&["explain"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.starts_with("imports/d  "), "{}", out.stdout);
    assert!(
        !out.stdout.contains("imports/b  "),
        "only shared checkouts: {}",
        out.stdout
    );

    let out = env.run(&["explain", "imports/b"]);
    assert!(out.stdout.starts_with("imports/b  "), "{}", out.stdout);
    assert!(out.stdout.contains("only request"), "{}", out.stdout);

    let out = env.run(&["explain", "nowhere"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("nowhere is not a checkout of this workspace"),
        "{}",
        out.stderr
    );
}

#[test]
fn normal_015_placement_follows_the_hoist_dir_majors_kind_and_names() {
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
    let out = env.run(&["sync"]);
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
    let out = env.run(&["sync"]);
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
    let out = env.run(&["sync"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("two repositories want imports/shared"),
        "{}",
        out.stderr
    );
}

/// A root override settles a singleton that two dependencies want at two
/// majors: one checkout, at the override, the other request noted as held.
/// Without it the singleton refuses; this is the way out the docs give.
#[test]
fn normal_028_an_override_at_the_root_settles_a_singletons_two_majors() {
    let env = TestEnv::new("resolution_singleton_override");
    let d = tagged(&env, "d", &[("v1.5.0", ""), ("v2.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v2.0.0\"")]);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        (
            "imports/d",
            &d,
            ", revision = \"v1.5.0\", override = true, singleton = true",
        ),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    assert!(!env.playground.join("imports/d_v2").exists());
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/c/libs/d")).unwrap(),
        PathBuf::from("../../d")
    );
    let row = status_row(&env, "imports/d");
    assert!(
        row.contains("held at v1.5.0, imports/c wants v2.0.0"),
        "{}",
        row
    );
}

/// The root may declare each major of one repository itself, under names of
/// its own: each dependency is linked to the checkout of the major it asked
/// for, and nothing is hoisted.
#[test]
fn normal_029_the_root_declares_each_major_under_its_own_name() {
    let env = TestEnv::new("resolution_root_majors");
    let d = tagged(&env, "d", &[("v1.2.0", ""), ("v1.5.0", ""), ("v2.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.2.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v2.0.0\"")]);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/new", &d, ", revision = \"v2.0.0\""),
        ("imports/old", &d, ", revision = \"v1.5.0\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/old"), tag_commit(&d, "v1.5.0"));
    assert_eq!(head(&env, "imports/new"), tag_commit(&d, "v2.0.0"));
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/b/libs/d")).unwrap(),
        PathBuf::from("../../old")
    );
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/c/libs/d")).unwrap(),
        PathBuf::from("../../new")
    );
    assert!(!env.playground.join("imports/d").exists());
}

/// A root entry with no revision for a repository its dependencies want at
/// two majors is the lowest of them; the other is placed beside it.
#[test]
fn normal_030_a_root_entry_without_a_revision_takes_the_lowest_major() {
    let env = TestEnv::new("resolution_unpinned_majors");
    let d = tagged(&env, "d", &[("v1.5.0", ""), ("v2.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v2.0.0\"")]);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ""),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    assert_eq!(head(&env, "imports/d_v2"), tag_commit(&d, "v2.0.0"));
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/c/libs/d")).unwrap(),
        PathBuf::from("../../d_v2")
    );
}

/// Tags of two streams of one monorepo are not compared as versions: the
/// request from the repository above the other wins, and two siblings
/// cannot be ordered at all.
#[test]
fn normal_031_two_streams_of_one_monorepo_are_ordered_by_position() {
    let env = TestEnv::new("resolution_streams");
    let mono = tagged(&env, "mono", &[("api-v1.4.0", ""), ("web-v1.0.0", "")]);
    let b = dependant(
        &env,
        "b",
        &[("libs/mono", &mono, ", revision = \"web-v1.0.0\"")],
    );
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/mono", &mono, ", revision = \"api-v1.4.0\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/mono"), tag_commit(&mono, "api-v1.4.0"));
    let why = env.run(&["explain", "imports/mono"]);
    assert!(
        why.stdout.contains("selected  api-v1.4.0")
            && why
                .stdout
                .contains("asked for by a repository above the other"),
        "{}",
        why.stdout
    );

    // Siblings asking for one stream each: nothing orders them.
    let c = dependant(
        &env,
        "c",
        &[("libs/mono", &mono, ", revision = \"api-v1.4.0\"")],
    );
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/mono", &mono, ""),
    ]));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("cannot order"), "{}", out.stderr);
}

/// A repository that moved from semver to calendar versions at its next
/// major, asked for in both: two majors, so each gets a checkout rather than
/// an error.
#[test]
fn normal_032_semver_and_calendar_versions_of_one_repository_get_a_checkout_each() {
    let env = TestEnv::new("resolution_semver_calver");
    let d = tagged(&env, "d", &[("v1.5.0", ""), ("v2-2026.10.01", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    let c = dependant(
        &env,
        "c",
        &[("libs/d", &d, ", revision = \"v2-2026.10.01\"")],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    assert_eq!(head(&env, "imports/d_v2"), tag_commit(&d, "v2-2026.10.01"));
    assert!(status_row(&env, "imports/d").contains("2 majors"));
}

/// A pre-release is above the version before it and below its own release,
/// as semver orders them.
#[test]
fn normal_033_a_pre_release_beats_the_version_before_it_and_loses_to_its_release() {
    let env = TestEnv::new("resolution_prerelease");
    let d = tagged(
        &env,
        "d",
        &[("v1.4.0", ""), ("v1.5.0-rc.1", ""), ("v1.5.0", "")],
    );
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.5.0-rc.1\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.4.0\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0-rc.1"));

    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.4.0\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    let why = env.run(&["explain", "imports/d"]);
    assert!(why.stdout.contains("highest semver"), "{}", why.stdout);
}

/// The root repository's own origin allows implicit dependencies from its
/// host and owner, with no `[resolve] allow`: a workspace of one
/// organisation needs no allowlist for that organisation's repositories.
#[test]
fn normal_034_the_roots_origin_allows_implicit_dependencies_from_its_owner() {
    let env = TestEnv::new("resolution_origin_allows");
    let d = tagged(&env, "d", &[("v1.5.0", "")]);
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            "[repos]\n\"libs/d\" = { url = \"https://example.com/org/d.git\", revision = \"v1.5.0\" }\n",
        )],
    );
    env.write_config(&repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")]));
    // Only d's URL is sent here: `git remote get-url` rewrites too, and the
    // root's origin must stay the URL it is.
    let vars: Vec<(String, String)> = vec![
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        (
            "GIT_CONFIG_KEY_0".into(),
            format!("url.{}.insteadOf", d.display()),
        ),
        (
            "GIT_CONFIG_VALUE_0".into(),
            "https://example.com/org/d.git".into(),
        ),
    ];

    let out = env.run_with_env(&borrowed(&vars), &["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("not on the allowlist") && out.stderr.contains("example.com/org/*"),
        "{}",
        out.stderr
    );

    env.set_playground_origin("https://example.com/org/root.git");
    let out = env.run_with_env(&borrowed(&vars), &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
}

/// The SSH and HTTPS spellings of one repository are one repository: a
/// dependency asking for it over SSH raises the root's HTTPS entry and is
/// linked to that one checkout, instead of getting a second checkout of it.
#[test]
fn normal_035_ssh_and_https_spellings_of_one_repository_share_a_checkout() {
    let env = TestEnv::new("resolution_url_spellings");
    let d = tagged(&env, "d", &[("v1.2.0", ""), ("v1.5.0", "")]);
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            "[repos]\n\"libs/d\" = { url = \"git@example.com:org/d.git\", revision = \"v1.5.0\" }\n",
        )],
    );
    env.write_config(&format!(
        "[repos]\n\"imports/b\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n\
         \"imports/d\" = {{ url = \"https://example.com/org/d.git\", revision = \"v1.2.0\" }}\n",
        b.display()
    ));
    let vars = rewritten_to_local(&env, &["https://example.com/org/", "git@example.com:org/"]);
    let out = env.run_with_env(&borrowed(&vars), &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/b/libs/d")).unwrap(),
        PathBuf::from("../../d")
    );
    let mut checkouts: Vec<String> = std::fs::read_dir(env.playground.join("imports"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    checkouts.sort();
    assert_eq!(checkouts, vec!["b", "d"]);

    let out = env.run_with_env(&borrowed(&vars), &["ls"]);
    let table = strip_ansi(&out.stdout);
    let row = table.lines().find(|l| l.contains("imports/d")).unwrap();
    assert!(row.contains("raised from v1.2.0 by imports/b"), "{}", table);
}

/// An implicit checkout takes the form the workspace prefers, as a declared
/// one does: here the artefact of its release.
#[test]
fn normal_036_an_implicit_checkout_takes_the_workspaces_preference() {
    let env = TestEnv::new("resolution_implicit_artefact");
    let art = env.artefact_repo("art", &[("art.bin", "built")]);
    let b = dependant(&env, "b", &[("libs/art", &art, ", revision = \"v1.0.0\"")]);
    let c = dependant(&env, "c", &[("libs/art", &art, ", revision = \"v1.0.0\"")]);
    env.prefer(&art, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    let dest = env.playground.join("imports/art");
    assert!(!dest.join(".git").exists(), "not a source checkout");
    assert!(dest.join("dist/art.bin").is_file(), "the image");
}

/// `git explain` with nothing named and no checkout more than one
/// repository asks for says so, rather than printing nothing.
#[test]
fn normal_056_explain_says_when_no_checkout_is_shared() {
    let env = TestEnv::new("resolution_why_unshared");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    env.write_config(&repos(&[("imports/d", &d, ", revision = \"v1.0.0\"")]));
    assert!(env.run(&["sync"]).success);
    let out = env.run(&["explain"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        out.stdout
            .contains("Every dependency is asked for by one repository only."),
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn edge_016_recursive_false_leaves_a_dependencys_config_unread() {
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

    let out = env.run(&["sync"]);
    assert!(
        out.success,
        "should succeed because recursion is disabled: stderr: {}",
        out.stderr
    );

    // No symlink should be created
    assert!(!env.playground.join("repoA/libs/b").exists());
}

/// An artefact has no git checkout to move: a revision a dependency asks for
/// must not send `git checkout` up into the workspace repo.
#[test]
fn edge_017_a_revision_a_dependency_asks_for_leaves_the_workspace_repo_alone() {
    let env = TestEnv::new("dep_rev_artefact");
    env.init_playground_git();
    let root_head = git_stdout(&env.playground, &["rev-parse", "HEAD"]);
    // No revision: the default branch's commit, whatever the branch is called.
    let bare_art = env.artefact_repo("art", &[("art.bin", "binary")]);
    let bare_a = env.create_bare_repo(
        "repoA",
        "main",
        &[
            ("a.txt", "A"),
            (
                ".gitscale.toml",
                &format!(
                    "[repos]\n\"libs/art\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
                    bare_art.display()
                ),
            ),
        ],
    );
    env.prefer(&bare_art, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        r#"{}[repos]
"repoA" = {{ url = "{}", revision = "main" }}
"meta/art" = {{ url = "{}" }}
"#,
        env.registries(),
        bare_a.display(),
        bare_art.display(),
    ));

    let out = env.run(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(env.playground.join("meta/art/dist/art.bin").is_file());
    assert_eq!(
        git_stdout(&env.playground, &["rev-parse", "HEAD"]),
        root_head
    );
}

/// A dependency pinned to a revision its repository does not have fails
/// resolution — unless the root overrides that dependency, which is how it
/// works around a broken pin it cannot edit.
#[test]
fn edge_018_an_override_from_above_works_around_a_missing_revision() {
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
    let out = env.run(&["sync"]);
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
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.2.0"));
    let row = status_row(&env, "imports/d");
    assert!(
        row.ends_with("held at v1.2.0, imports/b wants develop (no such revision), 2 requests"),
        "{}",
        row
    );

    // `git explain` marks the override, the winner and the broken pin it
    // overruled.
    let why = env.run(&["explain", "imports/d"]);
    assert!(why.success, "{}", why.stderr);
    let line = |needle: &str| {
        why.stdout
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or_default()
            .to_string()
    };
    let root = line("v1.2.0  ");
    assert!(
        root.contains("override") && root.contains("selected"),
        "{}",
        why.stdout
    );
    let broken = line("develop");
    assert!(
        broken.contains("no such revision") && broken.contains("overruled by root"),
        "{}",
        why.stdout
    );
    // So does the JSON output.
    let json = env.run(&["ls", "--format", "json"]);
    let rows: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    let requests = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "imports/d")
        .unwrap()["requests"]
        .as_array()
        .unwrap()
        .clone();
    let from_b = requests
        .iter()
        .find(|r| r["revision"] == "develop")
        .unwrap();
    assert_eq!(from_b["missing"], true, "{:#}", from_b);
    assert_eq!(from_b["overruled_by"], "root", "{:#}", from_b);
}

/// Offline, `ls` reads what is on this machine: a repository nothing has
/// fetched yet is unresolved until `ls --fetch`. In CI, where checkouts hold
/// no history, that is the light stores resolution keeps.
#[test]
fn edge_019_ls_is_unresolved_until_fetched() {
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
    assert!(env.run_with_env(&ci, &["sync", "--no-cache"]).success);
    // The workspace's own stores go; the checkouts stay.
    std::fs::remove_dir_all(env.playground.join(".git/gitscale/resolve")).unwrap();
    let out = env.run_with_env(&ci, &["ls"]);
    let table = strip_ansi(&out.stdout);
    let row = table.lines().find(|l| l.contains("imports/d")).unwrap();
    assert!(row.contains("unresolved"), "{}", table);
    assert!(
        row.contains("not fetched yet: run git scale ls --fetch"),
        "{}",
        table
    );

    let out = env.run_with_env(&ci, &["ls", "--fetch", "--no-cache"]);
    let table = strip_ansi(&out.stdout);
    let row = table.lines().find(|l| l.contains("imports/d")).unwrap();
    assert!(!row.contains("unresolved"), "{}", table);
    assert!(row.contains("raised from v1.2.0 by imports/b"), "{}", table);
}

/// A dependency's checkout at the selected commit is read from disk, so an
/// edit to its `.gitscale.toml` not yet committed takes effect at once.
#[test]
fn edge_020_an_uncommitted_config_edit_takes_effect() {
    let env = TestEnv::new("res_worktree_config");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["sync"]).success);
    assert!(status_row(&env, "imports/d").contains("v1.5.0"));

    // b now asks for no more than the root does.
    support::edit(
        &env.playground.join("imports/b/.gitscale.toml"),
        &repos(&[("libs/d", &d, ", revision = \"v1.2.0\"")]),
    );
    let row = status_row(&env, "imports/d");
    let expected = row.split_whitespace().nth(5).unwrap_or_default();
    assert_eq!(expected, "v1.2.0", "{}", row);
    assert!(row.contains("ref-mismatch"), "{}", row);
}

/// How the workspace takes a repository plays no part in resolution: two
/// requests for it are one checkout, the image when the workspace prefers
/// its artefact, and both requesters link to it.
#[test]
fn edge_021_one_checkout_whatever_form_the_workspace_takes_it_in() {
    let env = TestEnv::new("res_place_artefact");
    let d = env.artefact_repo("d", &[("d.bin", "built")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    env.prefer(&d, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        "{}{}{}",
        env.registries(),
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        env.playground.join("imports/d/dist/d.bin").is_file(),
        "the build"
    );
    assert!(!env.playground.join("imports/d/.git").exists(), "no source");
    for requester in ["imports/b", "imports/c"] {
        assert_eq!(
            std::fs::read_link(env.playground.join(requester).join("libs/d")).unwrap(),
            PathBuf::from("../../d")
        );
    }
}

/// Where the root's store holds history, a winner by position that is behind
/// what a losing request asked for is flagged — and still wins.
#[test]
fn edge_022_a_winner_behind_a_request_is_flagged_where_history_is_local() {
    let env = TestEnv::new("res_behind");
    let d = tagged(&env, "d", &[("v1.3.1", ""), ("v1.5.0", "")]);
    run_git_pub(&d, &["branch", "stable", "v1.3.1"]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    env.write_config(&repos(&[
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"stable\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.3.1"));
    let row = status_row(&env, "imports/d");
    assert!(row.starts_with('↧'), "{}", row);
    assert!(row.contains("behind imports/c's v1.5.0"), "{}", row);
}

/// A row with nothing on disk says `missed` and nothing more, even where
/// resolution has a story to tell.
#[test]
fn edge_023_a_missed_row_has_no_resolution_text() {
    let env = TestEnv::new("res_missed_quiet");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["sync"]).success);
    std::fs::remove_dir_all(env.playground.join("imports/d")).unwrap();
    let row = status_row(&env, "imports/d");
    assert!(row.ends_with("missed"), "{}", row);
}

/// Two requests naming one commit agree, whatever they name — here a branch
/// and a tag from two siblings, which position alone could not order.
#[test]
fn edge_037_two_requests_naming_one_commit_agree_whatever_they_name() {
    let env = TestEnv::new("resolution_same_commit");
    let d = tagged(&env, "d", &[("v1.5.0", "")]);
    run_git_pub(&d, &["branch", "stable", "v1.5.0"]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"stable\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    let why = env.run(&["explain", "imports/d"]);
    assert!(why.stdout.contains("same commit"), "{}", why.stdout);
    // The root asked for nothing: the highest request is what it gets.
    let json = env.run(&["ls", "--format", "json"]);
    let rows: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "imports/d")
        .unwrap();
    assert_eq!(row["resolution"], "highest", "{:#}", row);
}

/// A new, lower major takes the plain name, and the checkout there moves to
/// it — but never at the cost of work: a commit on no branch stops the move,
/// and once it is gone the old major lands beside it.
#[test]
fn edge_038_a_new_lower_major_takes_the_plain_name_without_losing_work() {
    let env = TestEnv::new("resolution_lower_major");
    let d = tagged(&env, "d", &[("v1.5.0", ""), ("v2.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v2.0.0\"")]);
    let config = |entries: &[(&str, &Path, &str)]| format!("{}{}", allow(&env), repos(entries));
    env.write_config(&config(&[("imports/c", &c, ", revision = \"v1.0.0\"")]));
    assert!(env.run(&["sync"]).success);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v2.0.0"));

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

    env.write_config(&config(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
    ]));
    let out = env.run(&["sync"]);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(!out.success, "{}", text);
    assert!(text.contains("is on no branch"), "{}", text);
    assert_eq!(head(&env, "imports/d"), work);

    run_git_pub(
        &dest,
        &["checkout", "--quiet", "--detach", &tag_commit(&d, "v2.0.0")],
    );
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.5.0"));
    assert_eq!(head(&env, "imports/d_v2"), tag_commit(&d, "v2.0.0"));
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/c/libs/d")).unwrap(),
        PathBuf::from("../../d_v2")
    );
}

/// A name that is both a branch and a tag is the branch, as git has it;
/// `refs/tags/` and `refs/heads/` choose explicitly.
#[test]
fn edge_039_a_branch_beats_a_tag_of_the_same_name_and_ref_prefixes_choose() {
    let env = TestEnv::new("resolution_ref_prefixes");
    let d = env.create_bare_repo("d", "main", &[("README.md", "d")]);
    let at_tag = env.push_commit(&d, "main", "VERSION", "tagged");
    run_git_pub(&d, &["tag", "stable", &at_tag]);
    let at_branch = env.push_commit(&d, "main", "VERSION", "branch");
    run_git_pub(&d, &["branch", "stable", &at_branch]);
    for (revision, expected) in [
        ("stable", &at_branch),
        ("refs/tags/stable", &at_tag),
        ("refs/heads/stable", &at_branch),
    ] {
        env.write_config(&repos(&[(
            "imports/d",
            &d,
            &format!(", revision = \"{}\"", revision),
        )]));
        let out = env.run(&["sync"]);
        assert!(out.success, "{}: {}{}", revision, out.stdout, out.stderr);
        assert_eq!(&head(&env, "imports/d"), expected, "{}", revision);
    }
}

/// Only a tag is read as a version: a branch called `v9.0.0` is a branch, so
/// it joins the checkout of the versions asked for instead of making a
/// major 9 of its own.
#[test]
fn edge_040_a_branch_named_like_a_version_is_a_branch() {
    let env = TestEnv::new("resolution_version_branch");
    let d = tagged(&env, "d", &[("v1.5.0", "")]);
    let ahead = env.push_commit(&d, "main", "VERSION", "ahead");
    run_git_pub(&d, &["branch", "v9.0.0", &ahead]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v9.0.0\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), ahead);
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/b/libs/d")).unwrap(),
        PathBuf::from("../../d")
    );
    assert!(!env.playground.join("imports/d_v1").exists());
    assert!(!env.playground.join("imports/d_v9").exists());
    let why = env.run(&["explain", "imports/d"]);
    assert!(
        why.stdout
            .contains("branch, asked for by a repository above the other"),
        "{}",
        why.stdout
    );
    assert!(!status_row(&env, "imports/d").contains("majors"));
}

/// An implicit checkout's own dependencies are read unless every request
/// for it says `recursive = false`.
#[test]
fn edge_041_an_implicit_checkout_reads_its_dependencies_unless_every_request_says_not() {
    let env = TestEnv::new("resolution_implicit_recursive");
    let e = tagged(&env, "e", &[("v1.0.0", "")]);
    let d = tagged(
        &env,
        "d",
        &[(
            "v1.0.0",
            &repos(&[("libs/e", &e, ", revision = \"v1.0.0\"")]),
        )],
    );
    let b = dependant(
        &env,
        "b",
        &[("libs/d", &d, ", revision = \"v1.0.0\", recursive = false")],
    );
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let config = |entries: &[(&str, &Path, &str)]| format!("{}{}", allow(&env), repos(entries));

    env.write_config(&config(&[("imports/b", &b, ", revision = \"v1.0.0\"")]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/d/VERSION").is_file());
    assert!(!env.playground.join("imports/e").exists());
    assert!(std::fs::symlink_metadata(env.playground.join("imports/d/libs/e")).is_err());

    env.write_config(&config(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/e/VERSION").is_file());
    assert!(env.playground.join("imports/d/libs/e").is_symlink());
}

/// An implicit checkout of a calendar major whose plain name is the root's
/// checkout of another major is placed beside it, with its major's suffix.
#[test]
fn edge_042_an_implicit_calendar_major_beside_a_root_major_takes_its_suffix() {
    let env = TestEnv::new("resolution_any_suffix");
    let d = tagged(&env, "d", &[("v2.0.0", ""), ("v1-2026.10.01", "")]);
    let b = dependant(
        &env,
        "b",
        &[("libs/mylib", &d, ", revision = \"v1-2026.10.01\"")],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/mylib", &d, ", revision = \"v2.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/mylib"), tag_commit(&d, "v2.0.0"));
    assert_eq!(
        head(&env, "imports/mylib_v1"),
        tag_commit(&d, "v1-2026.10.01")
    );
    assert_eq!(
        std::fs::read_link(env.playground.join("imports/b/libs/mylib")).unwrap(),
        PathBuf::from("../../mylib_v1")
    );
}

/// A chain seventy deep — deeper than the rounds resolution allows itself —
/// still settles: a graph without cycles always does.
#[test]
#[ignore = "bug: a chain 64 or more deep fails 'did not settle after 64 rounds' (MAX_ROUNDS)"]
fn edge_053_a_chain_seventy_deep_settles() {
    let env = TestEnv::new("resolution_very_deep");
    let chain = chain(&env, 70);
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[("imports/c000", &chain[0], ", revision = \"v1.0.0\"")])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(env.playground.join("imports/c069/VERSION").is_file());
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_024_two_branches_nothing_orders_fail_the_sync() {
    let env = TestEnv::new("recursive_revision_conflict");

    let bare_c = env.create_bare_repo("repoC", "main", &[("c.txt", "C")]);
    bare_git_stdout(&bare_c, &["branch", "develop", "main"]);
    commit_to_bare(&bare_c, "develop", "c.txt", "C develop");

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

    // Two branches are not versions, and neither A nor B is above the
    // other: without history to consult, nothing orders them.
    let out = env.run(&["sync"]);
    assert!(!out.success, "should fail on conflicting revisions");
    assert!(
        out.stderr.contains("cannot order"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn error_025_a_cycle_fails_before_anything_is_cloned() {
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
    let out = env.run(&["sync"]);
    assert!(!out.success);
    assert!(out.stderr.contains("cycle"), "{}", out.stderr);
    assert!(!env.playground.join("imports").exists());
    // The message names the loop and what to do about it.
    assert!(
        out.stderr.contains("imports/b@v1.0.0") && out.stderr.contains("imports/c@v1.0.0"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("Remove the dependency from the repository that closes the loop"),
        "{}",
        out.stderr
    );
}

#[test]
fn error_026_bad_resolution_config_is_refused_when_read() {
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
        let out = env.run(&["ls"]);
        assert!(!out.success, "{}", config);
        assert!(out.stderr.contains(message), "{}: {}", message, out.stderr);
    }
}

/// A repository that lists itself as a dependency is a cycle, refused
/// before anything is cloned.
#[test]
fn error_043_a_repository_depending_on_itself_is_a_cycle() {
    let env = TestEnv::new("resolution_self_cycle");
    let b_url = env.repos_remote.join("b.git");
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/self", &b_url, ", revision = \"v1.0.0\"")]),
        )],
    );
    env.write_config(&repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")]));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("cycle: imports/b@v1.0.0 depends on itself"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());
}

/// A dependency that asks for the root repository — known by its origin —
/// closes a loop through the root, and is refused like any cycle rather than
/// checking the root out inside itself.
#[test]
fn error_044_a_dependency_on_the_root_repository_is_a_cycle() {
    let env = TestEnv::new("resolution_root_cycle");
    let root = tagged(&env, "root", &[("v1.0.0", "")]);
    env.set_playground_origin(root.to_str().unwrap());
    let b = dependant(
        &env,
        "b",
        &[("libs/root", &root, ", revision = \"v1.0.0\"")],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")])
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("cycle: ")
            && out.stderr.contains("imports/b@v1.0.0")
            && out.stderr.contains("root"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("Remove the dependency from the repository that closes the loop"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());
}

/// Two overrides at different commits, neither from a repository above the
/// other, are a conflict only an override above both can settle.
#[test]
fn error_045_conflicting_overrides_from_siblings_fail() {
    let env = TestEnv::new("resolution_conflicting_overrides");
    let d = tagged(&env, "d", &[("v1.2.0", ""), ("v1.5.0", "")]);
    let b = dependant(
        &env,
        "b",
        &[("libs/d", &d, ", revision = \"v1.5.0\", override = true")],
    );
    let c = dependant(
        &env,
        "c",
        &[("libs/d", &d, ", revision = \"v1.2.0\", override = true")],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("conflicting overrides"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());
}

/// A revision a dependency's repository does not have is covered only by an
/// override from a repository above that dependency; a sibling's override
/// does not excuse it.
#[test]
fn error_046_a_missing_revision_is_not_covered_by_a_siblings_override() {
    let env = TestEnv::new("resolution_missing_sibling");
    let d = tagged(&env, "d", &[("v1.2.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"develop\"")]);
    let c = dependant(
        &env,
        "c",
        &[("libs/d", &d, ", revision = \"v1.2.0\", override = true")],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("'develop' is not a branch, tag or commit"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("root → imports/b@v1.0.0"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());
}

/// Pairwise wins that go round in a circle settle nothing: x above y asks
/// for v1.5.0 over y's `main`, y above z asks for `main` over z's v1.6.0,
/// and v1.6.0 beats v1.5.0. No request is at least every other, so
/// resolution fails rather than letting the order requests were read in pick
/// the winner.
#[test]
fn error_047_requests_whose_pairwise_winners_go_round_in_a_circle_fail() {
    let env = TestEnv::new("resolution_intransitive");
    let d = tagged(&env, "d", &[("v1.5.0", ""), ("v1.6.0", "")]);
    env.push_commit(&d, "main", "VERSION", "next");
    let z = dependant(&env, "z", &[("libs/d", &d, ", revision = \"v1.6.0\"")]);
    let y = dependant(
        &env,
        "y",
        &[
            ("libs/d", &d, ", revision = \"main\""),
            ("libs/z", &z, ", revision = \"v1.0.0\""),
        ],
    );
    let x = dependant(
        &env,
        "x",
        &[
            ("libs/d", &d, ", revision = \"v1.5.0\""),
            ("libs/y", &y, ", revision = \"v1.0.0\""),
        ],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[("imports/x", &x, ", revision = \"v1.0.0\"")])
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("cannot order"), "{}", out.stderr);
    // Whichever request the pairs end on, some other request beats it: the
    // refusal names two requests by their paths, y's `main` among them.
    assert!(
        out.stderr.contains("wants d main against")
            || out
                .stderr
                .contains("against root → imports/x@v1.0.0 → imports/y@v1.0.0 wants d main"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());
}

/// Two root entries of one repository in one major are refused: they would
/// be one checkout under two names.
#[test]
fn error_048_two_root_entries_of_one_major_are_refused() {
    let env = TestEnv::new("resolution_two_root_entries");
    let d = tagged(&env, "d", &[("v1.2.0", ""), ("v1.5.0", "")]);
    env.write_config(&repos(&[
        ("imports/d", &d, ", revision = \"v1.2.0\""),
        ("imports/d2", &d, ", revision = \"v1.5.0\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("are both checkouts of"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());
}

/// A branch asked for while the repository is checked out at two majors
/// names no checkout in particular, and is refused with what to ask for
/// instead.
#[test]
fn error_049_a_branch_asked_for_beside_two_majors_is_refused() {
    let env = TestEnv::new("resolution_branch_two_majors");
    let d = tagged(&env, "d", &[("v1.5.0", ""), ("v2.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.5.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v2.0.0\"")]);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"main\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("which is not a version")
            && out.stderr.contains("Ask for a version instead"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());
}

/// A full SHA is not checked by name: one the repository does not have
/// fails when its config is read, saying which commit could not be fetched.
#[test]
fn error_050_a_missing_commit_in_a_dependency_fails_reading_its_config() {
    let env = TestEnv::new("resolution_missing_sha");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = dependant(
        &env,
        "b",
        &[(
            "libs/d",
            &d,
            ", revision = \"0123456789abcdef0123456789abcdef01234567\"",
        )],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")])
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("cannot fetch .gitscale.toml at 0123456"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());
}

/// A dependency's config that is not valid TOML fails resolution, naming
/// the file by the checkout and revision it was read at.
#[test]
fn error_054_an_invalid_dependency_config_names_where_it_was_read() {
    let env = TestEnv::new("resolution_invalid_dependency_config");
    let b = tagged(&env, "b", &[("v1.0.0", "[repos\n\"libs/d\" = 1\n")]);
    env.write_config(&repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")]));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("imports/b@v1.0.0:.gitscale.toml")
            && out.stderr.contains("invalid TOML"),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("imports").exists());
}

/// A cycle is refused even when it exists only at a revision that loses:
/// c v1.0.0 asks for b, c v1.1.0 does not, and b raises c to v1.1.0. The
/// check covers every revision resolution considers, which is what
/// guarantees it ends.
#[test]
fn error_055_a_cycle_at_a_revision_that_loses_still_fails() {
    let env = TestEnv::new("resolution_losing_cycle");
    let b_url = env.repos_remote.join("b.git");
    let c = tagged(
        &env,
        "c",
        &[
            (
                "v1.0.0",
                &repos(&[("libs/b", &b_url, ", revision = \"v1.0.0\"")]),
            ),
            ("v1.1.0", "[repos]\n"),
        ],
    );
    let b = dependant(&env, "b", &[("libs/c", &c, ", revision = \"v1.1.0\"")]);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/c", &c, ", revision = \"v1.0.0\""),
    ]));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("cycle: "), "{}", out.stderr);
    assert!(!env.playground.join("imports").exists());
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

/// In CI without the cache, resolution reads refs from `ls-remote` and each
/// config from one commit fetched at depth 1: no history crosses the wire.
#[test]
fn perf_027_resolving_in_ci_without_the_cache_fetches_no_history() {
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
    let out = env.run_with_env(&[("CI", "true")], &["sync", "--no-cache"]);
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

    // Offline afterwards: `ls` reads what is on disk.
    let out = env.run_with_env(&[("CI", "true")], &["ls"]);
    let table = strip_ansi(&out.stdout);
    let row = table.lines().find(|l| l.contains("imports/d")).unwrap();
    assert!(row.contains("implicit via imports/b"), "{}", row);
}

/// A wide graph — eight dependencies, each asking for a shared one at a
/// different version — fetches every repository's store exactly once per
/// command, however many requests and resolution rounds there are, and ends
/// with one checkout of the shared one at the highest version.
#[test]
fn perf_051_a_wide_graph_fetches_each_repository_once() {
    const N: usize = 8;
    let env = TestEnv::new("resolution_wide");
    let versions: Vec<String> = (0..N).map(|i| format!("v1.{}.0", i)).collect();
    let d = tagged(
        &env,
        "d",
        &versions
            .iter()
            .map(|v| (v.as_str(), ""))
            .collect::<Vec<_>>(),
    );
    let deps: Vec<PathBuf> = versions
        .iter()
        .enumerate()
        .map(|(i, v)| {
            dependant(
                &env,
                &format!("dep{}", i),
                &[("libs/d", &d, &format!(", revision = \"{}\"", v))],
            )
        })
        .collect();
    let dirs: Vec<String> = (0..N).map(|i| format!("imports/dep{}", i)).collect();
    let entries: Vec<(&str, &Path, &str)> = dirs
        .iter()
        .zip(&deps)
        .map(|(dir, dep)| (dir.as_str(), dep.as_path(), ", revision = \"v1.0.0\""))
        .collect();
    env.write_config(&format!("{}{}", allow(&env), repos(&entries)));

    for run in ["first", "second"] {
        let (out, argvs) = traced(&env, &[], &["sync"]);
        assert!(out.success, "{} pull: {}{}", run, out.stdout, out.stderr);
        for repo in deps.iter().chain(std::iter::once(&d)) {
            assert_eq!(
                store_fetches(&argvs, repo),
                1,
                "{} pull, {}: {:#?}",
                run,
                repo.display(),
                fetches(&argvs)
            );
        }
        assert_eq!(fetches(&argvs).len(), N + 1, "{} pull", run);
    }
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1.7.0"));
    for dir in &dirs {
        assert_eq!(
            std::fs::read_link(env.playground.join(dir).join("libs/d")).unwrap(),
            PathBuf::from("../../d"),
            "{}",
            dir
        );
    }
}

/// A chain ten deep settles — resolution takes a round per level — while
/// still fetching each repository's store exactly once.
#[test]
fn perf_052_a_deep_chain_fetches_each_repository_once() {
    let env = TestEnv::new("resolution_deep");
    let chain = chain(&env, 10);
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[("imports/c000", &chain[0], ", revision = \"v1.0.0\"")])
    ));
    let (out, argvs) = traced(&env, &[], &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    for repo in &chain {
        assert_eq!(
            store_fetches(&argvs, repo),
            1,
            "{}: {:#?}",
            repo.display(),
            fetches(&argvs)
        );
    }
    assert_eq!(fetches(&argvs).len(), chain.len());
    assert!(env.playground.join("imports/c009/VERSION").is_file());
    assert!(env.playground.join("imports/c008/libs/c009").is_symlink());
}

/// Calendar versions of two majors are two checkouts, as semver majors are.
#[test]
fn normal_057_calendar_majors_get_a_checkout_each() {
    let env = TestEnv::new("resolution_calver_majors");
    let d = tagged(
        &env,
        "d",
        &[("v1-2026.10.01-1", ""), ("v2-2026.11.02-1", "")],
    );
    let b = dependant(
        &env,
        "b",
        &[("libs/d", &d, ", revision = \"v1-2026.10.01-1\"")],
    );
    let c = dependant(
        &env,
        "c",
        &[("libs/d", &d, ", revision = \"v2-2026.11.02-1\"")],
    );
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(head(&env, "imports/d"), tag_commit(&d, "v1-2026.10.01-1"));
    assert_eq!(
        head(&env, "imports/d_v2"),
        tag_commit(&d, "v2-2026.11.02-1")
    );
}

/// A tag with a prefix is not a version, so two of them asked for by
/// siblings cannot be ordered, however their numbers compare.
#[test]
fn error_058_prefixed_tags_from_siblings_cannot_be_ordered() {
    let env = TestEnv::new("resolution_prefixed_tags");
    let d = tagged(&env, "d", &[("api-v1.4.0", ""), ("api-v1.5.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"api-v1.4.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"api-v1.5.0\"")]);
    env.write_config(&format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("cannot order api-v1.4.0 against api-v1.5.0"),
        "{}",
        out.stderr
    );
}
