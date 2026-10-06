//! Topics: the root's branch is the topic, `git topic join` / `leave` put a
//! checkout on it and take it off, and `start`, `switch`, `status`, `list` and
//! `finish` begin, go to, show and end topics.

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

    let out = gs(&ws, &["topic", "join", "imports/core"]);
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
    let table = strip_ansi(&gs(&ws, &["ls"]).stdout);
    assert!(table.starts_with("topic feat/x"), "{}", table);

    // A sync keeps it there.
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

/// The remote has the topic branch: a sync puts the child on it, writable,
/// tracking the remote's.
#[test]
fn normal_002_a_child_follows_its_remote_branch_of_the_topic() {
    let f = fixture("wt_follow", "");
    let ws = f.clone_root("ws");
    let tip = f.core_commit("feat/x", "lib.txt", "remote work");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["sync"]));
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
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert!(!writable(&child.join("lib.txt")));
}

/// `git switch -c feat/y` from topic feat/x carries the joined children to
/// feat/y, at the commits they are at, edits included.
#[test]
fn normal_003_a_new_branch_from_a_topic_carries_its_children() {
    let f = fixture("wt_carry", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "committed").unwrap();
    run_git_pub(&child, &["commit", "-q", "-am", "x work"]);
    std::fs::write(child.join("lib.txt"), "uncommitted").unwrap();
    let at = head(&child);

    run_git_pub(&ws, &["switch", "-q", "-c", "feat/y"]);
    let out = gs(&ws, &["sync"]);
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
    let out = gs(&ws, &["sync"]);
    ok(&out);
    assert!(!out.stdout.contains("carry"), "{}", out.stdout);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

#[test]
fn normal_004_leave_takes_a_child_back_to_its_pin() {
    let f = fixture("wt_stop", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let child = ws.join("imports/core");
    identity(&child);
    std::fs::write(child.join("lib.txt"), "work").unwrap();

    // Uncommitted work: refused.
    let out = gs(&ws, &["topic", "leave", "imports/core"]);
    assert!(!out.success);
    assert!(out.stderr.contains("uncommitted"), "{}", out.stderr);

    run_git_pub(&child, &["checkout", "--", "lib.txt"]);
    let out = gs(&ws, &["topic", "leave", "imports/core"]);
    ok(&out);
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert!(!git_ok(
        &child,
        &["rev-parse", "--verify", "-q", "refs/heads/feat/x"]
    ));
    // And it stays off the topic.
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child), None);
}

/// A branch the root pins builds every child from pins, even one whose remote
/// has a branch of that name.
#[test]
fn normal_005_a_pinned_branch_follows_no_topic() {
    let f = fixture(
        "wt_pinned",
        "[branches]\npinned = [\"main\", \"staging\"]\n\n",
    );
    let ws = f.clone_root("ws");
    f.core_commit("staging", "lib.txt", "staging");
    run_git_pub(&ws, &["switch", "-q", "-c", "staging"]);
    ok(&gs(&ws, &["sync"]));
    let child = ws.join("imports/core");
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    let out = gs(&ws, &["topic", "join", "imports/core"]);
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
    let out = ci_job(&job, &f.env.cache, "feat/x", &["sync"]);
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
    let out = ci_job(&job, &f.env.cache, "main", &["sync"]);
    ok(&out);
    assert_eq!(head(&child), git(&f.core, &["rev-parse", "v1.0.0"]));
    ok(&ci_job(&job, &f.env.cache, "main", &["check"]));
}

/// Leaving a topic never loses its commits: a joined child with commits
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
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "unpushed work");

    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), pin);
    assert_eq!(git(&child, &["rev-parse", "refs/heads/feat/x"]), work);

    run_git_pub(&ws, &["switch", "-q", "feat/x"]);
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), work);
    assert!(writable(&child.join("lib.txt")));
}

/// Edits made in a checkout before it is joined come along onto the topic
/// branch: `git topic join` is how they get somewhere they can be committed.
#[test]
fn normal_015_join_carries_uncommitted_edits_onto_the_topic() {
    let f = fixture("develop_edits", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let pin = head(&child);
    crate::support::edit(&child.join("lib.txt"), "edited at the pin");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), pin);
    assert_eq!(
        std::fs::read_to_string(child.join("lib.txt")).unwrap(),
        "edited at the pin"
    );
    assert!(writable(&child.join("lib.txt")));
}

/// Joining a checkout that is installed as its image makes it a
/// worktree of its source, at the commit the image was built from, on the
/// topic branch — and a sync keeps it so.
#[test]
fn normal_016_join_turns_an_installed_image_into_a_source_worktree() {
    let env = TestEnv::new("develop_artefact_image");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, gitscale::prefer::Form::Artefact);
    ok(&gs(&ws, &["sync"]));
    let dest = ws.join("meta/app");
    assert!(!dest.join(".git").exists(), "installed as its image");
    let built = bare_git(&app, &["rev-parse", "main"]);

    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["topic", "join", "meta/app"]);
    ok(&out);
    assert!(dest.join(".git").is_file(), "a source worktree");
    assert_eq!(branch(&dest).as_deref(), Some("feat/x"));
    assert_eq!(head(&dest), built);
    assert!(!dest.join("dist/app.bin").exists(), "the image is gone");

    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&dest).as_deref(), Some("feat/x"));
}

/// `git topic leave` on a joined artefact puts its image back: off the
/// topic an artefact is what its registry holds, not a source checkout.
#[test]
fn normal_017_leave_puts_an_artefact_back_to_its_image() {
    let env = TestEnv::new("develop_artefact_stop");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    let ws = artefact_workspace(&env, &app, gitscale::prefer::Form::Artefact);
    ok(&gs(&ws, &["sync"]));
    let dest = ws.join("meta/app");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "meta/app"]));
    assert!(dest.join(".git").is_file());

    let out = gs(&ws, &["topic", "leave", "meta/app"]);
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

/// A joined child whose topic branch a colleague pushed to is
/// fast-forwarded to it by the next sync.
#[test]
fn normal_018_sync_fast_forwards_a_joined_child_to_its_pushed_upstream() {
    let f = fixture("develop_ff", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    commit_in(&child, "lib.txt", "mine");
    ok(&gs(&ws, &["push"]));

    let theirs = f.core_commit("feat/x", "other.txt", "theirs");
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), theirs);
}

