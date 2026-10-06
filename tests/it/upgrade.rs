//! `git upgrade`: promoting a topic's released repositories, and raising
//! named dependencies, in the configs on the topic.

use crate::support;
use crate::support::resolution::*;
use crate::support::worktrees::*;
use crate::support::{run_git_pub, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// Once a release tag holds a joined child's change, `upgrade` writes the
/// tag into the root's config, deletes the child's topic branch and detaches
/// it at the tag.
#[test]
fn normal_002_promotes_a_child_whose_change_is_released() {
    let f = fixture("wt_promote", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
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
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
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
    // Only that revision changed, and nothing is committed without --commit.
    assert_eq!(config, before.replace("v1.0.0", "v1.2.0"));
    assert_eq!(branch(&ws).as_deref(), Some("feat/bump"));
    assert_eq!(support::worktrees::head(&ws), root_head);
}

/// `upgrade <dir>` edits only the configs on the topic: a requester off it
/// is named with the command that joins it, and left as it is. Joined by
/// `git topic join --dependants`, it is edited in place, comments kept, and
/// with `--commit` each config is committed alone, as `pin <dep> <tag>`.
#[test]
fn normal_005_a_raise_edits_the_topics_configs_and_names_requesters_off_it() {
    let env = TestEnv::new("upgrade_raise_topic");
    let (b, d, config) = raise_graph(&env);
    let ws = root_workspace(&env, &config);
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
    let child = ws.join("imports/b");
    let child_before = config_text(&child);

    let out = gs(&ws, &["upgrade", "--dry-run", "imports/d"]);
    ok(&out);
    assert!(
        out.stdout.contains(
            "imports/b/.gitscale.toml   libs/d asks for v1.0.0: git topic join imports/b"
        ),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&child), None);
    assert_eq!(config_text(&child), child_before);

    let out = gs(&ws, &["topic", "join", "--dependants", "imports/d"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/b on feat/bump"),
        "{}",
        out.stdout
    );
    identity(&child);

    let out = gs(&ws, &["upgrade", "--commit", "imports/d"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/d   v1.0.0 → v1.1.0"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("next to merge: imports/b"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&ws).as_deref(), Some("feat/bump"));
    assert_eq!(branch(&child).as_deref(), Some("feat/bump"));

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

/// `upgrade <dir> --dry-run` prints every edit and changes nothing.
#[test]
fn normal_006_a_dry_run_raise_changes_no_file() {
    let env = TestEnv::new("upgrade_raise_dry_run");
    let (_, _, config) = raise_graph(&env);
    let ws = root_workspace(&env, &config);
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
    ok(&gs(&ws, &["topic", "join", "imports/b"]));
    let child = ws.join("imports/b");
    let (root_before, child_before) = (config_text(&ws), config_text(&child));
    let heads = (
        support::worktrees::head(&ws),
        support::worktrees::head(&child),
    );

    let out = gs(&ws, &["upgrade", "--dry-run", "imports/d"]);
    ok(&out);
    for expected in [
        "imports/d   v1.0.0 → v1.1.0",
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
    assert_eq!(
        (
            support::worktrees::head(&ws),
            support::worktrees::head(&child)
        ),
        heads
    );
}

/// The newest release a raise picks stays in the pin's major and skips
/// pre-releases; a pre-release pin may move to a later pre-release, and
/// `--major` crosses majors.
#[test]
fn normal_008_a_raise_stays_in_its_major_and_skips_pre_releases_unless_asked() {
    let f = fixture("upgrade_raise_major", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
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

/// `upgrade --dry-run` on a topic reports the promotion, its edits and the
/// remote branch it would delete, and changes nothing; the real run deletes
/// the slot's topic branch on its remote, which placement and CI would
/// otherwise keep matching by name.
#[test]
fn normal_012_a_dry_run_promotion_changes_nothing_and_the_real_one_deletes_the_remote_branch() {
    let f = fixture("upgrade_promote_dry_run", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
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
        "imports/core  would promote → v1.1.0",
        "imports/core  v1.0.0 → v1.1.0",
        "imports/core: would delete origin/feat/x",
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
    assert!(git_ok(
        &f.core,
        &["rev-parse", "--verify", "-q", "refs/heads/feat/x"]
    ));

    let out = gs(&ws, &["upgrade"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core: deleted origin/feat/x"),
        "{}",
        out.stdout
    );
    assert!(!git_ok(
        &f.core,
        &["rev-parse", "--verify", "-q", "refs/heads/feat/x"]
    ));
    assert!(config_text(&ws).contains("revision = \"v1.1.0\""));
    assert_eq!(branch(&child), None);
}

/// A calendar raise stays in its major; `--major` crosses to the next.
#[test]
fn normal_022_a_calendar_raise_stays_in_its_major_and_major_crosses_it() {
    let env = TestEnv::new("upgrade_raise_calver");
    let core = tagged(
        &env,
        "core",
        &[
            ("v1-2026.10.01-1", ""),
            ("v1-2026.10.05-1", ""),
            ("v2-2026.11.02-1", ""),
        ],
    );
    let ws = root_workspace(
        &env,
        &repos(&[("imports/core", &core, ", revision = \"v1-2026.10.01-1\"")]),
    );
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);

    let out = gs(&ws, &["upgrade", "--dry-run", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/core   v1-2026.10.01-1 → v1-2026.10.05-1"),
        "{}",
        out.stdout
    );
    let out = gs(&ws, &["upgrade", "--dry-run", "--major", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/core   v1-2026.10.01-1 → v2-2026.11.02-1"),
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A raise edits table-style entries as well as inline ones, comments kept.
#[test]
fn edge_004_a_raise_edits_table_style_entries() {
    let env = TestEnv::new("res_write_table");
    let (b, d, _) = raise_graph(&env);
    let ws = root_workspace(
        &env,
        &format!(
            "[repos.\"imports/b\"]\nurl = \"{}\"\nrevision = \"v1.0.0\"\n\n\
             [repos.\"imports/d\"]\nurl = \"{}\"\nrevision = \"v1.0.0\"  # held back?\n",
            b.display(),
            d.display()
        ),
    );
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
    ok(&gs(&ws, &["upgrade", "imports/d"]));
    let config = config_text(&ws);
    assert!(
        config.contains("revision = \"v1.1.0\"  # held back?"),
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
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
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
        out.stdout.contains("imports/core  no tag"),
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
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
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
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
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
    ok(&gs(&ws, &["ls"]));
}

/// A dependency may be named by the link a repository has to it, as that
/// repository calls it.
#[test]
fn edge_016_a_raise_takes_a_dependency_by_its_link_path() {
    let env = TestEnv::new("upgrade_raise_link_name");
    let (_, _, config) = raise_graph(&env);
    let ws = root_workspace(&env, &config);
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
    let out = gs(&ws, &["upgrade", "--dry-run", "imports/b/libs/d"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/d   v1.0.0 → v1.1.0"),
        "{}",
        out.stdout
    );
    assert_eq!(config_text(&ws), config);
}

/// A requester asking for a branch is reported and left alone.
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
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
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
    assert_eq!(branch(&child), None);
    assert_eq!(config_text(&child), child_before);
    assert!(config_text(&ws).contains("revision = \"v1.1.0\""));
}

/// Overrides stop a raise where they stand: a dependency the root overrides
/// is not raised at all, and a requester the root overrides cannot be
/// joined, so its pin is reported and left — while the root's own entry
/// is still raised.
#[test]
fn edge_018_root_overrides_hold_what_a_raise_would_change() {
    let env = TestEnv::new("upgrade_raise_overrides");
    env.init_playground_git();
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/bump"]);
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
    assert_eq!(
        config_text(&env.playground),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\", override = true"),
            ("imports/d", &d, ", revision = \"v1.1.0\""),
        ])
    );
    assert_eq!(
        git(&env.playground, &["symbolic-ref", "--short", "HEAD"]),
        "feat/bump"
    );
}

/// Nothing to raise is said, not done: a pin already at its newest release,
/// and an entry pinned to a branch.
#[test]
fn edge_019_a_raise_with_nothing_to_raise_says_why_and_changes_nothing() {
    let f = fixture("upgrade_raise_nothing", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
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
/// the config and the root's branch as they were. Off a topic both forms
/// refuse: only `git topic start` begins one.
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
    let refused = |args: &[&str], message: &str| {
        let out = env.run(args);
        assert!(!out.success, "{:?}: {}", args, out.stdout);
        assert!(out.stderr.contains(message), "{:?}: {}", args, out.stderr);
        assert_eq!(config_text(&env.playground), config, "{:?}", args);
    };
    refused(&["upgrade", "--major"], "--major raises a named dependency");
    refused(&["upgrade"], "not on a topic: git topic start NAME");
    refused(
        &["upgrade", "imports/d"],
        "not on a topic: git topic start NAME",
    );
    refused(
        &["upgrade", "--resolved"],
        "unexpected argument '--resolved'",
    );
    refused(
        &["upgrade", "-c", "feat/y", "imports/d"],
        "unexpected argument '-c'",
    );
    assert_eq!(
        git(&env.playground, &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );

    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    refused(
        &["upgrade", "imports/nowhere"],
        "imports/nowhere is not a checkout of this workspace",
    );
}

/// `upgrade` edits the configs of a developer machine's checkouts; in CI,
/// which keeps none, it refuses.
#[test]
fn error_021_upgrade_refuses_in_ci() {
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

/// A promotion that would delete a remote branch holding work its release
/// lacks fails before anything changes: the branch, the config and the
/// checkout's place on the topic all stay.
#[test]
fn error_023_promotion_fails_while_the_remote_branch_holds_unreleased_work() {
    let f = fixture("upgrade_promote_unreleased", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    ok(&gs(&ws, &["push"]));
    let released = f.core_commit("main", "lib.txt", "v2");
    run_git_pub(&f.core, &["tag", "v1.1.0", &released]);
    // Pushed after the merge by someone else: not in the release.
    let later = f.core_commit("feat/x", "more.txt", "after the merge");
    let before = config_text(&ws);

    for args in [&["upgrade", "--dry-run"][..], &["upgrade"][..]] {
        let out = gs(&ws, args);
        assert!(!out.success, "{:?}: {}", args, out.stdout);
        assert!(
            out.stderr.contains(
                "imports/core: origin/feat/x has changes v1.1.0 does not hold; merge or drop \
                 them, then run again"
            ),
            "{:?}: {}",
            args,
            out.stderr
        );
    }
    assert_eq!(git(&f.core, &["rev-parse", "feat/x"]), later);
    assert_eq!(config_text(&ws), before);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

/// A remote that refuses the deletion fails the promotion with no config
/// edited; once the deletion can go through, running it again completes.
#[test]
fn error_024_a_refused_deletion_changes_no_config_and_a_second_run_completes() {
    let f = fixture("upgrade_promote_refused", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    ok(&gs(&ws, &["push"]));
    let released = f.core_commit("main", "lib.txt", "v2");
    run_git_pub(&f.core, &["tag", "v1.1.0", &released]);
    let hook = f.core.join("hooks/pre-receive");
    crate::support::hooks::write_script(&hook, "echo protected >&2\nexit 1\n");
    let before = config_text(&ws);

    let out = gs(&ws, &["upgrade", "--commit"]);
    assert!(!out.success, "{}", out.stdout);
    // Git's own reason is in the message, not dropped.
    assert!(
        out.stderr
            .contains("imports/core: cannot delete origin/feat/x (error: failed to push"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("; no config was changed"),
        "{}",
        out.stderr
    );
    assert_eq!(config_text(&ws), before);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));

    std::fs::remove_file(&hook).unwrap();
    let out = gs(&ws, &["upgrade", "--commit"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core: deleted origin/feat/x"),
        "{}",
        out.stdout
    );
    assert!(config_text(&ws).contains("revision = \"v1.1.0\""));
    assert_eq!(branch(&child), None);
}

/// Promoting a checkout only its dependants ask for: the release goes into
/// the config of each requester on the topic, a requester left at its pin
/// keeps its older request, and the higher one wins once both are read.
#[test]
fn normal_025_promoting_an_implicit_checkout_edits_the_requesters_on_the_topic() {
    let env = TestEnv::new("upgrade_promote_implicit");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let config = format!(
        "{}{}",
        allow(&env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    );
    let ws = root_workspace(&env, &config);
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/d", "imports/b"]));
    let child = ws.join("imports/d");
    identity(&child);
    identity(&ws.join("imports/b"));
    std::fs::write(child.join("VERSION"), "changed").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    ok(&gs(&ws, &["push"]));
    let released = env.push_commit(&d, "main", "VERSION", "changed");
    run_git_pub(&d, &["tag", "v1.1.0", &released]);
    let c_before = config_text(&ws.join("imports/c"));

    let out = gs(&ws, &["upgrade", "--commit"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/b/.gitscale.toml   libs/d  v1.0.0 → v1.1.0"),
        "{}",
        out.stdout
    );
    assert!(config_text(&ws.join("imports/b")).contains("revision = \"v1.1.0\""));
    assert_eq!(config_text(&ws.join("imports/c")), c_before);
    assert_eq!(config_text(&ws), config);
    ok(&gs(&ws, &["sync"]));
    assert_eq!(support::worktrees::head(&child), released);
}

/// A raise only takes a release whose history holds the pin: a newer tag on
/// a line split off before the pin would lose what the pin had. `--major`
/// relaxes that, since a new major is often cut on a line of its own.
#[test]
fn normal_026_a_raise_skips_a_line_split_off_before_the_pin_unless_crossing_a_major() {
    let f = fixture("upgrade_raise_split_line", "");
    f.core_commit(
        "main",
        ".gitscale.toml",
        "[branches]\npinned = [\"main\", \"maint\"]\n",
    );
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
    let pin = f.core_commit("main", "lib.txt", "v1.1");
    run_git_pub(&f.core, &["tag", "v1.1.0", &pin]);
    std::fs::write(
        ws.join(".gitscale.toml"),
        config_text(&ws).replace("v1.0.0", "v1.1.0"),
    )
    .unwrap();
    run_git_pub(&f.core, &["branch", "maint", "v1.0.0"]);
    let split = f.core_commit("maint", "fix.txt", "a fix");
    run_git_pub(&f.core, &["tag", "v1.5.0", &split]);
    let major = f.core_commit("maint", "fix.txt", "a major");
    run_git_pub(&f.core, &["tag", "v2.0.0", &major]);
    let newer = f.core_commit("main", "lib.txt", "v1.2");
    run_git_pub(&f.core, &["tag", "v1.2.0", &newer]);

    let out = gs(&ws, &["upgrade", "--dry-run", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core   v1.1.0 → v1.2.0"),
        "{}",
        out.stdout
    );
    let out = gs(&ws, &["upgrade", "--dry-run", "--major", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core   v1.1.0 → v2.0.0"),
        "{}",
        out.stdout
    );
}

/// Promotion takes the newest release that holds the change: a newer
/// hotfix cut from the pin, without the change, is walked past rather than
/// reported as `no tag`.
#[test]
fn normal_027_promotion_takes_the_newest_release_that_holds_the_change() {
    let f = fixture("upgrade_promote_past_hotfix", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    ok(&gs(&ws, &["push"]));
    let released = f.core_commit("main", "lib.txt", "v2");
    run_git_pub(&f.core, &["tag", "v1.1.0", &released]);
    run_git_pub(&f.core, &["branch", "hotfix", "v1.0.0"]);
    let hotfix = f.core_commit("hotfix", "other.txt", "a hotfix");
    run_git_pub(&f.core, &["tag", "v1.2.0", &hotfix]);

    let out = gs(&ws, &["upgrade", "--dry-run"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core  would promote → v1.1.0"),
        "{}",
        out.stdout
    );
}

/// A repository's releases are the tags its pinned branches hold — by
/// default its default branch alone — as its default branch's config says,
/// so a pin older than the policy follows it too.
#[test]
fn normal_028_pinned_branches_bind_the_tags_a_raise_takes() {
    let f = fixture("upgrade_release_branches", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
    run_git_pub(&f.core, &["branch", "hotfix", "v1.0.0"]);
    let hotfix = f.core_commit("hotfix", "fix.txt", "a hotfix");
    run_git_pub(&f.core, &["tag", "v1.1.0", &hotfix]);

    // On a branch it does not pin: no release.
    let out = gs(&ws, &["upgrade", "--dry-run", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/core is already at its newest release, v1.0.0"),
        "{}",
        out.stdout
    );

    f.core_commit(
        "main",
        ".gitscale.toml",
        "[branches]\npinned = [\"main\", \"hotfix\"]\n",
    );
    let out = gs(&ws, &["upgrade", "--dry-run", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core   v1.0.0 → v1.1.0"),
        "{}",
        out.stdout
    );
}

/// Pinned branches that name no branch of the repository are an error that
/// says which repository — not an empty list of candidates that would read
/// as "no release".
#[test]
fn error_029_pinned_branches_matching_no_branch_say_so() {
    let f = fixture("upgrade_release_branches_none", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/bump"]);
    f.core_commit(
        "main",
        ".gitscale.toml",
        "[branches]\npinned = [\"release/*\"]\n",
    );
    let out = gs(&ws, &["upgrade", "--dry-run", "imports/core"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains(&format!(
            "no branch of {} is pinned: [branches] pinned = release/*, so it has no releases",
            f.core.display()
        )),
        "{}",
        out.stderr
    );
}

/// A pin no release holds — cut on a branch the repository does not pin —
/// says so, rather than `no tag`.
#[test]
fn edge_030_a_pin_no_release_holds_says_so() {
    let f = fixture("upgrade_no_release_holds_pin", "");
    let side = f.core_commit("side", "lib.txt", "side");
    run_git_pub(&f.core, &["tag", "v1.0.1", &side]);
    let ws = f.clone_root("ws");
    std::fs::write(
        ws.join(".gitscale.toml"),
        config_text(&ws).replace("v1.0.0", "v1.0.1"),
    )
    .unwrap();
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);

    let out = gs(&ws, &["upgrade", "--dry-run"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("imports/core  no release contains v1.0.1"),
        "{}",
        out.stdout
    );
}

/// A repository that publishes artefacts is released when its tag's image is
/// there too — whatever form this workspace takes it in. Until then the
/// promotion waits, saying so.
#[test]
fn normal_031_promotion_waits_for_the_image_of_a_repository_that_publishes_one() {
    let f = fixture("upgrade_promote_waits_for_image", "");
    let ws = f.clone_root("ws");
    std::fs::write(
        ws.join(".gitscale.toml"),
        format!("{}{}", f.env.registries(), config_text(&ws)),
    )
    .unwrap();
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "v2").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "the change"]);
    ok(&gs(&ws, &["push"]));
    f.core_commit("main", "lib.txt", "v2");
    let released = f.core_commit(
        "main",
        ".gitscale.toml",
        "[artefact]\ninclude = [\"dist/**\"]\n",
    );
    run_git_pub(&f.core, &["tag", "v1.1.0", &released]);

    let out = gs(&ws, &["upgrade", "--dry-run"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core  tagged v1.1.0, no image"),
        "{}",
        out.stdout
    );

    f.env.publish(&f.core, "v1.1.0", &[("lib.bin", "built")]);
    let out = gs(&ws, &["upgrade", "--dry-run"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core  would promote → v1.1.0"),
        "{}",
        out.stdout
    );
}
