//! The topic workflow: the root's branch is the topic, `develop` puts a
//! checkout on it, and pinned branches follow no topic.

use crate::support::worktrees::*;
use crate::support::{run_git_pub, strip_ansi, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_puts_a_child_on_the_topic_from_its_pin() {
    let f = fixture("wt_develop", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let pin = head(&child);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    let out = gs(&ws, &["develop", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core on feat/x, from v1.0.0"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), pin);
    assert!(writable(&child.join("lib.txt")));

    let row = status_row(&ws, "imports/core");
    assert!(row.contains("feat/x"), "{}", row);
    assert!(row.contains(" ok "), "{}", row);
    assert!(row.ends_with("topic, no change yet"), "{}", row);
    let table = strip_ansi(&gs(&ws, &["status"]).stdout);
    assert!(table.starts_with("topic feat/x"), "{}", table);

    // A pull keeps it there.
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

/// The remote has the topic branch: a pull puts the child on it, writable,
/// tracking the remote's.
#[test]
fn normal_002_a_child_follows_its_remote_branch_of_the_topic() {
    let f = fixture("wt_follow", "");
    let ws = f.clone_root("ws");
    let tip = f.core_commit("feat/x", "lib.txt", "remote work");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    let child = ws.join("imports/core");
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), tip);
    assert_eq!(
        git(&child, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/feat/x"
    );
    assert!(writable(&child.join("lib.txt")));

    // Back on main: detached at the pin, read-only again.
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert!(!writable(&child.join("lib.txt")));
}

/// `git switch -c feat/y` from topic feat/x carries the developed children to
/// feat/y, at the commits they are at, edits included.
#[test]
fn normal_003_a_new_branch_from_a_topic_carries_its_children() {
    let f = fixture("wt_carry", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "committed").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "x work"]);
    std::fs::write(child.join("lib.txt"), "uncommitted").unwrap();
    let at = head(&child);

    run_git_pub(&ws, &["switch", "-q", "-c", "feat/y"]);
    let out = gs(&ws, &["pull"]);
    ok(&out);
    assert!(
        out.stdout.contains("carry  imports/core → feat/y"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&child).as_deref(), Some("feat/y"));
    assert_eq!(head(&child), at);
    assert_eq!(
        std::fs::read_to_string(child.join("lib.txt")).unwrap(),
        "uncommitted"
    );

    // Switching back to an existing topic is no new branch: nothing carried.
    run_git_pub(&child, &["checkout", "--", "lib.txt"]);
    run_git_pub(&ws, &["switch", "-q", "feat/x"]);
    let out = gs(&ws, &["pull"]);
    ok(&out);
    assert!(!out.stdout.contains("carry"), "{}", out.stdout);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

#[test]
fn normal_004_stop_takes_a_child_back_to_its_pin() {
    let f = fixture("wt_stop", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "work").unwrap();

    // Uncommitted work: refused.
    let out = gs(&ws, &["develop", "--stop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stdout.contains("uncommitted"), "{}", out.stdout);

    run_git_pub(&child, &["checkout", "--", "lib.txt"]);
    let out = gs(&ws, &["develop", "--stop", "imports/core"]);
    ok(&out);
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert!(!git_ok(
        &child,
        &["rev-parse", "--verify", "-q", "refs/heads/feat/x"]
    ));
    // And it stays off the topic.
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child), None);
}

/// A branch the root pins builds every child from pins, even one whose remote
/// has a branch of that name.
#[test]
fn normal_005_a_pinned_branch_follows_no_topic() {
    let f = fixture(
        "wt_pinned",
        "[develop]\npinned = [\"main\", \"staging\"]\n\n",
    );
    let ws = f.clone_root("ws");
    f.core_commit("staging", "lib.txt", "staging");
    run_git_pub(&ws, &["switch", "-q", "-c", "staging"]);
    ok(&gs(&ws, &["pull"]));
    let child = ws.join("imports/core");
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stderr.contains("which it pins"), "{}", out.stderr);
}

/// A pipeline on the root's topic branch takes each child whose remote has
/// that branch from it — detached at its tip — and `check` fails until the
/// pins say so.
#[test]
fn normal_006_a_pipeline_on_a_topic_takes_children_from_their_branches() {
    let f = fixture("wt_ci_topic", "");
    let tip = f.core_commit("feat/x", "lib.txt", "remote work");
    // The runner's own checkout of the root, detached at the pipeline's
    // commit.
    let job = f.env.repos_remote.join("job");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            job.to_str().unwrap(),
        ],
    );
    run_git_pub(&job, &["checkout", "-q", "--detach"]);
    let out = ci_job(&job, &f.env.cache, "feat/x", &["pull"]);
    ok(&out);
    let child = job.join("imports/core");
    assert_eq!(head(&child), tip);
    assert_eq!(branch(&child), None);

    let out = ci_job(&job, &f.env.cache, "feat/x", &["check"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "imports/core was taken from branch feat/x, not from v1.0.0 pinned in .gitscale.toml"
        ),
        "{}",
        out.stderr
    );

    // On main nothing is followed, and the gate passes.
    let out = ci_job(&job, &f.env.cache, "main", &["pull"]);
    ok(&out);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    ok(&ci_job(&job, &f.env.cache, "main", &["check"]));
}