/// On a topic, `git scale push` pushes the root's topic branch too, as its
/// upstream; with `-s`, only the checkouts named.
#[test]
fn normal_019_push_pushes_the_root_topic_branch_unless_s_names_others() {
    let f = fixture("develop_push_root", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    commit_in(&child, "lib.txt", "change");

    let out = gs(&ws, &["--for", "imports/core", "push", "--quiet"]);
    ok(&out);
    assert!(bare_has(&f.core, "refs/heads/feat/x"), "{}", out.stdout);
    assert!(!bare_has(&f.root, "refs/heads/feat/x"), "{}", out.stdout);

    let out = gs(&ws, &["push", "--quiet"]);
    ok(&out);
    assert_eq!(bare_git(&f.root, &["rev-parse", "feat/x"]), head(&ws));
    assert_eq!(
        git(&ws, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/feat/x"
    );
}

/// A checkout can be named by the link a repository has to it: joining
/// `imports/b/libs/d` joins the hoisted `imports/d`.
#[test]
fn normal_020_join_names_a_checkout_by_a_dependencys_link() {
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
    ok(&gs(&ws, &["sync"]));
    assert!(ws.join("imports/b/libs/d").is_symlink());
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    let out = gs(&ws, &["topic", "join", "imports/b/libs/d"]);
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
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/d", "imports/d_v1"]));

    run_git_pub(&ws, &["switch", "-q", "-c", "feat/y"]);
    let out = gs(&ws, &["sync"]);
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
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let child = ws.join("imports/core");
    std::fs::write(child.join("lib.txt"), "unfinished").unwrap();

    run_git_pub(&ws, &["switch", "-q", "main"]);
    let out = gs(&ws, &["sync"]);
    assert!(!out.success);
    assert!(out.stderr.contains("uncommitted changes"), "{}", out.stderr);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

/// `pinned = []` pins nothing: the root's main follows every child's main.
#[test]
fn edge_008_an_empty_pinned_list_makes_the_default_branch_a_topic() {
    let f = fixture("wt_unpinned", "[branches]\npinned = []\n\n");
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
        "[branches]\npinned = [\"staging\"]\n\n[repos]\n\"libs/d\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
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
    ok(&gs(&ws, &["sync"]));

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
    let out = gs(&ws, &["topic", "join", "imports/d"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("imports/b pins staging for its dependencies"),
        "{}",
        out.stderr
    );
}

/// Two checkouts of one repository share a store, so their topic branches
/// need names of their own: the newest major takes the topic's, the older
/// `<topic>@v<major>`.
#[test]
fn edge_010_an_older_major_joins_on_a_suffixed_branch() {
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
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    ok(&gs(&ws, &["topic", "join", "imports/d", "imports/d_v1"]));
    assert_eq!(branch(&ws.join("imports/d")).as_deref(), Some("feat/x"));
    assert_eq!(
        branch(&ws.join("imports/d_v1")).as_deref(),
        Some("feat/x@v1")
    );
}

/// `git topic join` on a topic whose branch the store already has, before any
/// placement has put the checkout on it, puts it on that branch: a checkout
/// left detached and read-only while `git topic join` reports it joined
/// invites commits on no branch.
#[test]
#[ignore = "bug: join says 'already on' a topic branch the checkout is not on"]
fn edge_022_join_on_an_existing_topic_puts_the_checkout_on_its_branch() {
    let f = fixture("develop_rejoin_develop", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "work");
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["sync"]));

    // Back on the topic, without the hook's placement.
    run_git_pub(&ws, &["switch", "-q", "feat/x"]);
    let out = gs(&ws, &["topic", "join", "imports/core"]);
    ok(&out);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"), "{}", out.stdout);
    assert_eq!(head(&child), work);
}

/// In a new root worktree on a topic joined elsewhere, `git topic join`
/// before any placement makes the checkout, on the topic branch — not just a
/// message that it is there.
#[test]
#[ignore = "bug: join says 'already on' for a checkout that does not exist yet"]
fn edge_023_join_in_a_new_root_worktree_of_a_topic_makes_the_checkout() {
    let f = fixture("develop_new_worktree", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let work = commit_in(&ws.join("imports/core"), "lib.txt", "work");
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["sync"]));

    let other = f.env.repos_remote.join("other");
    run_git_pub(
        &ws,
        &["worktree", "add", "-q", other.to_str().unwrap(), "feat/x"],
    );
    let out = gs(&other, &["topic", "join", "imports/core"]);
    ok(&out);
    let child = other.join("imports/core");
    assert!(child.join(".git").is_file(), "{}", out.stdout);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), work);
}

/// Commits made at a detached pin are where `git topic join` starts the topic
/// branch, so they end up on it rather than lost.
#[test]
fn edge_024_join_keeps_commits_made_at_a_detached_pin() {
    let f = fixture("develop_detached_commits", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let made = commit_in(&child, "lib.txt", "at the pin");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), made);
}

/// A topic branch that has diverged from its upstream is left where it
/// is: a fast-forward cannot reconcile it, and a sync never merges or resets
/// somebody's work.
#[test]
fn edge_025_sync_leaves_a_diverged_topic_branch_alone() {
    let f = fixture("develop_diverged", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    commit_in(&child, "lib.txt", "pushed");
    ok(&gs(&ws, &["push"]));
    f.core_commit("feat/x", "other.txt", "theirs");
    let mine = commit_in(&child, "lib.txt", "mine, not pushed");

    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), mine);
}

/// A child that followed its remote's topic branch keeps its local copy of
/// it once the remote deletes the branch: a local branch of the topic is
/// rule one of where a checkout goes. `git topic leave` then refuses, since
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
    ok(&gs(&ws, &["sync"]));
    assert_eq!(head(&child), tip);

    bare_git(&f.core, &["branch", "-D", "feat/x"]);
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(head(&child), tip);

    let out = gs(&ws, &["topic", "leave", "imports/core"]);
    assert!(!out.success);
    assert!(out.stderr.contains("not pushed"), "{}", out.stderr);
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
}

/// A detached root has no topic: a sync puts joined children back at
/// their pins, and their topic branches keep their commits.
#[test]
fn edge_027_sync_on_a_detached_root_puts_joined_children_at_their_pins() {
    let f = fixture("develop_detached_root", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let pin = head(&child);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "work");

    run_git_pub(&ws, &["checkout", "-q", "--detach"]);
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), pin);
    assert!(!writable(&child.join("lib.txt")));
    assert_eq!(git(&child, &["rev-parse", "refs/heads/feat/x"]), work);
}

/// On a detached root `commit` commits the root, on no branch, as it does
/// on any root off a topic.
///
/// Off a topic a git command runs in the root alone — a detached root too,
/// where a commit lands on no branch, as git's own would.
#[test]
fn edge_028_commit_on_a_detached_root_commits_the_root_on_no_branch() {
    let f = fixture("develop_detached_commit", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["checkout", "-q", "--detach"]);
    let before = head(&ws);
    std::fs::write(ws.join("README.md"), "edited").unwrap();

    let out = gs(&ws, &["commit", "-qam", "detached root commit"]);
    ok(&out);
    assert!(out.stdout.starts_with("── 1/1 . "), "{}", out.stdout);
    assert_ne!(head(&ws), before);
    assert_eq!(branch(&ws), None);
    assert!(!on_some_branch(&ws, &head(&ws)));
}

/// With the newer major's entry and checkout gone, the older major becomes
/// the highest, and its branch of the topic is the topic's own name — which
/// the store already has, holding the newer major's work. The next sync
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
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/d", "imports/d_v1"]));
    let older = ws.join("imports/d_v1");
    let v1_work = commit_in(&older, "d.txt", "v1 work");

    let only_v1 = format!(
        "[repos]\n\"imports/d_v1\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    std::fs::write(ws.join(".gitscale.toml"), only_v1).unwrap();
    crate::support::make_writable(&ws.join("imports/d/d.txt"));
    std::fs::remove_dir_all(ws.join("imports/d")).unwrap();

    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&older).as_deref(), Some("feat/x"));
    assert_eq!(head(&older), v2);
    assert_eq!(git(&older, &["rev-parse", "refs/heads/feat/x@v1"]), v1_work);
}

/// Joining a checkout that is already on the topic says so and changes
/// nothing.
#[test]
fn edge_030_joining_twice_says_it_is_already_on_the_topic() {
    let f = fixture("develop_twice", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "work");

    let out = gs(&ws, &["topic", "join", "imports/core"]);
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
    let out = gs(&ws, &["topic", "join", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("git topic start <name>"),
        "{}",
        out.stderr
    );
    // Detached.
    run_git_pub(&ws, &["checkout", "-q", "--detach"]);
    let out = gs(&ws, &["topic", "join", "imports/core"]);
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
    let out = gs(&ws, &["topic", "join", "imports/core"]);
    assert!(!out.success);
    assert!(out.stderr.contains("held by override"), "{}", out.stderr);
}

/// A topic branch the remote has keeps being followed: stopping refuses until
/// it is deleted there.
#[test]
fn error_013_leave_refuses_while_the_remote_has_the_branch() {
    let f = fixture("wt_stop_pushed", "");
    let ws = f.clone_root("ws");
    f.core_commit("feat/x", "lib.txt", "pushed");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["sync"]));
    let out = gs(&ws, &["topic", "leave", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("Delete it there first"),
        "{}",
        out.stdout
    );
}

