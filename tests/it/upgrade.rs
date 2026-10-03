//! `gitscale upgrade`: raising pins and recording what resolution chose.

use crate::support;
use crate::support::resolution::*;
use crate::support::worktrees::*;
use crate::support::{run_git_pub, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_resolved_records_the_resolved_revisions_and_keeps_comments() {
    let env = TestEnv::new("res_write");
    let (b, d) = diamond(&env);
    env.write_config(&format!(
        "# the workspace\n[repos]\n\"imports/b\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n\
         \"imports/d\" = {{ url = \"{}\", revision = \"v1.2.0\" }} # raised by b\n",
        b.display(),
        d.display()
    ));

    let before = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    let out = env.run(&["upgrade", "--resolved", "--dry-run"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("v1.2.0 → v1.5.0"), "{}", out.stdout);
    assert!(
        out.stdout.contains("1 entry would change"),
        "{}",
        out.stdout
    );
    // A dry run leaves the file exactly as it was.
    assert_eq!(
        std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap(),
        before
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

/// Once a release tag holds a developed child's change, `upgrade` writes the
/// tag into the root's config, deletes the child's topic branch and detaches
/// it at the tag.
#[test]
fn normal_002_promotes_a_child_whose_change_is_released() {
    let f = fixture("wt_promote", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    let out = gs(&ws, &["push"]);
    ok(&out);

    // Merged upstream, squashed, and released.
    let released = f.core_commit("main", "lib.txt", "v2");
    run_git_pub(&f.core, &["tag", "v1.1.0", &released]);
    run_git_pub(&f.core, &["branch", "-D", "feat/x"]);

    let out = gs(&ws, &["upgrade", "--commit"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core  promoted → v1.1.0"),
        "{}",
        out.stdout
    );
    // With its one dependency promoted, the root is what merges next.
    assert!(out.stdout.contains("next to merge: ."), "{}", out.stdout);
    let config = std::fs::read_to_string(ws.join(".gitscale.toml")).unwrap();
    assert!(config.contains("revision = \"v1.1.0\""), "{}", config);
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%s"]),
        "pin imports/core v1.1.0"
    );
    assert_eq!(branch(&child), None);
    assert_eq!(support::worktrees::head(&child), released);
    assert!(!git_ok(
        &child,
        &["rev-parse", "--verify", "-q", "refs/heads/feat/x"]
    ));
}

/// `upgrade <dir>` raises a dependency to its newest release in the root's
/// config.
#[test]
fn normal_003_raises_a_named_dependency_to_its_newest_release() {
    let f = fixture("wt_raise", "");
    let ws = f.clone_root("ws");
    let newer = f.core_commit("main", "lib.txt", "v1.2");
    run_git_pub(&f.core, &["tag", "v1.2.0", &newer]);
    let before = config_text(&ws);
    let root_head = support::worktrees::head(&ws);
    let out = gs(&ws, &["upgrade", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core   v1.0.0 → v1.2.0"),
        "{}",
        out.stdout
    );
    let config = std::fs::read_to_string(ws.join(".gitscale.toml")).unwrap();
    assert!(config.contains("revision = \"v1.2.0\""), "{}", config);
    // Only that revision changed; with only the root to edit there is no
    // topic to create, and nothing is committed without --commit.
    assert_eq!(config, before.replace("v1.0.0", "v1.2.0"));
    assert_eq!(branch(&ws).as_deref(), Some("main"));
    assert_eq!(support::worktrees::head(&ws), root_head);
}

/// `upgrade <dir>` with a requester other than the root, and no topic:
/// the root's new branch becomes the topic, the requester is developed on
/// it, and both configs get the new release in place, comments kept. With
/// `--commit` each config is committed alone, as `pin <dep> <tag>`.
#[test]
fn normal_005_raising_creates_the_topic_and_edits_every_requester() {
    let env = TestEnv::new("upgrade_raise_topic");
    let (b, d, config) = raise_graph(&env);
    let ws = root_workspace(&env, &config);
    ok(&gs(&ws, &["pull"]));
    let child = ws.join("imports/b");
    identity(&child);

    let out = gs(&ws, &["upgrade", "--commit", "imports/d"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/d   v1.0.0 → v1.1.0"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("topic upgrade/d-v1.1.0 (created): imports/b developed"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("next to merge: imports/b"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&ws).as_deref(), Some("upgrade/d-v1.1.0"));
    assert_eq!(branch(&child).as_deref(), Some("upgrade/d-v1.1.0"));

    assert_eq!(
        config_text(&ws),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/d", &d, ", revision = \"v1.1.0\""),
        ]),
        "only d's revision changes in the root"
    );
    let child_config = config_text(&child);
    assert!(
        child_config.contains("# what b builds on")
            && child_config.contains("revision = \"v1.1.0\""),
        "{}",
        child_config
    );
    for repo in [&ws, &child] {
        assert_eq!(
            git(repo, &["log", "-1", "--format=%s"]),
            "pin imports/d v1.1.0",
            "{}",
            repo.display()
        );
        assert_eq!(
            git(repo, &["show", "--name-only", "--format=", "HEAD"]),
            ".gitscale.toml",
            "{}",
            repo.display()
        );
    }
}

/// `upgrade <dir> --dry-run` prints the whole plan — the topic it would
/// create, every edit — and changes nothing: no file, no branch, no
/// developed checkout.
#[test]
fn normal_006_a_dry_run_raise_changes_no_file_or_branch() {
    let env = TestEnv::new("upgrade_raise_dry_run");
    let (_, _, config) = raise_graph(&env);
    let ws = root_workspace(&env, &config);
    ok(&gs(&ws, &["pull"]));
    let child = ws.join("imports/b");
    let (root_before, child_before) = (config_text(&ws), config_text(&child));
    let root_head = support::worktrees::head(&ws);

    let out = gs(&ws, &["upgrade", "--dry-run", "imports/d"]);
    ok(&out);
    for expected in [
        "imports/d   v1.0.0 → v1.1.0",
        "would develop on topic upgrade/d-v1.1.0: imports/b",
        "imports/b/.gitscale.toml",
        "(dry run: nothing changed)",
    ] {
        assert!(
            out.stdout.contains(expected),
            "{}: {}",
            expected,
            out.stdout
        );
    }
    assert_eq!(config_text(&ws), root_before);
    assert_eq!(config_text(&child), child_before);
    assert_eq!(branch(&ws).as_deref(), Some("main"));
    assert_eq!(support::worktrees::head(&ws), root_head);
    assert_eq!(branch(&child), None);
    assert!(!git_ok(
        &ws,
        &["rev-parse", "--verify", "-q", "refs/heads/upgrade/d-v1.1.0"]
    ));
}

/// `-c` names the topic a raise creates.
#[test]
fn normal_007_c_names_the_topic_a_raise_creates() {
    let env = TestEnv::new("upgrade_raise_named_topic");
    let (_, _, config) = raise_graph(&env);
    let ws = root_workspace(&env, &config);
    ok(&gs(&ws, &["pull"]));
    let out = gs(&ws, &["upgrade", "-c", "feat/bump", "imports/d"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("topic feat/bump (created): imports/b developed"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&ws).as_deref(), Some("feat/bump"));
    assert_eq!(branch(&ws.join("imports/b")).as_deref(), Some("feat/bump"));
}

/// The newest release a raise picks stays in the pin's major and skips
/// pre-releases; a pre-release pin may move to a later pre-release, and
/// `--major` crosses majors.
#[test]
fn normal_008_a_raise_stays_in_its_major_and_skips_pre_releases_unless_asked() {
    let f = fixture("upgrade_raise_major", "");
    let ws = f.clone_root("ws");
    for tag in ["v1.2.0", "v1.3.0-rc.1", "v1.3.0-rc.2", "v2.0.0"] {
        let commit = f.core_commit("main", "lib.txt", tag);
        run_git_pub(&f.core, &["tag", tag, &commit]);
    }
    let before = config_text(&ws);

    let out = gs(&ws, &["upgrade", "--dry-run", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core   v1.0.0 → v1.2.0"),
        "{}",
        out.stdout
    );
    assert_eq!(config_text(&ws), before);

    std::fs::write(
        ws.join(".gitscale.toml"),
        before.replace("v1.0.0", "v1.3.0-rc.1"),
    )
    .unwrap();
    let out = gs(&ws, &["upgrade", "--dry-run", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/core   v1.3.0-rc.1 → v1.3.0-rc.2"),
        "{}",
        out.stdout
    );

    std::fs::write(ws.join(".gitscale.toml"), &before).unwrap();
    let out = gs(&ws, &["upgrade", "--major", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core   v1.0.0 → v2.0.0"),
        "{}",
        out.stdout
    );
    assert_eq!(config_text(&ws), before.replace("v1.0.0", "v2.0.0"));
}

/// `upgrade --resolved --commit` commits `.gitscale.toml` alone: whatever
/// else is staged stays staged, out of the commit.
#[test]
fn normal_009_resolved_commit_commits_the_config_alone() {
    let env = TestEnv::new("upgrade_resolved_commit");
    env.init_playground_git();
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    run_git_pub(&env.playground, &["add", ".gitscale.toml"]);
    run_git_pub(&env.playground, &["commit", "-q", "-m", "config"]);
    std::fs::write(env.playground.join("notes.txt"), "staged").unwrap();
    run_git_pub(&env.playground, &["add", "notes.txt"]);

    let out = env.run(&["upgrade", "--resolved", "--commit"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("commit  .gitscale.toml: pin imports/d v1.5.0"),
        "{}",
        out.stdout
    );
    assert_eq!(
        git(&env.playground, &["log", "-1", "--format=%s"]),
        "pin imports/d v1.5.0"
    );
    assert_eq!(
        git(
            &env.playground,
            &["show", "--name-only", "--format=", "HEAD"]
        ),
        ".gitscale.toml"
    );
    assert_eq!(
        git(&env.playground, &["diff", "--cached", "--name-only"]),
        "notes.txt"
    );
}

/// `upgrade --resolved` changes only root entries that give a revision and
/// are not overrides; named directories narrow it further.
#[test]
fn normal_010_resolved_leaves_overrides_and_unpinned_entries_alone() {
    let env = TestEnv::new("upgrade_resolved_skips");
    let d = tagged(&env, "d", &[("v1.2.0", ""), ("v1.5.0", "")]);
    let e = tagged(&env, "e", &[("v1.2.0", ""), ("v1.5.0", "")]);
    let f = tagged(&env, "f", &[("v1.2.0", ""), ("v1.5.0", "")]);
    let b = dependant(
        &env,
        "b",
        &[
            ("libs/d", &d, ", revision = \"v1.5.0\""),
            ("libs/e", &e, ", revision = \"v1.5.0\""),
            ("libs/f", &f, ", revision = \"v1.5.0\""),
        ],
    );
    let config = repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ""),
        ("imports/e", &e, ", revision = \"v1.2.0\", override = true"),
        ("imports/f", &f, ", revision = \"v1.2.0\""),
    ]);
    env.write_config(&config);

    let out = env.run(&[
        "upgrade",
        "--resolved",
        "--dry-run",
        "imports/b",
        "imports/e",
    ]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("already declares"), "{}", out.stdout);

    let out = env.run(&["upgrade", "--resolved"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("Updated 1 entry in .gitscale.toml"),
        "{}",
        out.stdout
    );
    let expected = repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ""),
        ("imports/e", &e, ", revision = \"v1.2.0\", override = true"),
        ("imports/f", &f, ", revision = \"v1.5.0\""),
    ]);
    assert_eq!(config_text(&env.playground), expected);
}

/// On a topic, `upgrade --resolved` writes what a merge would pin — the
/// revision resolution would choose without the topic — never the topic
/// branch itself.
#[test]
fn normal_011_resolved_on_a_topic_writes_the_pin_not_the_branch() {
    let env = TestEnv::new("upgrade_resolved_topic");
    let (b, d) = diamond(&env);
    let ws = root_workspace(
        &env,
        &repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/d", &d, ", revision = \"v1.2.0\""),
        ]),
    );
    ok(&gs(&ws, &["pull"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/d"]));
    assert_eq!(branch(&ws.join("imports/d")).as_deref(), Some("feat/x"));

    let out = gs(&ws, &["upgrade", "--resolved"]);
    ok(&out);
    assert!(
        out.stdout.contains("v1.2.0 → v1.5.0") && out.stdout.contains("raised by imports/b"),
        "{}",
        out.stdout
    );
    let config = config_text(&ws);
    assert!(config.contains("revision = \"v1.5.0\""), "{}", config);
    assert!(!config.contains("feat/x"), "{}", config);
}

/// `upgrade --dry-run` on a topic reports the promotion and its edits and
/// changes nothing; the real run then warns that the slot's topic branch
/// still exists on its remote, which CI would keep matching by name.
#[test]
fn normal_012_a_dry_run_promotion_changes_nothing_and_the_real_one_warns_of_the_remote_branch() {
    let f = fixture("upgrade_promote_dry_run", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    ok(&gs(&ws, &["push"]));
    let released = f.core_commit("main", "lib.txt", "v2");
    run_git_pub(&f.core, &["tag", "v1.1.0", &released]);
    let before = config_text(&ws);

    let out = gs(&ws, &["upgrade", "--dry-run"]);
    ok(&out);
    for expected in [
        "imports/core  promoted → v1.1.0",
        "imports/core  v1.0.0 → v1.1.0",
        "(dry run: nothing changed)",
    ] {
        assert!(
            out.stdout.contains(expected),
            "{}: {}",
            expected,
            out.stdout
        );
    }
    assert_eq!(config_text(&ws), before);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));

    let out = gs(&ws, &["upgrade"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/core: branch feat/x still exists on its remote"),
        "{}",
        out.stdout
    );
    assert!(config_text(&ws).contains("revision = \"v1.1.0\""));
    assert_eq!(branch(&child), None);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// `upgrade --resolved` edits table-style entries as well as inline ones.
#[test]
fn edge_004_resolved_edits_table_style_entries() {
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

/// A topic slot whose change no release holds is left on the topic: its
/// branch, with commits nobody else has, survives `upgrade`, and no pin
/// moves. Promotion deletes the branch of what it promotes, so saying
/// "promoted" here would lose those commits.
#[test]
fn edge_013_promotion_leaves_a_slot_whose_change_no_tag_holds() {
    let f = fixture("upgrade_promote_untagged", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);

    let out = gs(&ws, &["upgrade"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core  no change yet"),
        "{}",
        out.stdout
    );

    // A commit only this workspace has, and a release without it.
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "unpushed"]);
    let tip = support::worktrees::head(&child);
    let other = f.core_commit("main", "other.txt", "elsewhere");
    run_git_pub(&f.core, &["tag", "v1.1.0", &other]);
    let before = config_text(&ws);

    let out = gs(&ws, &["upgrade"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core  not tagged yet"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("promoted"), "{}", out.stdout);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(support::worktrees::head(&child), tip);
    assert_eq!(config_text(&ws), before);
}

/// Uncommitted work holds a slot on the topic even once a release holds its
/// committed change: the work stays, the branch stays, no pin moves.
#[test]
fn edge_014_promotion_holds_a_slot_with_uncommitted_work() {
    let f = fixture("upgrade_promote_held", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    ok(&gs(&ws, &["push"]));
    let released = f.core_commit("main", "lib.txt", "v2");
    run_git_pub(&f.core, &["tag", "v1.1.0", &released]);
    std::fs::write(child.join("lib.txt"), "v3 in progress").unwrap();
    let before = config_text(&ws);

    let out = gs(&ws, &["upgrade"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/core  held (1 change uncommitted)"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(
        std::fs::read_to_string(child.join("lib.txt")).unwrap(),
        "v3 in progress"
    );
    assert_eq!(config_text(&ws), before);
}

/// Promotion stays in the major each pin is in: promoting a slot to v1.1.0
/// leaves the root's entry for the same repository at v0.9.0 alone —
/// rewritten, the two entries would be one major and the workspace would no
/// longer resolve.
#[test]
#[ignore = "bug: promotion rewrites every root entry of the repository below the tag, across majors"]
fn edge_015_promotion_leaves_another_majors_entry_alone() {
    let env = TestEnv::new("upgrade_promote_majors");
    let core = env.create_bare_repo("core", "main", &[("lib.txt", "v1")]);
    run_git_pub(&core, &["tag", "v0.9.0", "main"]);
    run_git_pub(&core, &["tag", "v1.0.0", "main"]);
    let ws = root_workspace(
        &env,
        &repos(&[
            ("imports/core", &core, ", revision = \"v1.0.0\""),
            ("imports/core_old", &core, ", revision = \"v0.9.0\""),
        ]),
    );
    ok(&gs(&ws, &["pull"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    ok(&gs(&ws, &["push"]));
    let released = env.push_commit(&core, "main", "lib.txt", "v2");
    run_git_pub(&core, &["tag", "v1.1.0", &released]);
    run_git_pub(&core, &["branch", "-D", "feat/x"]);

    let out = gs(&ws, &["upgrade"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core  promoted → v1.1.0"),
        "{}",
        out.stdout
    );
    let config = config_text(&ws);
    let line = |dir: &str| {
        config
            .lines()
            .find(|l| l.starts_with(&format!("\"{}\"", dir)))
            .unwrap_or_default()
            .to_string()
    };
    assert!(line("imports/core").contains("v1.1.0"), "{}", config);
    assert!(line("imports/core_old").contains("v0.9.0"), "{}", config);
    ok(&gs(&ws, &["status"]));
}

/// A dependency may be named by the link a repository has to it, as that
/// repository calls it.
#[test]
fn edge_016_a_raise_takes_a_dependency_by_its_link_path() {
    let env = TestEnv::new("upgrade_raise_link_name");
    let (_, _, config) = raise_graph(&env);
    env.write_config(&config);
    let out = env.run(&["upgrade", "--dry-run", "imports/b/libs/d"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("imports/d   v1.0.0 → v1.1.0"),
        "{}",
        out.stdout
    );
    assert_eq!(config_text(&env.playground), config);
}

/// A requester asking for a branch is reported and left alone, and with only
/// the root to edit no topic is created.
#[test]
fn edge_017_a_requester_on_a_branch_is_reported_and_left_alone() {
    let env = TestEnv::new("upgrade_raise_branch_requester");
    let d = tagged(&env, "d", &[("v1.0.0", ""), ("v1.1.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"main\"")]);
    let ws = root_workspace(
        &env,
        &repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/d", &d, ", revision = \"v1.0.0\""),
        ]),
    );
    ok(&gs(&ws, &["pull"]));
    let child = ws.join("imports/b");
    let child_before = config_text(&child);

    let out = gs(&ws, &["upgrade", "imports/d"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/b/.gitscale.toml   libs/d asks for main; not changed"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("topic"), "{}", out.stdout);
    assert_eq!(branch(&ws).as_deref(), Some("main"));
    assert_eq!(branch(&child), None);
    assert_eq!(config_text(&child), child_before);
    assert!(config_text(&ws).contains("revision = \"v1.1.0\""));
}

/// Overrides stop a raise where they stand: a dependency the root overrides
/// is not raised at all, and a requester the root overrides cannot be
/// developed, so its pin is reported and left — while the root's own entry
/// is still raised, with no topic created.
#[test]
fn edge_018_root_overrides_hold_what_a_raise_would_change() {
    let env = TestEnv::new("upgrade_raise_overrides");
    env.init_playground_git();
    let (b, d, _) = raise_graph(&env);

    let held = repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.0.0\", override = true"),
    ]);
    env.write_config(&held);
    let out = env.run(&["upgrade", "imports/d"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("imports/d is held at v1.0.0 by override in .gitscale.toml; nothing changed"),
        "{}",
        out.stdout
    );
    assert_eq!(config_text(&env.playground), held);

    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\", override = true"),
        ("imports/d", &d, ", revision = \"v1.0.0\""),
    ]));
    let out = env.run(&["upgrade", "imports/d"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("imports/b   held by override in .gitscale.toml; its pin is not changed"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("topic"), "{}", out.stdout);
    assert_eq!(
        config_text(&env.playground),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\", override = true"),
            ("imports/d", &d, ", revision = \"v1.1.0\""),
        ])
    );
    assert_eq!(
        git(&env.playground, &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
}

/// Nothing to raise is said, not done: a pin already at its newest release,
/// and an entry pinned to a branch.
#[test]
fn edge_019_a_raise_with_nothing_to_raise_says_why_and_changes_nothing() {
    let f = fixture("upgrade_raise_nothing", "");
    let ws = f.clone_root("ws");
    let before = config_text(&ws);
    let out = gs(&ws, &["upgrade", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/core is already at its newest release, v1.0.0"),
        "{}",
        out.stdout
    );
    assert_eq!(config_text(&ws), before);

    let on_branch = before.replace("v1.0.0", "main");
    std::fs::write(ws.join(".gitscale.toml"), &on_branch).unwrap();
    let out = gs(&ws, &["upgrade", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/core is not pinned to a version; nothing to raise from"),
        "{}",
        out.stdout
    );
    assert_eq!(config_text(&ws), on_branch);
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// Each misuse of `upgrade` is refused with what to do instead, and leaves
/// the config and the root's branch as they were.
#[test]
fn error_020_misused_options_are_refused_and_change_nothing() {
    let env = TestEnv::new("upgrade_option_errors");
    env.init_playground_git();
    let (b, d) = diamond(&env);
    let config = repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]);
    env.write_config(&config);
    for (args, message) in [
        (
            &["upgrade", "--resolved", "--major"][..],
            "it takes neither --major nor -c",
        ),
        (
            &["upgrade", "--resolved", "-c", "feat/y"][..],
            "it takes neither --major nor -c",
        ),
        (
            &["upgrade", "--major"][..],
            "--major raises a named dependency",
        ),
        (
            &["upgrade", "-c", "feat/y"][..],
            "-c names the topic an upgrade of named dependencies creates",
        ),
        (&["upgrade"][..], "not on a topic"),
        (
            &["upgrade", "imports/nowhere"][..],
            "no checkout at imports/nowhere",
        ),
        (
            &["upgrade", "--resolved", "imports/nowhere"][..],
            "imports/nowhere is not an entry of the root .gitscale.toml",
        ),
    ] {
        let out = env.run(args);
        assert!(!out.success, "{:?}: {}", args, out.stdout);
        assert!(out.stderr.contains(message), "{:?}: {}", args, out.stderr);
        assert_eq!(config_text(&env.playground), config, "{:?}", args);
        assert_eq!(
            git(&env.playground, &["symbolic-ref", "--short", "HEAD"]),
            "main",
            "{:?}",
            args
        );
    }

    // On a topic, -c may not name another one.
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let out = env.run(&["upgrade", "-c", "feat/y", "imports/d"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("already on topic feat/x"),
        "{}",
        out.stderr
    );
    assert_eq!(config_text(&env.playground), config);
}

/// `upgrade` edits the configs of a developer machine's checkouts; in CI,
/// which keeps none, it refuses.
#[test]
fn error_021a_a_raise_refuses_in_ci() {
    let env = TestEnv::new("upgrade_ci_raise");
    let (b, d) = diamond(&env);
    let config = repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]);
    env.write_config(&config);
    let out = env.run_with_env(&[("CI", "true")], &["upgrade", "imports/d"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("CI keeps none to edit"),
        "{}",
        out.stderr
    );
    assert_eq!(config_text(&env.playground), config);
}

/// The docs say every form of `upgrade` refuses in CI; `--resolved` too.
#[test]
#[ignore = "bug: upgrade --resolved runs in CI; it returns before the CI refusal (upgrade.rs:68-82)"]
fn error_021b_resolved_refuses_in_ci() {
    let env = TestEnv::new("upgrade_ci_resolved");
    let (b, d) = diamond(&env);
    let config = repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]);
    env.write_config(&config);
    let out = env.run_with_env(&[("CI", "true")], &["upgrade", "--resolved"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("CI keeps none to edit"),
        "{}",
        out.stderr
    );
    assert_eq!(config_text(&env.playground), config);
}