/// Leaving a topic never loses its commits: a developed child with commits
/// nobody pushed goes back to its pin when the root leaves, and back onto its
/// branch, commits and all, when the root returns — as the topics guide
/// promises.
#[test]
fn normal_014_leaving_and_rejoining_a_topic_keeps_unpushed_commits() {
    let f = fixture("develop_rejoin", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let pin = head(&child);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "unpushed work");

    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), pin);
    assert_eq!(git(&child, &["rev-parse", "refs/heads/feat/x"]), work);

    run_git_pub(&ws, &["switch", "-q", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), work);
    assert!(writable(&child.join("lib.txt")));
}

/// Edits made in a checkout before it is developed come along onto the topic
/// branch: `develop` is how they get somewhere they can be committed.
#[test]
fn normal_015_develop_carries_uncommitted_edits_onto_the_topic() {
    let f = fixture("develop_edits", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let pin = head(&child);
    crate::support::edit(&child.join("lib.txt"), "edited at the pin");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    ok(&gs(&ws, &["develop", "imports/core"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), pin);
    assert_eq!(
        std::fs::read_to_string(child.join("lib.txt")).unwrap(),
        "edited at the pin"
    );
    assert!(writable(&child.join("lib.txt")));
}

/// Developing a `replace` artefact that is installed as its image makes it a
/// worktree of its source, at the commit the image was built from, on the
/// topic branch — and a pull keeps it so.
#[test]
fn normal_016_develop_turns_an_installed_image_into_a_source_worktree() {
    let env = TestEnv::new("develop_artefact_image");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, "replace");
    ok(&gs(&ws, &["pull"]));
    let dest = ws.join("meta/app");
    assert!(!dest.join(".git").exists(), "installed as its image");
    let built = bare_git(&app, &["rev-parse", "main"]);

    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["develop", "meta/app"]);
    ok(&out);
    assert!(dest.join(".git").is_file(), "a source worktree");
    assert_eq!(branch(&dest).as_deref(), Some("feat/x"));
    assert_eq!(head(&dest), built);
    assert!(!dest.join("dist/app.bin").exists(), "the image is gone");

    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&dest).as_deref(), Some("feat/x"));
}

/// `develop --stop` on a developed artefact puts its image back: off the
/// topic an artefact is what its registry holds, not a source checkout.
#[test]
fn normal_017_stop_puts_an_artefact_back_to_its_image() {
    let env = TestEnv::new("develop_artefact_stop");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, "replace");
    ok(&gs(&ws, &["pull"]));
    let dest = ws.join("meta/app");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "meta/app"]));
    assert!(dest.join(".git").is_file());

    let out = gs(&ws, &["develop", "--stop", "meta/app"]);
    ok(&out);
    assert!(
        out.stdout.contains("meta/app left feat/x"),
        "{}",
        out.stdout
    );
    assert!(!dest.join(".git").exists(), "{}", out.stdout);
    assert_eq!(
        std::fs::read_to_string(dest.join("dist/app.bin")).unwrap(),
        "main build"
    );
    let store = store_for(&ws, &app);
    assert!(!has_ref(&store, "refs/heads/feat/x"));
}