/// `git topic leave` refuses while the topic branch holds commits no remote
/// has: deleting the branch would lose them.
#[test]
fn error_031_leave_refuses_a_branch_with_unpushed_commits() {
    let f = fixture("develop_stop_unpushed", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "unpushed");

    let out = gs(&ws, &["topic", "leave", "imports/core"]);
    assert!(!out.success);
    assert!(out.stderr.contains("1 commit not pushed"), "{}", out.stderr);
    assert!(
        out.stderr.contains("1 checkout not taken off the topic"),
        "{}",
        out.stderr
    );
    assert_eq!(branch(&child).as_deref(), Some("feat/x"));
    assert_eq!(git(&child, &["rev-parse", "refs/heads/feat/x"]), work);
}

/// `git topic leave` judges the topic branch, not only where the checkout is:
/// a checkout left detached at its pin (the root came back to the topic
/// without a placement) must not have its branch's unpushed commits deleted.
#[test]
#[ignore = "bug: leave deletes unpushed commits on a topic branch the checkout is not on"]
fn error_032_leave_keeps_unpushed_commits_of_a_branch_the_checkout_is_not_on() {
    let f = fixture("develop_stop_detached", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let work = commit_in(&child, "lib.txt", "unpushed");
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["sync"]));
    // Back on the topic, without the hook's placement: the child is at its pin.
    run_git_pub(&ws, &["switch", "-q", "feat/x"]);

    let out = gs(&ws, &["topic", "leave", "imports/core"]);
    assert!(
        has_ref(&child, "refs/heads/feat/x"),
        "the branch holding unpushed work was deleted: {}",
        out.stdout
    );
    assert_eq!(git(&child, &["rev-parse", "refs/heads/feat/x"]), work);
    assert!(!out.success, "{}", out.stdout);
}

/// Joining onto a topic branch the remote already has moves the
/// checkout to that branch; commits made at its detached pin must survive
/// that move — `sync` refuses the same move for exactly this reason.
#[test]
#[ignore = "bug: join onto a remote topic branch orphans commits made at the detached pin"]
fn error_033_join_onto_a_remote_topic_keeps_commits_made_at_the_pin() {
    let f = fixture("develop_remote_orphan", "");
    let ws = f.clone_root("ws");
    let child = ws.join("imports/core");
    let made = commit_in(&child, "mine.txt", "made at the pin");
    f.core_commit("feat/x", "lib.txt", "remote work");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    let out = gs(&ws, &["topic", "join", "imports/core"]);
    assert!(
        head(&child) == made || on_some_branch(&child, &made),
        "commit {} is on no branch any more: {}{}",
        made,
        out.stdout,
        out.stderr
    );
}

#[test]
fn error_034_join_without_a_directory_fails() {
    let f = fixture("develop_no_dir", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["topic", "join"]);
    assert!(!out.success);
    assert!(out.stderr.contains("no checkout named"), "{}", out.stderr);
}

/// One name that matches no checkout fails, and the others still join.
#[test]
fn error_035_an_unknown_directory_fails_and_the_rest_are_joined() {
    let f = fixture("develop_unknown", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["topic", "join", "imports/nope", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("FAIL  imports/nope: imports/nope is not a checkout of this workspace"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("1 checkout not joined to the topic"),
        "{}",
        out.stderr
    );
    assert_eq!(branch(&ws.join("imports/core")).as_deref(), Some("feat/x"));
}

/// CI checkouts are copies of exact commits: `git topic join` refuses there.
#[test]
fn error_036_join_refuses_in_ci() {
    let f = fixture("develop_ci", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = ci_job(
        &ws,
        &f.env.cache,
        "feat/x",
        &["topic", "join", "imports/core"],
    );
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("git topic join works on a developer machine"),
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
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["branch", "-q", "-D", "feat"]);
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    let out = gs(&ws, &["topic", "join", "imports/core"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("cannot create branch feat/x"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&child), None);
    assert_eq!(head(&child), pin);
}

// ---------------------------------------------------------------------------
// git topic: the topic of the checkout the current directory is in
// ---------------------------------------------------------------------------

/// `git topic` prints the topic: the root's branch in the root, the slot's
/// own branch in a child — `<topic>@v<major>` for an older major's — and
/// nothing, exiting 1, off a topic.
#[test]
fn normal_038_topic_prints_the_branch_of_the_checkout_it_is_run_in() {
    let env = TestEnv::new("topic_print");
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
    ok(&gs(&ws, &["sync"]));

    let out = gs(&ws, &["topic"]);
    assert!(!out.success);
    assert_eq!((out.stdout.as_str(), out.stderr.as_str()), ("", ""));

    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = gs(&ws, &["topic"]);
    ok(&out);
    assert_eq!(out.stdout, "feat/x\n");
    let out = gs(&ws.join("imports/d"), &["topic"]);
    ok(&out);
    assert_eq!(out.stdout, "feat/x\n");
    let out = gs(&ws.join("imports/d_v1"), &["topic"]);
    ok(&out);
    assert_eq!(out.stdout, "feat/x@v1\n");
}

/// Off a topic, `git topic` prints nothing for a script to read, but tells a
/// person at a terminal where they are, on stderr: on a pinned branch, on no
/// branch, or in a checkout the topic holds at its pin.
#[test]
fn normal_063_off_a_topic_a_terminal_is_told_where_it_is() {
    let env = TestEnv::new("topic_print_terminal");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    let config = format!(
        "[repos]\n\"imports/d\" = {{ url = \"{}\", revision = \"v1.0.0\", override = true }}\n",
        d.display()
    );
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", &config)]);
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    ok(&gs(&ws, &["sync"]));
    let at_terminal = |dir: &std::path::Path| {
        let full = ["gitscale", "-C", dir.to_str().unwrap(), "topic"];
        gitscale::run_cli_with(&full, true)
    };

    let out = at_terminal(&ws);
    assert!(!out.success);
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "main is pinned, not a topic: git topic start NAME, or git topic switch NAME\n"
    );

    run_git_pub(&ws, &["switch", "-q", "--detach"]);
    let out = at_terminal(&ws);
    assert!(!out.success);
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.starts_with("on no branch, not a topic"),
        "{}",
        out.stderr
    );

    // The root's override holds imports/d at its pin on any topic.
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    let out = at_terminal(&ws.join("imports/d"));
    assert!(!out.success);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "imports/d is held at its pin, not on feat/x\n");

    // A script reads stdout, and sees only the answer.
    let out = at_terminal(&ws);
    ok(&out);
    assert_eq!((out.stdout.as_str(), out.stderr.as_str()), ("feat/x\n", ""));
}