/// A developed child whose topic branch a colleague pushed to is
/// fast-forwarded to it by the next pull.
#[test]
fn normal_018_pull_fast_forwards_a_developed_child_to_its_pushed_upstream() {
    let f = fixture("develop_ff", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    commit_in(&child, "lib.txt", "mine");
    ok(&gs(&ws, &["push"]));

    let theirs = f.core_commit("feat/x", "other.txt", "theirs");
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), theirs);
}

/// On a topic, `push` with no names pushes the root's topic branch too, as
/// its upstream; named, it pushes only the checkouts named.
#[test]
fn normal_019_push_pushes_the_root_topic_branch_only_when_no_names_are_given() {
    let f = fixture("develop_push_root", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    commit_in(&child, "lib.txt", "change");

    let out = gs(&ws, &["push", "imports/core"]);
    ok(&out);
    assert!(bare_has(&f.core, "refs/heads/feat/x"), "{}", out.stdout);
    assert!(!bare_has(&f.root, "refs/heads/feat/x"), "{}", out.stdout);

    let out = gs(&ws, &["push"]);
    ok(&out);
    assert!(
        out.stdout.contains(". (workspace root) → feat/x"),
        "{}",
        out.stdout
    );
    assert_eq!(bare_git(&f.root, &["rev-parse", "feat/x"]), head(&ws));
    assert_eq!(
        git(&ws, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/feat/x"
    );
}

/// A checkout can be named by the link a repository has to it: developing
/// `imports/b/libs/d` develops the hoisted `imports/d`.
#[test]
fn normal_020_develop_names_a_checkout_by_a_dependencys_link() {
    let env = TestEnv::new("develop_by_link");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "d v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    let b_config = format!(
        "[repos]\n\"libs/d\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let b = env.create_bare_repo("b", "main", &[(".gitscale.toml", &b_config)]);
    run_git_pub(&b, &["tag", "v1.0.0", "main"]);
    let root_config = format!(
        "[resolve]\nallow = [\"{}/*\"]\n\n[repos]\n\"imports/b\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        env.repos_remote.display(),
        b.display()
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &root_config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    ok(&gs(&ws, &["pull"]));
    assert!(ws.join("imports/b/libs/d").is_symlink());
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    let out = gs(&ws, &["develop", "imports/b/libs/d"]);
    ok(&out);
    assert!(out.stdout.contains("imports/d on feat/x"), "{}", out.stdout);
    assert_eq!(branch(&ws.join("imports/d")).as_deref(), Some("feat/x"));
}

/// A new branch from a topic carries an older major's suffixed branch too:
/// `feat/x@v1` becomes `feat/y@v1`, beside `feat/x` becoming `feat/y`.
#[test]
fn normal_021_a_new_branch_from_a_topic_carries_older_majors_by_their_suffix() {
    let env = TestEnv::new("develop_carry_majors");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    let v2 = env.push_commit(&d, "main", "d.txt", "v2");
    run_git_pub(&d, &["tag", "v2.0.0", &v2]);
    let config = format!(
        "[repos]\n\"imports/d\" = {{ url = \"{0}\", revision = \"v2.0.0\" }}\n\
         \"imports/d_v1\" = {{ url = \"{0}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    ok(&gs(&ws, &["pull"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/d", "imports/d_v1"]));

    run_git_pub(&ws, &["switch", "-q", "-c", "feat/y"]);
    let out = gs(&ws, &["pull"]);
    ok(&out);
    assert!(
        out.stdout.contains("carry  imports/d_v1 → feat/y"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&ws.join("imports/d")).as_deref(), Some("feat/y"));
    assert_eq!(
        branch(&ws.join("imports/d_v1")).as_deref(),
        Some("feat/y@v1")
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A child with uncommitted changes does not move when the root changes
/// branch: its entry fails, the rest goes on.
#[test]
fn edge_007_a_dirty_child_stays_when_the_root_leaves_the_topic() {
    let f = fixture("wt_dirty_leave", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let child = ws.join("imports/core");
    std::fs::write(child.join("lib.txt"), "unfinished").unwrap();

    run_git_pub(&ws, &["switch", "-q", "main"]);
    let out = gs(&ws, &["pull"]);
    assert!(!out.success);
    assert!(out.stderr.contains("uncommitted changes"), "{}", out.stderr);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

/// `pinned = []` pins nothing: the root's main follows every child's main.
#[test]
fn edge_008_an_empty_pinned_list_makes_the_default_branch_a_topic() {
    let f = fixture("wt_unpinned", "[develop]\npinned = []\n\n");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    assert_eq!(branch(&child).as_deref(), Some("main"));
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "main"]));
}

/// A dependency that pins the topic branch keeps its own dependencies at
/// their pins: the root builds what that dependency's pinned branch built.
#[test]
fn edge_009_a_dependency_that_pins_the_topic_keeps_what_it_asks_for_at_its_pins() {
    let env = TestEnv::new("wt_pinned_below");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "d v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    run_git_pub(&d, &["branch", "staging", "main"]);
    let staging_d = env.push_commit(&d, "staging", "d.txt", "d staging");
    let b_config = format!(
        "[develop]\npinned = [\"staging\"]\n\n[repos]\n\"libs/d\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let b = env.create_bare_repo("b", "main", &[(".gitscale.toml", &b_config)]);
    run_git_pub(&b, &["tag", "v1.0.0", "main"]);
    run_git_pub(&b, &["branch", "staging", "main"]);
    let root_config = format!(
        "[resolve]\nallow = [\"{}/*\"]\n\n[repos]\n\"imports/b\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        env.repos_remote.display(),
        b.display()
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &root_config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    run_git_pub(&ws, &["switch", "-q", "-c", "staging"]);
    ok(&gs(&ws, &["pull"]));

    // B follows staging; D, below B, stays at B's pin.
    assert_eq!(branch(&ws.join("imports/b")).as_deref(), Some("staging"));
    let child_d = ws.join("imports/d");
    assert_ne!(head(&child_d), staging_d);
    assert_eq!(head(&child_d), git(&d, &["rev-parse", "v1.0.0"]));
    assert!(
        status_row(&ws, "imports/d").contains("pinned by imports/b"),
        "{}",
        status_row(&ws, "imports/d")
    );
    let out = gs(&ws, &["develop", "imports/d"]);
    assert!(!out.success);
    assert!(
        out.stdout
            .contains("imports/b pins staging for its dependencies"),
        "{}",
        out.stdout
    );
}

/// Two checkouts of one repository share a store, so their topic branches
/// need names of their own: the newest major takes the topic's, the older
/// `<topic>@v<major>`.
#[test]
fn edge_010_an_older_major_develops_on_a_suffixed_branch() {
    let env = TestEnv::new("wt_majors");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    env.push_commit(&d, "main", "d.txt", "v2");
    let v2 = git(&d, &["rev-parse", "main"]);
    run_git_pub(&d, &["tag", "v2.0.0", &v2]);
    let config = format!(
        "[repos]\n\"imports/d\" = {{ url = \"{0}\", revision = \"v2.0.0\" }}\n\
         \"imports/d_v1\" = {{ url = \"{0}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    ok(&gs(&ws, &["pull"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    ok(&gs(&ws, &["develop", "imports/d", "imports/d_v1"]));
    assert_eq!(branch(&ws.join("imports/d")).as_deref(), Some("feat/x"));
    assert_eq!(
        branch(&ws.join("imports/d_v1")).as_deref(),
        Some("feat/x@v1")
    );
}

/// `develop` on a topic whose branch the store already has, before any pull
/// has put the checkout on it, puts it on that branch: a checkout left
/// detached and read-only while `develop` reports it developed invites
/// commits on no branch.
#[test]
#[ignore = "bug: develop says 'already on' a topic branch the checkout is not on"]
fn edge_022_develop_on_an_existing_topic_puts_the_checkout_on_its_branch() {
    let f = fixture("develop_rejoin_develop", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "work");
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["pull"]));

    // Back on the topic, without the hook's pull.
    run_git_pub(&ws, &["switch", "-q", "feat/x"]);
    let out = gs(&ws, &["develop", "imports/core"]);
    ok(&out);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"), "{}", out.stdout);
    assert_eq!(head(&child), work);
}

/// In a new root worktree on a topic developed elsewhere, `develop` before
/// any pull makes the checkout, on the topic branch — not just a message
/// that it is there.
#[test]
#[ignore = "bug: develop says 'already on' for a checkout that does not exist yet"]
fn edge_023_develop_in_a_new_root_worktree_of_a_topic_makes_the_checkout() {
    let f = fixture("develop_new_worktree", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let work = commit_in(&ws.join("imports/core"), "lib.txt", "work");
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["pull"]));

    let other = f.env.repos_remote.join("other");
    run_git_pub(
        &ws,
        &["worktree", "add", "-q", other.to_str().unwrap(), "feat/x"],
    );
    let out = gs(&other, &["develop", "imports/core"]);
    ok(&out);
    let child = other.join("imports/core");
    assert!(child.join(".git").is_file(), "{}", out.stdout);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), work);
}

/// Commits made at a detached pin are where `develop` starts the topic
/// branch, so they end up on it rather than lost.
#[test]
fn edge_024_develop_keeps_commits_made_at_a_detached_pin() {
    let f = fixture("develop_detached_commits", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let made = commit_in(&child, "lib.txt", "at the pin");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    ok(&gs(&ws, &["develop", "imports/core"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), made);
}

/// A developed branch that has diverged from its upstream is left where it
/// is: a fast-forward cannot reconcile it, and a pull never merges or resets
/// somebody's work.
#[test]
fn edge_025_pull_leaves_a_diverged_topic_branch_alone() {
    let f = fixture("develop_diverged", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    commit_in(&child, "lib.txt", "pushed");
    ok(&gs(&ws, &["push"]));
    f.core_commit("feat/x", "other.txt", "theirs");
    let mine = commit_in(&child, "lib.txt", "mine, not pushed");

    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), mine);
}

/// A child that followed its remote's topic branch keeps its local copy of
/// it once the remote deletes the branch: a local branch of the topic is
/// rule one of where a checkout goes. `develop --stop` then refuses, since
/// those commits are on no remote any more.
///
/// Current behaviour, pinned: whether a followed branch the remote deleted
/// should be left as the user's, or dropped, is the owner's call.
#[test]
fn edge_026_a_followed_branch_deleted_upstream_stays_as_a_local_branch() {
    let f = fixture("develop_followed_deleted", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let tip = f.core_commit("feat/x", "lib.txt", "remote work");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(head(&child), tip);

    bare_git(&f.core, &["branch", "-D", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), tip);

    let out = gs(&ws, &["develop", "--stop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stdout.contains("not pushed"), "{}", out.stdout);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

/// A detached root has no topic: a pull puts developed children back at
/// their pins, and their topic branches keep their commits.
#[test]
fn edge_027_pull_on_a_detached_root_puts_developed_children_at_their_pins() {
    let f = fixture("develop_detached_root", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let pin = head(&child);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "work");

    run_git_pub(&ws, &["checkout", "-q", "--detach"]);
    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), pin);
    assert!(!writable(&child.join("lib.txt")));
    assert_eq!(git(&child, &["rev-parse", "refs/heads/feat/x"]), work);
}

/// On a detached root `commit` commits the root, on no branch, as it does
/// on any root off a topic.
///
/// Current behaviour, pinned: the docs say off a topic only the root is
/// committed, which includes a detached root, where the commit lands on no
/// branch. Whether a detached root should be skipped instead is the owner's
/// call.
#[test]
fn edge_028_commit_on_a_detached_root_commits_the_root_on_no_branch() {
    let f = fixture("develop_detached_commit", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["checkout", "-q", "--detach"]);
    let before = head(&ws);
    std::fs::write(ws.join("README.md"), "edited").unwrap();

    let out = gs(&ws, &["commit", "-m", "detached root commit"]);
    ok(&out);
    assert!(out.stdout.contains(". (workspace root)"), "{}", out.stdout);
    assert_ne!(head(&ws), before);
    assert_eq!(branch(&ws), None);
    assert!(!on_some_branch(&ws, &head(&ws)));
}

/// With the newer major's entry and checkout gone, the older major becomes
/// the highest, and its branch of the topic is the topic's own name — which
/// the store already has, holding the newer major's work. The next pull
/// moves the older major's checkout onto it; its own work stays on
/// `feat/x@v1`.
///
/// Current behaviour, pinned: a checkout of v1 switching to a branch cut
/// from v2 is surprising, and whether a slot should keep its suffixed branch
/// is the owner's call.
#[test]
fn edge_029_dropping_the_newer_major_moves_the_older_onto_the_topics_own_branch() {
    let env = TestEnv::new("develop_drop_major");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    let v2 = env.push_commit(&d, "main", "d.txt", "v2");
    run_git_pub(&d, &["tag", "v2.0.0", &v2]);
    let both = format!(
        "[repos]\n\"imports/d\" = {{ url = \"{0}\", revision = \"v2.0.0\" }}\n\
         \"imports/d_v1\" = {{ url = \"{0}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &both)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    ok(&gs(&ws, &["pull"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/d", "imports/d_v1"]));
    let older = ws.join("imports/d_v1");
    let v1_work = commit_in(&older, "d.txt", "v1 work");

    let only_v1 = format!(
        "[repos]\n\"imports/d_v1\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    std::fs::write(ws.join(".gitscale.toml"), only_v1).unwrap();
    crate::support::make_writable(&ws.join("imports/d/d.txt"));
    std::fs::remove_dir_all(ws.join("imports/d")).unwrap();

    ok(&gs(&ws, &["pull"]));
    assert_eq!(branch(&older).as_deref(), Some("feat/x"));
    assert_eq!(head(&older), v2);
    assert_eq!(git(&older, &["rev-parse", "refs/heads/feat/x@v1"]), v1_work);
}

/// Developing a checkout that is already on the topic says so and changes
/// nothing.
#[test]
fn edge_030_developing_twice_says_it_is_already_on_the_topic() {
    let f = fixture("develop_twice", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "work");

    let out = gs(&ws, &["develop", "imports/core"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/core is already on feat/x"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), work);
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_011_refuses_off_a_topic() {
    let f = fixture("wt_develop_refusals", "");
    let ws = f.clone_root("ws");
    // On main, which the root pins.
    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("git switch -c <topic>"),
        "{}",
        out.stderr
    );
    // Detached.
    run_git_pub(&ws, &["checkout", "-q", "--detach"]);
    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stderr.contains("on no branch"), "{}", out.stderr);
}

#[test]
fn error_012_refuses_an_entry_the_root_overrides() {
    let f = fixture("wt_develop_override", "");
    let ws = f.clone_root("ws");
    let config = std::fs::read_to_string(ws.join(".gitscale.toml"))
        .unwrap()
        .replace(
            "revision = \"v1.0.0\"",
            "revision = \"v1.0.0\", override = true",
        );
    std::fs::write(ws.join(".gitscale.toml"), config).unwrap();
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stdout.contains("held by override"), "{}", out.stdout);
}

/// A topic branch the remote has keeps being followed: stopping refuses until
/// it is deleted there.
#[test]
fn error_013_stop_refuses_while_the_remote_has_the_branch() {
    let f = fixture("wt_stop_pushed", "");
    let ws = f.clone_root("ws");
    f.core_commit("feat/x", "lib.txt", "pushed");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["pull"]));
    let out = gs(&ws, &["develop", "--stop", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stdout.contains("Delete it there first"),
        "{}",
        out.stdout
    );
}

/// `develop --stop` refuses while the topic branch holds commits no remote
/// has: deleting the branch would lose them.
#[test]
fn error_031_stop_refuses_a_branch_with_unpushed_commits() {
    let f = fixture("develop_stop_unpushed", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "unpushed");

    let out = gs(&ws, &["develop", "--stop", "imports/core"]);
    assert!(!out.success);
    assert!(out.stdout.contains("1 commit not pushed"), "{}", out.stdout);
    assert!(
        out.stderr.contains("1 checkout not taken off the topic"),
        "{}",
        out.stderr
    );
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(git(&child, &["rev-parse", "refs/heads/feat/x"]), work);
}

/// `develop --stop` judges the topic branch, not only where the checkout is:
/// a checkout left detached at its pin (the root came back to the topic
/// without a pull) must not have its branch's unpushed commits deleted.
#[test]
#[ignore = "bug: develop --stop deletes unpushed commits on a topic branch the checkout is not on"]
fn error_032_stop_keeps_unpushed_commits_of_a_branch_the_checkout_is_not_on() {
    let f = fixture("develop_stop_detached", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "unpushed");
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["pull"]));
    // Back on the topic, without the hook's pull: the child is at its pin.
    run_git_pub(&ws, &["switch", "-q", "feat/x"]);

    let out = gs(&ws, &["develop", "--stop", "imports/core"]);
    assert!(
        has_ref(&child, "refs/heads/feat/x"),
        "the branch holding unpushed work was deleted: {}",
        out.stdout
    );
    assert_eq!(git(&child, &["rev-parse", "refs/heads/feat/x"]), work);
    assert!(!out.success, "{}", out.stdout);
}