/// `git topic status` asks the remotes first: a merge of the root's branch
/// since the last fetch is in its answer. `--offline` reads this machine
/// only, and at a terminal says how old that is, once it is an hour or more.
#[test]
fn normal_064_status_fetches_first_and_offline_says_how_old_it_is() {
    let f = fixture("topic_status_fetches", "");
    let ws = f.clone_root("ws");
    identity(&ws);
    ok(&gs(&ws, &["topic", "start", "feat"]));
    commit_in(&ws, "notes.txt", "the change");
    run_git_pub(&ws, &["push", "-q", "-u", "origin", "feat"]);
    let root_row = |stdout: &str| {
        stdout
            .lines()
            .find(|l| l.split_whitespace().next() == Some("."))
            .unwrap_or_default()
            .to_string()
    };

    squash_merge(&f.env, &f.root, "feat");
    let offline = gs(&ws, &["topic", "status", "--offline"]);
    ok(&offline);
    assert!(
        root_row(&offline.stdout).ends_with("ready to merge"),
        "{}",
        offline.stdout
    );
    let out = gs(&ws, &["topic", "status"]);
    ok(&out);
    assert!(
        root_row(&out.stdout).ends_with("merged into main"),
        "{}",
        out.stdout
    );
    // The default, still accepted from scripts that pass it.
    ok(&gs(&ws, &["topic", "status", "--fetch"]));
    ok(&gs(&ws, &["topic", "list", "--fetch"]));

    let fetch_head = ws.join(".git/FETCH_HEAD");
    let aged = std::process::Command::new("touch")
        .args(["-d", "3 hours ago", fetch_head.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(aged.success());
    let at_terminal = |args: &[&str]| {
        let mut full = vec!["gitscale", "-C", ws.to_str().unwrap()];
        full.extend_from_slice(args);
        gitscale::run_cli_with(&full, true)
    };
    let out = at_terminal(&["topic", "status", "--offline"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("fetched 3 h ago: git topic status fetches first without --offline"),
        "{}",
        out.stdout
    );
    let out = at_terminal(&["topic", "list", "--offline"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("fetched 3 h ago: git topic list fetches first without --offline"),
        "{}",
        out.stdout
    );
    // Not when it fetched, and never off a terminal.
    let out = at_terminal(&["topic", "status"]);
    ok(&out);
    assert!(!out.stdout.contains("fetched"), "{}", out.stdout);
    let out = gs(&ws, &["topic", "status", "--offline"]);
    assert!(!out.stdout.contains("fetched"), "{}", out.stdout);
}

/// A root merged with a merge commit, rather than a squash, has every one of
/// its commits in the default branch's history: it is merged, as a topic
/// just started — on the same commit as the default branch — is not.
#[test]
fn normal_065_a_merge_commit_counts_as_merged() {
    let f = fixture("topic_status_merge_commit", "");
    let ws = f.clone_root("ws");
    identity(&ws);
    ok(&gs(&ws, &["topic", "start", "feat"]));
    commit_in(&ws, "notes.txt", "the change");
    run_git_pub(&ws, &["push", "-q", "-u", "origin", "feat"]);

    let work = f.env.repos_remote.join("merge-commit");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    identity(&work);
    run_git_pub(
        &work,
        &["merge", "-q", "--no-ff", "-m", "merge feat", "origin/feat"],
    );
    run_git_pub(&work, &["push", "-q", "origin", "main"]);

    let out = gs(&ws, &["topic", "status"]);
    ok(&out);
    let root = out
        .stdout
        .lines()
        .find(|l| l.split_whitespace().next() == Some("."))
        .unwrap_or_default();
    assert!(root.ends_with("merged into main"), "{}", out.stdout);
    assert!(
        out.stdout.contains("next: git topic finish"),
        "{}",
        out.stdout
    );
}

/// `join` and `leave` with nothing named fail, as `git add` does; inside a
/// checkout the hint says how to name it.
#[test]
fn error_039_join_and_leave_need_a_directory_and_hint_inside_a_checkout() {
    let f = fixture("topic_join_hint", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    for verb in ["join", "leave"] {
        let out = gs(&ws, &["topic", verb]);
        assert!(!out.success);
        assert!(out.stderr.contains("no checkout named"), "{}", out.stderr);
        assert!(!out.stderr.contains("hint:"), "{}", out.stderr);

        let out = gs(&ws.join("imports/core"), &["topic", verb]);
        assert!(!out.success);
        assert!(
            out.stderr.contains(&format!(
                "hint: inside a checkout, use: git topic {} .",
                verb
            )),
            "{}",
            out.stderr
        );
    }
    // And `.` there does it.
    ok(&gs(&ws.join("imports/core"), &["topic", "join", "."]));
    assert_eq!(branch(&ws.join("imports/core")).as_deref(), Some("feat/x"));
}

/// Directory arguments are paths from the current directory: `.` in a
/// checkout, a relative path from a subdirectory; a path out of the
/// workspace, or the root where a checkout is wanted, fails.
#[test]
fn normal_040_directories_are_paths_from_the_current_directory() {
    let f = fixture("topic_paths", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws.join("imports"), &["topic", "join", "core"]));
    assert_eq!(branch(&ws.join("imports/core")).as_deref(), Some("feat/x"));
    ok(&gs(&ws.join("imports/core"), &["topic", "leave", "."]));
    assert_eq!(branch(&ws.join("imports/core")), None);

    let out = gs(&ws, &["topic", "join", "../elsewhere"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("../elsewhere is outside the workspace"),
        "{}",
        out.stderr
    );
    let out = gs(&ws, &["topic", "join", "."]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("the root is not a checkout"),
        "{}",
        out.stderr
    );
}

// ---------------------------------------------------------------------------
// start, switch, status, list, finish
// ---------------------------------------------------------------------------

/// Squash `branch` of the bare repository `bare` into its `main`, as a merge
/// request would, and push it. Returns the new commit.
fn squash_merge(env: &TestEnv, bare: &std::path::Path, branch: &str) -> String {
    let work = env.repos_remote.join("merge-tmp");
    let _ = std::fs::remove_dir_all(&work);
    run_git_pub(
        &env.repos_remote,
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    identity(&work);
    run_git_pub(
        &work,
        &["merge", "-q", "--squash", &format!("origin/{}", branch)],
    );
    run_git_pub(
        &work,
        &["commit", "-q", "-m", &format!("squashed {}", branch)],
    );
    run_git_pub(&work, &["push", "-q", "origin", "main"]);
    let commit = git(&work, &["rev-parse", "HEAD"]);
    let _ = std::fs::remove_dir_all(&work);
    commit
}

/// The binary run in `dir` with `vars` set and `unset` removed, `HOME` the
/// test's own: for what reads the environment, as `{user}` does.
fn topic_bin(
    env: &TestEnv,
    dir: &std::path::Path,
    args: &[&str],
    vars: &[(&str, &str)],
    unset: &[&str],
) -> crate::support::CliOutput {
    let home = env.repos_remote.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"));
    cmd.args(["-C", dir.to_str().unwrap()])
        .args(args)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"));
    for name in unset {
        cmd.env_remove(name);
    }
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let output = cmd.output().unwrap();
    crate::support::CliOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        success: output.status.success(),
    }
}

/// A bare clone of `bare` at `parent/.git`, as `git clone --bare` makes one.
fn bare_clone(bare: &std::path::Path, parent: &std::path::Path) {
    std::fs::create_dir_all(parent).unwrap();
    run_git_pub(
        parent,
        &["clone", "-q", "--bare", bare.to_str().unwrap(), ".git"],
    );
    identity(parent);
}

/// In a plain clone, `start` makes the branch from the remote's default
/// branch, not tracking it, and places the children.
#[test]
fn normal_041_start_in_a_plain_clone_branches_from_the_remote_default() {
    let f = fixture("topic_start_plain", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "elsewhere"]);
    std::fs::write(ws.join("note.txt"), "x").unwrap();
    run_git_pub(&ws, &["add", "note.txt"]);
    run_git_pub(&ws, &["commit", "-q", "-m", "elsewhere"]);

    let out = gs(&ws, &["topic", "start", "PROJ-1"]);
    ok(&out);
    assert!(
        out.stdout.contains("started PROJ-1 from origin/main"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&ws).as_deref(), Some("PROJ-1"));
    assert_eq!(head(&ws), git(&ws, &["rev-parse", "origin/main"]));
    assert!(!git_ok(
        &ws,
        &["rev-parse", "--verify", "-q", "@{upstream}"]
    ));
    assert!(ws.join("imports/core/lib.txt").is_file());
}

/// `start` refuses a name that exists here or on the remote, and one the
/// root pins.
#[test]
fn error_042_start_refuses_existing_and_pinned_names() {
    let f = fixture(
        "topic_start_refuse",
        "[branches]\npinned = [\"main\", \"release/*\"]\n\n",
    );
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["branch", "local-one"]);
    f.env.push_commit(&f.root, "main", "x.txt", "x");
    run_git_pub(&f.root, &["branch", "remote-one", "main"]);
    for (name, message) in [
        ("local-one", "local-one exists: git topic switch local-one"),
        (
            "remote-one",
            "remote-one exists: git topic switch remote-one",
        ),
        ("release/1", "release/1 is pinned, not a topic"),
    ] {
        let out = gs(&ws, &["topic", "start", name]);
        assert!(!out.success, "{}", name);
        assert!(out.stderr.contains(message), "{}: {}", name, out.stderr);
    }
    assert_eq!(branch(&ws).as_deref(), Some("main"));
}

/// `--from` a topic carries the children joined to it.
#[test]
fn normal_043_start_from_a_topic_carries_its_joined_children() {
    let f = fixture("topic_start_from", "");
    let ws = f.clone_root("ws");
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let work = commit_in(&ws.join("imports/core"), "lib.txt", "x work");
    run_git_pub(&ws, &["switch", "-q", "main"]);
    ok(&gs(&ws, &["sync"]));
    assert_eq!(branch(&ws.join("imports/core")), None);

    ok(&gs(&ws, &["topic", "start", "--from", "feat/x", "feat/y"]));
    assert_eq!(branch(&ws.join("imports/core")).as_deref(), Some("feat/y"));
    assert_eq!(head(&ws.join("imports/core")), work);
}

/// In a bare clone, `start` adds a worktree beside the others, named after
/// the branch with every `/` a `-`, and says where to go; an existing
/// directory is refused, and `--dir` chooses another.
#[test]
fn normal_044_start_in_a_bare_clone_adds_a_worktree() {
    let f = fixture("topic_start_bare", "");
    let app = f.env.repos_remote.join("app");
    bare_clone(&f.root, &app);

    let out = gs(&app, &["topic", "start", "feat/retry"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("configured origin to fetch remote branches"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("created feat-retry"), "{}", out.stdout);
    assert!(out.stdout.ends_with("cd feat-retry\n"), "{}", out.stdout);
    let wt = app.join("feat-retry");
    assert_eq!(branch(&wt).as_deref(), Some("feat/retry"));
    assert!(wt.join("imports/core/lib.txt").is_file());

    std::fs::create_dir_all(app.join("feat-other")).unwrap();
    let out = gs(&app, &["topic", "start", "feat/other"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("feat-other exists; choose another with --dir"),
        "{}",
        out.stderr
    );
    let out = gs(
        &app,
        &["topic", "start", "--dir", "elsewhere", "feat/other"],
    );
    ok(&out);
    assert_eq!(
        branch(&app.join("elsewhere")).as_deref(),
        Some("feat/other")
    );
}

/// `--worktree` in a plain clone adds the topic's worktree beside the clone.
#[test]
fn normal_045_start_with_worktree_in_a_plain_clone_goes_beside_it() {
    let f = fixture("topic_start_worktree", "");
    let ws = f.clone_root("app");
    let out = gs(&ws, &["topic", "start", "--worktree", "feat-blah"]);
    ok(&out);
    let wt = f.env.repos_remote.join("app-feat-blah");
    assert!(out.stdout.contains("cd ../app-feat-blah"), "{}", out.stdout);
    assert_eq!(branch(&wt).as_deref(), Some("feat-blah"));
    assert!(wt.join("imports/core/lib.txt").is_file());
    assert_eq!(branch(&ws).as_deref(), Some("main"));
}

/// `switch` goes to a colleague's branch, tracking the remote's, and the
/// children with that branch join it; a pinned branch works too; a name
/// that exists nowhere is refused with the `start` hint.
#[test]
fn normal_046_switch_goes_to_a_colleagues_topic_and_back() {
    let f = fixture("topic_switch_plain", "");
    let ws = f.clone_root("ws");
    run_git_pub(&f.root, &["branch", "PROJ-9", "main"]);
    let theirs = f.core_commit("PROJ-9", "lib.txt", "colleague");

    let out = gs(&ws, &["topic", "switch", "PROJ-9"]);
    ok(&out);
    assert_eq!(branch(&ws).as_deref(), Some("PROJ-9"));
    assert_eq!(
        git(&ws, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/PROJ-9"
    );
    assert_eq!(branch(&ws.join("imports/core")).as_deref(), Some("PROJ-9"));
    assert_eq!(head(&ws.join("imports/core")), theirs);

    ok(&gs(&ws, &["topic", "switch", "main"]));
    assert_eq!(branch(&ws.join("imports/core")), None);

    let out = gs(&ws, &["topic", "switch", "nowhere"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("nowhere does not exist: git topic start nowhere"),
        "{}",
        out.stderr
    );
}

/// In a bare clone, `switch` adds a worktree for the branch — after which a
/// colleague's new branch is found, the remote branches being fetched now —
/// or, when one is already on it, says where it is.
#[test]
fn normal_047_switch_in_a_bare_clone_adds_or_finds_the_worktree() {
    let f = fixture("topic_switch_bare", "");
    let app = f.env.repos_remote.join("app");
    bare_clone(&f.root, &app);

    let out = gs(&app, &["topic", "switch", "main"]);
    ok(&out);
    assert!(
        out.stdout
            .contains("configured origin to fetch remote branches"),
        "{}",
        out.stdout
    );
    assert!(app.join("main/imports/core/lib.txt").is_file());
    // Set up once.
    let out = gs(&app.join("main"), &["topic", "list"]);
    ok(&out);
    assert!(!out.stdout.contains("configured origin"), "{}", out.stdout);

    run_git_pub(&f.root, &["branch", "PROJ-9", "main"]);
    let out = gs(&app.join("main"), &["topic", "switch", "PROJ-9"]);
    ok(&out);
    assert!(out.stdout.contains("cd ../PROJ-9"), "{}", out.stdout);
    assert_eq!(branch(&app.join("PROJ-9")).as_deref(), Some("PROJ-9"));

    let out = gs(&app, &["topic", "switch", "PROJ-9"]);
    ok(&out);
    assert_eq!(out.stdout, "cd PROJ-9\n");
}

/// `[topic] prefix`: `{user}` is `gitscale.user`, else `$USER`; with
/// neither, `start` says how to set one. A name already prefixed is not
/// prefixed again, the worktree's directory drops the prefix, and
/// `switch` and `finish` take the name either way.
#[test]
fn normal_048_a_branch_prefix_names_the_branch_and_not_the_directory() {
    let f = fixture("topic_prefix", "[topic]\nprefix = \"{user}/\"\n\n");
    let app = f.env.repos_remote.join("app");
    bare_clone(&f.root, &app);

    let out = topic_bin(
        &f.env,
        &app,
        &["topic", "start", "feat-blah"],
        &[],
        &["USER", "USERNAME"],
    );
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("set your name for branches: git config --global gitscale.user NAME"),
        "{}",
        out.stderr
    );

    let out = topic_bin(
        &f.env,
        &app,
        &["topic", "start", "feat-blah"],
        &[("USER", "andrey")],
        &[],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        branch(&app.join("feat-blah")).as_deref(),
        Some("andrey/feat-blah")
    );

    run_git_pub(&app, &["config", "gitscale.user", "ann"]);
    let out = topic_bin(
        &f.env,
        &app,
        &["topic", "start", "ann/hotfix/retry"],
        &[("USER", "andrey")],
        &[],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        branch(&app.join("hotfix-retry")).as_deref(),
        Some("ann/hotfix/retry")
    );

    let out = topic_bin(&f.env, &app, &["topic", "switch", "hotfix/retry"], &[], &[]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(out.stdout, "cd hotfix-retry\n");
    let out = topic_bin(
        &f.env,
        &app,
        &["topic", "finish", "--force", "hotfix/retry"],
        &[],
        &[],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!app.join("hotfix-retry").exists());
}

/// Without `[topic] prefix` nothing is added.
#[test]
fn normal_049_without_a_prefix_nothing_is_added() {
    let f = fixture("topic_no_prefix", "");
    let ws = f.clone_root("ws");
    let out = topic_bin(
        &f.env,
        &ws,
        &["topic", "start", "plain"],
        &[("USER", "andrey")],
        &[],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(branch(&ws).as_deref(), Some("plain"));
}

/// `status` shows the root and the joined children, with what each still
/// needs, the merge order and the next command; off a topic it fails.
#[test]
fn normal_050_status_says_what_each_joined_repository_still_needs() {
    let f = fixture("topic_status", "");
    let ws = f.clone_root("ws");
    let out = gs(&ws, &["topic", "status"]);
    assert!(!out.success);
    assert!(out.stderr.contains("not on a topic"), "{}", out.stderr);

    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let out = gs(&ws, &["topic", "status"]);
    ok(&out);
    assert!(out.stdout.starts_with("topic feat\n"), "{}", out.stdout);
    let row = |stdout: &str, repo: &str| {
        stdout
            .lines()
            .find(|l| l.split_whitespace().next() == Some(repo))
            .unwrap_or_default()
            .to_string()
    };
    assert!(
        row(&out.stdout, "imports/core").ends_with("no change yet"),
        "{}",
        out.stdout
    );
    assert!(
        row(&out.stdout, ".").ends_with("ready to merge"),
        "{}",
        out.stdout
    );

    commit_in(&ws.join("imports/core"), "lib.txt", "change");
    let out = gs(&ws, &["topic", "status"]);
    ok(&out);
    let core = row(&out.stdout, "imports/core");
    assert!(core.contains(" 1 ") && core.contains(" no "), "{}", core);
    assert!(core.ends_with("no tag"), "{}", core);
    assert!(
        row(&out.stdout, ".").ends_with("waits on imports/core"),
        "{}",
        out.stdout
    );
    // What to run comes first: the branch is pushed before it is merged.
    assert!(
        out.stdout
            .contains("next: git scale push\nthen merge: imports/core\n"),
        "{}",
        out.stdout
    );

    ok(&gs(&ws, &["push", "--quiet"]));
    squash_merge(&f.env, &f.core, "feat");
    let merged = git(&f.core, &["rev-parse", "main"]);
    run_git_pub(&f.core, &["tag", "v1.1.0", &merged]);
    let out = gs(&ws, &["topic", "status", "--fetch"]);
    ok(&out);
    assert!(
        row(&out.stdout, "imports/core").ends_with("released as v1.1.0"),
        "{}",
        out.stdout
    );
    assert!(
        row(&out.stdout, ".").ends_with("ready to merge"),
        "{}",
        out.stdout
    );
    // The promotion is the next step: nothing is left to merge before it.
    assert!(
        out.stdout.contains("\nnext: git upgrade --commit"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("next to merge"), "{}", out.stdout);
    // `ls` says the same in its first line.
    let out = gs(&ws, &["ls"]);
    ok(&out);
    assert_eq!(
        out.stdout.lines().next(),
        Some("topic feat · next: git upgrade --commit, then merge: ."),
        "{}",
        out.stdout
    );

    let out = gs(&ws, &["topic", "status", "--format", "json"]);
    ok(&out);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(json["topic"], "feat");
    assert_eq!(json["repos"][1]["repo"], "imports/core");
    assert_eq!(json["repos"][1]["state"], "released");
    assert_eq!(json["repos"][1]["pushed"], true);
}

/// `list` shows every topic: the current one marked, its worktree, its
/// joined children and whether it is pushed or merged — a squash merge
/// counting.
#[test]
fn normal_051_list_shows_every_topic_and_where_it_stands() {
    let f = fixture("topic_list", "");
    let ws = f.clone_root("app");
    ok(&gs(&ws, &["topic", "start", "--worktree", "other"]));
    // The clone's own worktree is on main: pinned, and listed all the same.
    let out = gs(&ws, &["topic", "list"]);
    ok(&out);
    let main = out
        .stdout
        .lines()
        .find(|l| l.split_whitespace().next() == Some("▸") || l.trim_start().starts_with("main"))
        .unwrap_or_default()
        .to_string();
    assert!(
        main.starts_with("▸ main") && main.contains(" . ") && main.ends_with(" -"),
        "{}",
        out.stdout
    );
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    commit_in(&ws.join("imports/core"), "lib.txt", "change");

    let out = gs(&ws, &["topic", "list"]);
    ok(&out);
    let row = |topic: &str| {
        out.stdout
            .lines()
            .find(|l| l.split_whitespace().any(|w| w == topic))
            .unwrap_or_default()
            .to_string()
    };
    assert!(row("feat").starts_with("▸ feat"), "{}", out.stdout);
    assert!(row("feat").contains("imports/core"), "{}", out.stdout);
    assert!(row("feat").ends_with("1 not pushed"), "{}", out.stdout);
    assert!(row("other").contains("../app-other"), "{}", out.stdout);
    // No worktree is on main any more: a pinned branch is listed only where
    // it is checked out.
    assert!(row("main").is_empty(), "{}", out.stdout);

    ok(&gs(&ws, &["push", "--quiet"]));
    let out = gs(&ws, &["topic", "list"]);
    ok(&out);
    assert!(
        out.stdout
            .lines()
            .any(|l| l.starts_with("▸ feat") && l.ends_with("pushed")),
        "{}",
        out.stdout
    );

    commit_in(&ws, "root.txt", "root change");
    ok(&gs(&ws, &["--for", ".", "push", "--quiet"]));
    squash_merge(&f.env, &f.root, "feat");
    let out = gs(&ws, &["topic", "list", "--fetch"]);
    ok(&out);
    assert!(
        out.stdout
            .lines()
            .any(|l| l.starts_with("▸ feat") && l.ends_with("merged")),
        "{}",
        out.stdout
    );
}

/// `finish` refuses a topic not merged and one with a child still on it —
/// `--force` drops either — and uncommitted changes; then, merged, the clone
/// is back on main with every child at its pin, the branches gone here and
/// kept on the remotes.
#[test]
fn normal_052_finish_ends_a_merged_topic_in_a_plain_clone() {
    let f = fixture("topic_finish_plain", "");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    commit_in(&ws.join("imports/core"), "lib.txt", "change");
    commit_in(&ws, "root.txt", "root change");
    ok(&gs(&ws, &["push", "--quiet"]));

    let out = gs(&ws, &["topic", "finish"]);
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("feat is not merged into main; --force to drop it"),
        "{}",
        out.stderr
    );
    squash_merge(&f.env, &f.root, "feat");
    let out = gs(&ws, &["topic", "finish"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "imports/core is still on the topic: git upgrade, or git topic leave imports/core\n\
             --force takes them back to their pins"
        ),
        "{}",
        out.stderr
    );

    // Released and promoted, the child leaves the topic.
    squash_merge(&f.env, &f.core, "feat");
    let merged = git(&f.core, &["rev-parse", "main"]);
    run_git_pub(&f.core, &["tag", "v1.1.0", &merged]);
    ok(&gs(&ws, &["upgrade"]));
    run_git_pub(&f.core, &["update-ref", "-d", "refs/heads/feat"]);
    std::fs::write(ws.join("README.md"), "uncommitted").unwrap();
    let out = gs(&ws, &["topic", "finish"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "uncommitted changes in .: commit or stash them (git scale stash -u), or \
             discard them (git scale reset --hard && git scale clean -fd)"
        ),
        "{}",
        out.stderr
    );
    run_git_pub(&ws, &["checkout", "--", "README.md", ".gitscale.toml"]);

    let out = gs(&ws, &["topic", "finish"]);
    ok(&out);
    // Promotion took the child's branch already.
    assert!(out.stdout.contains("deleted feat in .\n"), "{}", out.stdout);
    assert_eq!(branch(&ws).as_deref(), Some("main"));
    assert!(!git_ok(
        &ws,
        &["rev-parse", "--verify", "-q", "refs/heads/feat"]
    ));
    assert!(
        bare_has(&f.root, "refs/heads/feat"),
        "the remote branch stays"
    );
    assert_eq!(branch(&ws.join("imports/core")), None);
    assert_eq!(
        head(&ws.join("imports/core")),
        git(&f.core, &["rev-parse", "v1.0.0"])
    );
}

/// `finish --force` drops a topic not merged: the local branch goes, its
/// commit no remote has named with the tip it was at, and the remote's
/// branch stays.
#[test]
fn normal_053_finish_force_drops_an_unmerged_topic() {
    let f = fixture("topic_finish_force", "");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    commit_in(&ws, "root.txt", "root change");
    ok(&gs(&ws, &["push", "--quiet"]));
    commit_in(&ws, "root.txt", "unpushed change");
    let tip = git(&ws, &["rev-parse", "--short", "HEAD"]);

    let out = gs(&ws, &["topic", "finish", "--force"]);
    ok(&out);
    assert_eq!(branch(&ws).as_deref(), Some("main"));
    assert!(
        out.stdout.contains(&format!(
            "dropped feat in . (was {}): 1 commit no remote had\n",
            tip
        )),
        "{}",
        out.stdout
    );
    assert!(!git_ok(
        &ws,
        &["rev-parse", "--verify", "-q", "refs/heads/feat"]
    ));
    assert!(bare_has(&f.root, "refs/heads/feat"));
}

/// In a bare clone, `finish` removes the topic's worktree, says where to go
/// when it was run from inside it, and deletes the branch.
#[test]
fn normal_054_finish_in_a_bare_clone_removes_the_worktree() {
    let f = fixture("topic_finish_bare", "");
    let app = f.env.repos_remote.join("app");
    bare_clone(&f.root, &app);
    ok(&gs(&app, &["topic", "switch", "main"]));
    ok(&gs(&app, &["topic", "start", "PROJ-13"]));
    let wt = app.join("PROJ-13");
    commit_in(&wt, "root.txt", "root change");
    ok(&gs(&wt, &["push", "--quiet"]));
    squash_merge(&f.env, &f.root, "PROJ-13");

    let out = gs(&wt.join("imports"), &["topic", "finish"]);
    ok(&out);
    assert!(!wt.exists());
    assert!(out.stdout.ends_with("cd ../../main\n"), "{}", out.stdout);
    assert!(!git_ok(
        &app,
        &["rev-parse", "--verify", "-q", "refs/heads/PROJ-13"]
    ));
    assert!(app.join("main/imports/core/lib.txt").is_file());
    assert!(bare_has(&f.root, "refs/heads/PROJ-13"));
}

/// `finish --force` abandons a topic with a child still on it: the child goes
/// back to its pin, uncommitted changes refused first, and its commit no
/// remote has is dropped. `switch` brings back what was pushed.
#[test]
fn normal_057_finish_force_abandons_a_topic() {
    let f = fixture("topic_abandon", "");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let core = ws.join("imports/core");
    commit_in(&core, "lib.txt", "core work");
    let pushed = head(&core);
    commit_in(&ws, "root.txt", "root change");
    ok(&gs(&ws, &["push", "--quiet"]));
    commit_in(&core, "lib.txt", "more core work");

    std::fs::write(core.join("lib.txt"), "uncommitted").unwrap();
    let out = gs(&ws, &["topic", "finish", "--force"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "uncommitted changes in imports/core: commit or stash them (git scale stash -u), or \
             discard them (git scale reset --hard && git scale clean -fd)"
        ),
        "{}",
        out.stderr
    );
    // The discard the refusal names.
    ok(&gs(&ws, &["reset", "--hard", "--quiet"]));

    let out = gs(&ws, &["topic", "finish", "--force"]);
    ok(&out);
    assert!(out.stdout.contains("deleted feat in .\n"), "{}", out.stdout);
    assert!(
        out.stdout.contains("dropped feat in imports/core (was "),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("): 1 commit no remote had\n"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&ws).as_deref(), Some("main"));
    assert_eq!(branch(&core), None);
    assert_eq!(head(&core), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert!(!gs(&ws, &["topic", "list"]).stdout.contains("feat"));

    ok(&gs(&ws, &["topic", "switch", "feat"]));
    assert_eq!(branch(&core).as_deref(), Some("feat"));
    assert_eq!(head(&core), pushed);
}

/// In a bare clone, `finish --force` removes the worktree of an abandoned
/// topic with its checkouts, and drops the child's branch from the store.
#[test]
fn edge_058_finish_force_in_a_bare_clone_drops_a_childs_branch() {
    let f = fixture("topic_abandon_bare", "");
    let app = f.env.repos_remote.join("app");
    bare_clone(&f.root, &app);
    ok(&gs(&app, &["topic", "switch", "main"]));
    ok(&gs(&app, &["topic", "start", "PROJ-14"]));
    let wt = app.join("PROJ-14");
    ok(&gs(&wt, &["topic", "join", "imports/core"]));
    commit_in(&wt.join("imports/core"), "lib.txt", "core work");

    let out = gs(&app, &["topic", "finish", "--force", "PROJ-14"]);
    ok(&out);
    assert!(!wt.exists());
    assert!(
        out.stdout.contains("deleted PROJ-14 in .\n"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("dropped PROJ-14 in imports/core (was "),
        "{}",
        out.stdout
    );
    let store = app.join("main/imports/core");
    assert!(!git_ok(
        &store,
        &["rev-parse", "--verify", "-q", "refs/heads/PROJ-14"]
    ));
}

/// A merged topic still has a branch in a store with a commit no remote
/// has — a major's own, left behind: `finish` refuses to drop it unless
/// `--force`.
#[test]
fn edge_059_finish_refuses_to_drop_commits_no_remote_has() {
    let f = fixture("topic_finish_unpushed", "");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat"]));
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    let core = ws.join("imports/core");
    commit_in(&core, "lib.txt", "left behind");
    run_git_pub(&core, &["branch", "feat@v7"]);
    run_git_pub(&core, &["reset", "--hard", "--quiet", "HEAD~1"]);
    ok(&gs(&ws, &["topic", "leave", "imports/core"]));
    commit_in(&ws, "root.txt", "root change");
    ok(&gs(&ws, &["push", "--quiet"]));
    squash_merge(&f.env, &f.root, "feat");

    let out = gs(&ws, &["topic", "finish"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "feat@v7 in imports/core has 1 commit no remote has\n\
             push them first, or --force to drop them"
        ),
        "{}",
        out.stderr
    );
    assert_eq!(branch(&ws).as_deref(), Some("feat"));

    let out = gs(&ws, &["topic", "finish", "--force"]);
    ok(&out);
    assert!(out.stdout.contains("deleted feat in .\n"), "{}", out.stdout);
    assert!(
        out.stdout.contains("dropped feat@v7 in imports/core (was "),
        "{}",
        out.stdout
    );
}

/// `start` while on another topic begins afresh from the default branch:
/// nothing joined to the old topic comes along — only `--from` carries.
#[test]
fn edge_055_start_while_on_a_topic_carries_nothing_without_from() {
    let f = fixture("topic_start_no_carry", "");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "feat/x"]));
    ok(&gs(&ws, &["topic", "join", "imports/core"]));
    commit_in(&ws.join("imports/core"), "lib.txt", "x work");

    ok(&gs(&ws, &["topic", "start", "feat/y"]));
    let core = ws.join("imports/core");
    assert_eq!(branch(&core), None);
    assert_eq!(head(&core), git(&f.core, &["rev-parse", "v1.0.0"]));
    assert!(!git_ok(
        &core,
        &["rev-parse", "--verify", "-q", "refs/heads/feat/y"]
    ));
}

/// A topic with nothing on it yet is `new` in `list`, and `status` shows no
/// push state for a repository with nothing to push.
#[test]
fn normal_056_a_topic_with_nothing_on_it_is_new() {
    let f = fixture("topic_new", "");
    let ws = f.clone_root("ws");
    ok(&gs(&ws, &["topic", "start", "fresh"]));
    ok(&gs(&ws, &["topic", "join", "imports/core"]));

    let out = gs(&ws, &["topic", "list"]);
    ok(&out);
    assert!(
        out.stdout
            .lines()
            .any(|l| l.starts_with("▸ fresh") && l.ends_with(" new")),
        "{}",
        out.stdout
    );
    let out = gs(&ws, &["topic", "status"]);
    ok(&out);
    for repo in [".", "imports/core"] {
        let row = out
            .stdout
            .lines()
            .find(|l| l.split_whitespace().next() == Some(repo))
            .unwrap();
        let cells: Vec<&str> = row.split_whitespace().collect();
        assert_eq!((cells[2], cells[3]), ("0", "-"), "{}", out.stdout);
    }

    commit_in(&ws.join("imports/core"), "lib.txt", "work");
    let out = gs(&ws, &["topic", "list"]);
    ok(&out);
    assert!(
        out.stdout
            .lines()
            .any(|l| l.starts_with("▸ fresh") && l.ends_with("1 not pushed")),
        "{}",
        out.stdout
    );
}

/// A topic's worktree deleted by hand: `list` shows the topic with no
/// worktree, and `switch` makes it again, the joined checkout back on its
/// branch with the commit no remote has.
#[test]
fn edge_060_a_worktree_deleted_by_hand_comes_back_with_switch() {
    let f = fixture("topic_worktree_deleted", "");
    let app = f.env.repos_remote.join("app");
    bare_clone(&f.root, &app);
    ok(&gs(&app, &["topic", "switch", "main"]));
    ok(&gs(&app, &["topic", "start", "PROJ-15"]));
    let wt = app.join("PROJ-15");
    ok(&gs(&wt, &["topic", "join", "imports/core"]));
    commit_in(&wt.join("imports/core"), "lib.txt", "core work");
    let work = head(&wt.join("imports/core"));
    std::fs::remove_dir_all(&wt).unwrap();

    let out = gs(&app, &["topic", "list"]);
    ok(&out);
    assert!(
        out.stdout
            .lines()
            .any(|l| l.starts_with("  PROJ-15   -   ") && l.ends_with("1 not pushed")),
        "{}",
        out.stdout
    );

    let out = gs(&app.join("main"), &["topic", "switch", "PROJ-15"]);
    ok(&out);
    assert!(out.stdout.ends_with("cd ../PROJ-15\n"), "{}", out.stdout);
    assert_eq!(branch(&wt.join("imports/core")).as_deref(), Some("PROJ-15"));
    assert_eq!(head(&wt.join("imports/core")), work);
}

/// `git topic join --dependants <dir>` joins the checkouts whose configs ask
/// for less than the dependency's newest release — how a raise begins — and
/// leaves those already asking for it.
#[test]
fn normal_061_join_dependants_of_a_dependency_joins_those_asking_for_less() {
    use crate::support::resolution::{allow, dependant, repos, root_workspace, tagged};
    let env = TestEnv::new("topic_dependants_raise");
    let d = tagged(&env, "d", &[("v1.0.0", ""), ("v1.1.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.1.0\"")]);
    let ws = root_workspace(
        &env,
        &format!(
            "{}{}",
            allow(&env),
            repos(&[
                ("imports/b", &b, ", revision = \"v1.0.0\""),
                ("imports/c", &c, ", revision = \"v1.0.0\""),
            ])
        ),
    );
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/raise"]);

    let out = gs(&ws, &["topic", "join", "--dependants", "imports/d"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/b on feat/raise"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("imports/c"), "{}", out.stdout);
    assert_eq!(branch(&ws.join("imports/b")).as_deref(), Some("feat/raise"));
    assert_eq!(branch(&ws.join("imports/c")), None);
    assert_eq!(branch(&ws.join("imports/d")), None);
}

/// `git topic join --dependants` alone climbs one level from the topic's
/// changes: the dependants of each checkout carrying one — commits on the
/// topic, or uncommitted work — while none of them is on the topic. One taken
/// off stays off, a checkout joined with nothing to carry is not climbed
/// from, and the root is never joined.
#[test]
fn normal_062_join_dependants_climbs_one_level_from_the_topics_changes() {
    use crate::support::resolution::{allow, dependant, repos, root_workspace, tagged};
    let env = TestEnv::new("topic_dependants_climb");
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = dependant(&env, "b", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let c = dependant(&env, "c", &[("libs/d", &d, ", revision = \"v1.0.0\"")]);
    let x = dependant(&env, "x", &[("libs/b", &b, ", revision = \"v1.0.0\"")]);
    let ws = root_workspace(
        &env,
        &format!(
            "{}{}",
            allow(&env),
            repos(&[
                ("imports/c", &c, ", revision = \"v1.0.0\""),
                ("imports/x", &x, ", revision = \"v1.0.0\""),
            ])
        ),
    );
    ok(&gs(&ws, &["sync"]));
    run_git_pub(&ws, &["switch", "-q", "-c", "feat/climb"]);
    let out = gs(&ws, &["topic", "join", "--dependants"]);
    ok(&out);
    assert!(out.stdout.contains("nothing to join"), "{}", out.stdout);

    ok(&gs(&ws, &["topic", "join", "imports/d"]));
    let d_dir = ws.join("imports/d");
    identity(&d_dir);
    std::fs::write(d_dir.join("VERSION"), "changed").unwrap();
    run_git_pub(&d_dir, &["commit", "-q", "-am", "the change"]);
    let out = gs(&ws, &["topic", "join", "--dependants"]);
    ok(&out);
    for joined in ["imports/b on feat/climb", "imports/c on feat/climb"] {
        assert!(out.stdout.contains(joined), "{}: {}", joined, out.stdout);
    }
    assert_eq!(branch(&ws.join("imports/x")), None);

    ok(&gs(&ws, &["topic", "leave", "imports/c"]));
    let out = gs(&ws, &["topic", "join", "--dependants"]);
    ok(&out);
    assert!(out.stdout.contains("nothing to join"), "{}", out.stdout);
    assert_eq!(branch(&ws.join("imports/c")), None);

    std::fs::write(ws.join("imports/b/VERSION"), "in progress").unwrap();
    let out = gs(&ws, &["topic", "join", "--dependants"]);
    ok(&out);
    assert!(
        out.stdout.contains("imports/x on feat/climb"),
        "{}",
        out.stdout
    );
    assert_eq!(branch(&ws.join("imports/c")), None);
    assert_eq!(branch(&ws).as_deref(), Some("feat/climb"));
}