/// Developing onto a topic branch the remote already has moves the
/// checkout to that branch; commits made at its detached pin must survive
/// that move — `pull` refuses the same move for exactly this reason.
#[test]
#[ignore = "bug: develop onto a remote topic branch orphans commits made at the detached pin"]
fn error_033_develop_onto_a_remote_topic_keeps_commits_made_at_the_pin() {
    let f = fixture("develop_remote_orphan", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let made = commit_in(&child, "mine.txt", "made at the pin");
    f.core_commit("feat/x", "lib.txt", "remote work");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(
        head(&child) == made || on_some_branch(&child, &made),
        "commit {} is on no branch any more: {}{}",
        made,
        out.stdout,
        out.stderr
    );
}

#[test]
fn error_034_develop_without_a_directory_fails() {
    let f = fixture("develop_no_dir", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["develop"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("name the checkout to develop"),
        "{}",
        out.stderr
    );
}

/// One name that matches no checkout fails, and the others are still
/// developed.
#[test]
fn error_035_an_unknown_directory_fails_and_the_rest_are_developed() {
    let f = fixture("develop_unknown", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["develop", "imports/nope", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stdout
            .contains("FAIL  imports/nope: no checkout at imports/nope"),
        "{}",
        out.stdout
    );
    assert!(
        out.stderr.contains("1 checkout not developed"),
        "{}",
        out.stderr
    );
    assert_eq!(branch(&ws.join("imports/core")).as_deref(), Some("feat/x"));
}

/// CI checkouts are copies of exact commits: `develop` refuses there.
#[test]
fn error_036_develop_refuses_in_ci() {
    let f = fixture("develop_ci", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = ci_job(&ws, &f.env.cache, "feat/x", &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("develop works on a developer machine"),
        "{}",
        out.stderr
    );
    assert_eq!(branch(&ws.join("imports/core")), None);
}

/// A topic whose name git cannot store beside a branch the store already has
/// — `feat/x` after `feat` — fails that checkout and leaves it at its pin.
#[test]
fn error_037_a_topic_clashing_with_a_store_branch_fails_and_leaves_the_checkout() {
    let f = fixture("develop_ref_clash", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let pin = head(&child);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat"]);
    ok(&gs(&ws, &["develop", "imports/core"]));
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["pull"]));
    run_git_pub(&ws, &["branch", "-q", "-D", "feat"]);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    let out = gs(&ws, &["develop", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stdout.contains("cannot create branch feat/x"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), pin);
}
