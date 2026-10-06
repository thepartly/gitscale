//! `git scale ls`: the table and the JSON, one row per checkout.

use crate::support;
use crate::support::artefacts::entry_config;
use crate::support::resolution::{diamond, repos};
use crate::support::status_clean::*;
use crate::support::workspace::*;
use crate::support::worktrees::{fixture, gs};
use crate::support::{redact_shas, strip_ansi, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_table_shows_a_missing_checkout() {
    let env = TestEnv::new("status_table_missed");
    env.init_playground_git();
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run(&["ls"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    insta::assert_snapshot!("ls_table_missed_stdout", plain);
    // The JSON says the same: nothing there yet.
    let row = json_row(&env, "libs/mylib");
    assert_eq!(row["exists"], false, "{}", row);
    assert_eq!(row["current_ref"], "", "{}", row);
}

#[test]
fn normal_002_table_shows_a_checkout_at_its_pin() {
    let env = TestEnv::new("status_table_ok");
    env.init_playground_git();
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    env.run(&["sync"]);
    let out = env.run(&["ls"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = redact_shas(&strip_ansi(&out.stdout));
    insta::assert_snapshot!("ls_table_ok_stdout", plain);
}

#[test]
fn normal_003_json_lists_each_checkout() {
    let env = TestEnv::new("status_json");
    env.init_playground_git();
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    env.run(&["sync"]);
    let out = env.run(&["ls", "--format", "json"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // Parse to validate JSON, then snapshot
    let parsed: serde_json::Value = serde_json::from_str(&out.stdout).expect("valid JSON");
    // Redact dynamic fields for stable snapshots
    insta::assert_json_snapshot!("ls_json_output", parsed, {
        "[].current_ref" => "[ref]",
        "[].resolved_commit" => "[commit]",
        "[].requests[].commit" => "[commit]",
        "[].source_hash" => "[hash]",
    });
}

#[test]
fn normal_004_table_shows_a_missing_artefact() {
    let env = TestEnv::new("status_artefact_missed");
    env.init_playground_git();
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    support::run_git_pub(&bare, &["tag", "v1.0.0", "main"]);

    env.prefer(&bare, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "v1.0.0" }}
"#,
        env.registries(),
        bare.display(),
    ));

    let out = env.run(&["ls"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    insta::assert_snapshot!("ls_artefact_missed_stdout", plain);
}

#[test]
fn normal_005_table_shows_an_installed_artefact() {
    let env = TestEnv::new("status_artefact_ok");
    env.init_playground_git();
    let bare = env.artefact_repo("app", &[("app.bin", "content")]);

    env.prefer(&bare, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        r#"{}[repos]
"meta/app" = {{ url = "{}", revision = "v1.0.0" }}
"#,
        env.registries(),
        bare.display(),
    ));

    assert!(env.run(&["sync"]).success);
    let out = env.run(&["ls"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = redact_shas(&strip_ansi(&out.stdout));
    insta::assert_snapshot!("ls_artefact_ok_stdout", plain);
}

/// The other side of the same rule: detached somewhere the revision does not
/// point is a mismatch, which comparing the two columns as text used to miss
/// for exactly the repos that are pinned.
#[test]
fn normal_006_flags_a_pin_the_checkout_has_not_followed() {
    let env = TestEnv::new("status_tag_moved");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "v1")]);
    support::run_git_pub(&bare, &["tag", "demo-v1", "main"]);
    // Both tags exist before the clone, so the checkout knows demo-v2 and can
    // be told it is not on it. A revision the clone has never heard of is a
    // different thing, and `ls` says nothing about those rather than
    // guessing.
    commit_to_bare(&bare, "main", "a.txt", "v2");
    support::run_git_pub(&bare, &["tag", "demo-v2", "main"]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "demo-v1" }}
"#,
        bare.display()
    ));
    assert!(env.run(&["sync"]).success);

    // The config moves on to the later tag; the checkout has not.
    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "demo-v2" }}
"#,
        bare.display()
    ));

    let out = env.run(&["ls", "--color", "always"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    assert!(
        plain.contains("ref-mismatch"),
        "the checkout is not where the revision points: {}",
        plain
    );
    assert_eq!(
        status_cell(&out.stdout, "libs/mylib"),
        "ref-mismatch",
        "{}",
        plain
    );
    assert_eq!(
        icon_and_colour(&out.stdout, "libs/mylib"),
        ("≠".to_string(), "91".to_string())
    );
    // The wrong ref is painted yellow, so it shows without reading STATUS.
    let line = out
        .stdout
        .lines()
        .find(|l| l.contains("libs/mylib"))
        .unwrap();
    let head = support::git_stdout(
        &env.playground.join("libs/mylib"),
        &["rev-parse", "--short", "HEAD"],
    );
    assert!(
        line.contains(&format!("\x1b[33m{}", head)),
        "REF is not yellow: {:?}",
        line
    );
}

/// Uncommitted work alone: `dirty` in STATUS with the bright red `!`, and
/// `clean: false` in the JSON.
#[test]
fn normal_010_a_dirty_checkout_in_the_table_and_the_json() {
    let env = TestEnv::new("status_dirty");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.write_config(&mylib_config(&bare, "main"));
    assert!(env.run(&["sync"]).success);
    support::edit(&env.playground.join("libs/mylib/a.txt"), "edited");

    let out = env.run(&["ls", "--color", "always"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(
        status_cell(&out.stdout, "libs/mylib"),
        "dirty",
        "{}",
        out.stdout
    );
    assert_eq!(
        icon_and_colour(&out.stdout, "libs/mylib"),
        ("!".to_string(), "91".to_string())
    );
    let row = json_row(&env, "libs/mylib");
    assert_eq!(row["clean"], false, "{}", row);
    assert_eq!(row["exists"], true, "{}", row);
}

/// A topic branch never pushed has no upstream: its commits that no remote
/// branch or tag has are its `+N` — work only this machine holds. Pushed,
/// they are counted against the upstream again, and none is left.
#[test]
fn normal_033_a_topic_branch_never_pushed_is_ahead_by_what_no_remote_has() {
    let (env, _bare, clone) = setup_commit_env("status_ahead_unpushed");
    std::fs::write(clone.join("work.txt"), "one").unwrap();
    assert!(env.run(&["--for", "libs/mylib", "add", "-A"]).success);
    assert!(
        env.run(&["--for", "libs/mylib", "commit", "-m", "one"])
            .success
    );

    let out = env.run(&["ls", "--color", "always"]);
    assert_eq!(
        status_cell(&out.stdout, "libs/mylib"),
        "+1",
        "{}",
        out.stdout
    );
    assert_eq!(icon_and_colour(&out.stdout, "libs/mylib").0, "⇑");
    let row = json_row(&env, "libs/mylib");
    assert_eq!(
        (row["ahead"].clone(), row["behind"].clone()),
        (1.into(), 0.into())
    );

    let push = env.run(&["--for", "libs/mylib", "push"]);
    assert!(push.success, "{}{}", push.stdout, push.stderr);
    let row = json_row(&env, "libs/mylib");
    assert_eq!(
        (row["ahead"].clone(), row["behind"].clone()),
        (0.into(), 0.into())
    );
}

/// On a topic branch with an upstream, commits on either side are counted:
/// `+N` with `⇑`, `-N` with `⇓`, both with `⇅` — in the table and as `ahead`
/// and `behind` in the JSON.
#[test]
fn normal_011_counts_commits_ahead_of_and_behind_the_upstream() {
    let (env, bare, clone) = setup_commit_env("status_ahead_behind");
    std::fs::write(clone.join("work.txt"), "one").unwrap();
    assert!(env.run(&["--for", "libs/mylib", "add", "-A"]).success);
    assert!(
        env.run(&["--for", "libs/mylib", "commit", "-m", "one"])
            .success
    );
    let push = env.run(&["--for", "libs/mylib", "push"]);
    assert!(push.success, "{}{}", push.stdout, push.stderr);

    // One commit of our own.
    std::fs::write(clone.join("work.txt"), "two").unwrap();
    support::run_git_pub(&clone, &["commit", "-q", "-am", "two"]);
    let out = env.run(&["ls", "--color", "always"]);
    assert_eq!(
        status_cell(&out.stdout, "libs/mylib"),
        "+1",
        "{}",
        out.stdout
    );
    assert_eq!(icon_and_colour(&out.stdout, "libs/mylib").0, "⇑");
    let row = json_row(&env, "libs/mylib");
    assert_eq!(
        (row["ahead"].clone(), row["behind"].clone()),
        (1.into(), 0.into())
    );

    // And one on the remote's side, once fetched.
    env.push_commit(&bare, "feat/x", "remote.txt", "theirs");
    let out = env.run(&["ls", "--color", "always", "--fetch"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(
        status_cell(&out.stdout, "libs/mylib"),
        "+1, -1",
        "{}",
        out.stdout
    );
    assert_eq!(icon_and_colour(&out.stdout, "libs/mylib").0, "⇅");
    let row = json_row(&env, "libs/mylib");
    assert_eq!(
        (row["ahead"].clone(), row["behind"].clone()),
        (1.into(), 1.into())
    );

    // Only behind.
    support::run_git_pub(&clone, &["reset", "-q", "--hard", "HEAD~1"]);
    let out = env.run(&["ls", "--color", "always"]);
    assert_eq!(
        status_cell(&out.stdout, "libs/mylib"),
        "-1",
        "{}",
        out.stdout
    );
    assert_eq!(
        icon_and_colour(&out.stdout, "libs/mylib"),
        ("⇓".to_string(), "33".to_string())
    );
}

/// An entry whose directory is a symlink to a checkout somewhere else is
/// reported as `symlink` alone, with `⤷`, where it points in PATH and the ref
/// of what it points at — and the same in the JSON.
#[test]
fn normal_012_a_symlinked_entry_shows_where_it_points() {
    let env = TestEnv::new("status_symlink_entry");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.write_config(&mylib_config(&bare, "main"));
    assert!(env.run(&["sync"]).success);
    let checkout = env.playground.join("libs/mylib");
    std::fs::remove_dir_all(&checkout).unwrap();
    let own = env.repos_remote.join("own-mylib");
    support::run_git_pub(
        &env.repos_remote,
        &["clone", "-q", bare.to_str().unwrap(), own.to_str().unwrap()],
    );
    std::os::unix::fs::symlink(&own, &checkout).unwrap();

    let out = env.run(&["ls", "--color", "always"]);
    assert!(out.success, "{}", out.stderr);
    let row = cells(&table_row(&out.stdout, "libs/mylib"));
    assert_eq!(row[2], own.display().to_string(), "{:?}", row);
    assert_eq!(row[4], "main", "{:?}", row);
    assert_eq!(row[6], "symlink", "{:?}", row);
    assert_eq!(
        icon_and_colour(&out.stdout, "libs/mylib"),
        ("⤷".to_string(), "36".to_string())
    );
    let json = json_row(&env, "libs/mylib");
    assert_eq!(json["symlink"], true, "{}", json);
    assert_eq!(
        json["symlink_target"],
        own.display().to_string(),
        "{}",
        json
    );
    assert_eq!(json["exists"], true, "{}", json);
}

/// Planted links git sees as untracked — once GitScale's block in the
/// checkout's `info/exclude` is gone — are listed in the JSON, relative to the
/// checkout, and do not make it unclean there either.
#[test]
fn normal_013_json_lists_the_untracked_links() {
    let env = TestEnv::new("status_json_untracked_links");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    assert!(env.run(&["sync"]).success);
    let row = json_row(&env, "imports/b");
    assert_eq!(row["untracked_links"], serde_json::json!([]), "{}", row);

    let checkout = env.playground.join("imports/b");
    let exclude = support::git_stdout(&checkout, &["rev-parse", "--git-path", "info/exclude"]);
    std::fs::write(checkout.join(exclude), "").unwrap();
    let row = json_row(&env, "imports/b");
    assert_eq!(
        row["untracked_links"],
        serde_json::json!(["libs/d"]),
        "{}",
        row
    );
    assert_eq!(row["clean"], true, "{}", row);
}

/// An artefact's JSON carries what the last fetch saw beside what is
/// installed, and the flags they make: `ref-mismatch` and `missing` while
/// the release wanted has no image, `ref-mismatch` once it has. The table's
/// icon follows: `!`, then `≠`, both in bright red.
#[test]
fn normal_014_json_carries_an_artefacts_remote_state_and_flags() {
    let env = TestEnv::new("status_artefact_json_remote");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);
    let row = json_row(&env, "meta/app");
    assert_eq!(row["artefact"]["remote"]["tag"], "v1.0.0", "{}", row);
    assert_eq!(row["artefact"]["flags"], serde_json::json!([]), "{}", row);

    env.push_commit(&bare, "main", "README.md", "v2");
    support::run_git_pub(&bare, &["tag", "v1.1.0", "main"]);
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    let _ = env.run(&["fetch"]);
    let row = json_row(&env, "meta/app");
    let artefact = &row["artefact"];
    assert_eq!(artefact["installed"]["tag"], "v1.0.0", "{}", row);
    assert_eq!(artefact["remote"]["tag"], "v1.1.0", "{}", row);
    assert_eq!(
        artefact["remote"]["digest"],
        serde_json::Value::Null,
        "{}",
        row
    );
    assert_eq!(
        artefact["flags"],
        serde_json::json!(["ref-mismatch", "missing"]),
        "{}",
        row
    );
    let out = env.run(&["ls", "--color", "always"]);
    assert_eq!(
        icon_and_colour(&out.stdout, "meta/app"),
        ("!".to_string(), "91".to_string())
    );

    env.publish(&bare, "main", &[("app.bin", "v2")]);
    assert!(env.run(&["fetch"]).success);
    let row = json_row(&env, "meta/app");
    assert!(row["artefact"]["remote"]["digest"]
        .as_str()
        .is_some_and(|d| d.starts_with("sha256:")));
    assert_eq!(
        row["artefact"]["flags"],
        serde_json::json!(["ref-mismatch"]),
        "{}",
        row
    );
    let out = env.run(&["ls", "--color", "always"]);
    assert_eq!(
        icon_and_colour(&out.stdout, "meta/app"),
        ("≠".to_string(), "91".to_string())
    );
}

/// `ls --fetch` asks the registry itself: the same run reports what it
/// just found, with no separate `fetch` before it.
#[test]
fn normal_015_fetch_reports_what_the_registry_has_now() {
    let env = TestEnv::new("status_fetch_artefact");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);
    let artefact = "[artefact]\ninclude = [\"dist/**\"]\n";
    let (out, _) = env.publish_with(
        &bare,
        "main",
        artefact,
        &[("app.bin", "rebuilt")],
        &["--force", "v1.0.0"],
    );
    assert!(out.success, "{}", out.stderr);
    let offline = env.run(&["ls"]);
    assert_eq!(
        status_cell(&offline.stdout, "meta/app"),
        "ok",
        "{}",
        offline.stdout
    );

    let out = env.run(&["ls", "--fetch"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(
        status_cell(&out.stdout, "meta/app"),
        "changed",
        "{}{}",
        out.stdout,
        out.stderr
    );
}

/// On a topic, the table starts with the topic's branch and what may merge
/// next, and the JSON carries both as an object of their own, with the
/// row's `topic` saying which branch it is on and that it is joined here.
#[test]
fn normal_016_a_topic_is_named_with_what_may_merge_next() {
    let (env, _bare, clone) = setup_commit_env("status_topic_next");
    std::fs::write(clone.join("work.txt"), "change").unwrap();
    assert!(env.run(&["commit", "-m", "change"]).success);

    let out = env.run(&["ls"]);
    assert!(out.success, "{}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    let first = plain.lines().next().unwrap_or_default();
    assert_eq!(
        first, "topic feat/x · next to merge: libs/mylib",
        "{}",
        plain
    );

    let json = env.run(&["ls", "--format", "json"]);
    let rows = json_rows(&json.stdout);
    let topic = rows
        .iter()
        .find(|r| r.get("topic").is_some_and(|t| t.is_string()))
        .unwrap_or_else(|| panic!("no topic object:\n{}", json.stdout));
    assert_eq!(topic["topic"], "feat/x");
    assert_eq!(topic["next_to_merge"], serde_json::json!(["libs/mylib"]));
    let row = rows
        .iter()
        .find(|r| r["directory"] == "libs/mylib")
        .unwrap();
    assert_eq!(row["topic"]["branch"], "feat/x", "{}", row);
    assert_eq!(row["topic"]["developed"], true, "{}", row);
}

/// A topic branch only the remote has yet is `topic, from remote` until a
/// placement puts it on a local branch.
#[test]
fn normal_017_a_topic_branch_only_the_remote_has_is_from_remote() {
    let f = fixture("status_topic_from_remote", "");
    f.core_commit("feat/x", "lib.txt", "remote work");
    let ws = f.clone_root("ws");
    support::run_git_pub(&ws, &["switch", "-q", "-c", "feat/x"]);

    let out = gs(&ws, &["ls", "--fetch"]);
    assert!(out.success, "{}", out.stderr);
    let row = table_row(&out.stdout, "imports/core");
    assert!(row.contains("topic, from remote"), "{}", row);
}

/// `git explain` marks the requests an override beat and the override itself.
#[test]
fn normal_018_explain_marks_the_override_and_what_it_overruled() {
    let env = TestEnv::new("status_why_override");
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\", override = true"),
    ]));
    assert!(env.run(&["sync"]).success);

    let out = env.run(&["explain", "imports/d"]);
    assert!(out.success, "{}", out.stderr);
    let root = out
        .stdout
        .lines()
        .find(|l| l.trim_start().starts_with("root "))
        .unwrap_or_else(|| panic!("no root request:\n{}", out.stdout));
    assert!(
        root.contains("override") && root.contains("selected"),
        "{}",
        root
    );
    let beaten = out
        .stdout
        .lines()
        .find(|l| l.contains("imports/b@v1.0.0"))
        .unwrap_or_else(|| panic!("no request from b:\n{}", out.stdout));
    assert!(beaten.contains("v1.5.0"), "{}", beaten);
    assert!(beaten.contains("overruled by root"), "{}", beaten);

    // The JSON carries the same structure.
    let row = json_row(&env, "imports/d");
    assert_eq!(row["resolution"], "override", "{}", row);
    let requests = row["requests"].as_array().unwrap();
    assert!(
        requests
            .iter()
            .any(|r| r["from"] == "root" && r["override"] == true && r["selected"] == true),
        "{}",
        row
    );
    assert!(
        requests
            .iter()
            .any(|r| r["revision"] == "v1.5.0" && r["overruled_by"] == "root"),
        "{}",
        row
    );
}

/// A workspace that declares nothing says so, rather than printing an empty
/// table.
#[test]
fn normal_019_a_workspace_with_no_repos_says_so() {
    let env = TestEnv::new("status_no_repos");
    env.write_config("[repos]\n");
    let out = env.run(&["ls"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "No repos declared in .gitscale.toml");
}

/// `git explain` alone, when no checkout is asked for by more than one
/// repository, says so rather than printing nothing.
#[test]
fn normal_020_explain_with_nothing_shared_says_so() {
    let env = TestEnv::new("status_why_nothing_shared");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.write_config(&mylib_config(&bare, "main"));
    assert!(env.run(&["sync"]).success);
    let out = env.run(&["explain"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(
        out.stdout.trim(),
        "Every dependency is asked for by one repository only."
    );
}

/// A topic slot that asks for another topic slot not yet promoted waits on
/// it, and the one waited on is what may merge next.
#[test]
fn normal_028_a_topic_slot_waits_on_the_topic_slots_it_asks_for() {
    let env = TestEnv::new("status_topic_waits");
    let d = repo_with_topic(&env, "d", &[("d.txt", "d")]);
    let b_config = format!(
        "[repos]\n\"libs/d\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let b = repo_with_topic(&env, "b", &[(".gitscale.toml", &b_config)]);
    env.write_config(&format!(
        "{}[repos]\n\"imports/b\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        crate::support::resolution::allow(&env),
        b.display()
    ));
    env.init_playground_git();
    support::run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let pull = env.run(&["sync"]);
    assert!(pull.success, "{}{}", pull.stdout, pull.stderr);

    let out = env.run(&["ls"]);
    assert!(out.success, "{}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    assert_eq!(
        plain.lines().next().unwrap_or_default(),
        "topic feat/x · next to merge: imports/d",
        "{}",
        plain
    );
    let row = table_row(&out.stdout, "imports/b");
    assert!(row.contains("waits on imports/d"), "{}", row);
    let row = table_row(&out.stdout, "imports/d");
    assert!(!row.contains("waits on"), "{}", row);
}

/// A topic branch cut before the release the graph now pins is behind it,
/// and the row says what to do: rebase.
#[test]
fn normal_029_a_topic_branch_behind_the_pin_says_rebase_it() {
    let env = TestEnv::new("status_topic_behind_pin");
    let core = repo_with_topic(&env, "core", &[("lib.txt", "v1")]);
    let release = env.push_commit(&core, "main", "lib.txt", "v1.1");
    support::run_git_pub(&core, &["tag", "v1.1.0", &release]);
    env.write_config(&format!(
        "[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.1.0\" }}\n",
        core.display()
    ));
    env.init_playground_git();
    support::run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let pull = env.run(&["sync"]);
    assert!(pull.success, "{}{}", pull.stdout, pull.stderr);

    let out = env.run(&["ls"]);
    assert!(out.success, "{}", out.stderr);
    let row = table_row(&out.stdout, "imports/core");
    assert!(
        row.contains("behind v1.1.0 wanted by root: rebase it"),
        "{}",
        row
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A checkout at its pin is detached, so REF reads as a commit while
/// EXPECTED reads as the tag. That is not a mismatch, and `ls` must not dress
/// it as one — the yellow REF and a `ref-mismatch` flag both have to key off
/// where HEAD actually is.
#[test]
fn edge_007_a_detached_tag_pin_is_not_a_mismatch() {
    let env = TestEnv::new("status_tag_pinned");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    support::run_git_pub(&bare, &["tag", "demo-v1", "main"]);

    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "demo-v1" }}
"#,
        bare.display()
    ));
    assert!(env.run(&["sync"]).success);

    let out = env.run(&["ls"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let plain = strip_ansi(&out.stdout);
    assert!(
        plain
            .lines()
            .any(|l| l.contains("demo-v1") && l.trim_end().ends_with("ok")),
        "{}",
        plain
    );
    assert!(
        !plain.contains("ref-mismatch"),
        "a checkout at the pinned tag is on the right commit: {}",
        plain
    );
    assert!(
        !out.stdout.contains("\x1b[33m"),
        "nothing here is amber: {:?}",
        out.stdout
    );
}

/// A consumer of `--format json` must get JSON whatever the workspace holds.
#[test]
fn edge_008_json_with_no_repos_is_json() {
    let env = TestEnv::new("status_json_empty");
    env.write_config("[repos]\n");

    let out = env.run(&["ls", "--format", "json"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        serde_json::from_str::<serde_json::Value>(&out.stdout).is_ok(),
        "not JSON: {}",
        out.stdout
    );
}

/// An orphaned link whose target still resolves is `orphan` in yellow; one
/// whose target is gone is `orphan, broken` in bright red. Both carry `⊘`,
/// and both are rows of their own in the JSON.
#[test]
fn edge_021a_an_orphan_link_whose_target_resolves() {
    let (env, _orphan) = setup_orphan_env("status_orphan_valid", true);
    let out = env.run(&["ls", "--color", "always"]);
    assert!(out.success, "{}", out.stderr);
    let row = table_row(&out.stdout, "repoA/libs/c");
    assert!(row.trim_end().ends_with("orphan"), "{}", row);
    assert_eq!(
        icon_and_colour(&out.stdout, "repoA/libs/c"),
        ("⊘".to_string(), "33".to_string())
    );
    let json = env.run(&["ls", "--color", "always", "--format", "json"]);
    let rows = json_rows(&json.stdout);
    assert!(
        rows.contains(
            &serde_json::json!({"directory": "repoA/libs/c", "orphan": true, "broken": false})
        ),
        "{}",
        json.stdout
    );
}

/// See `edge_021a`: the same with the target gone.
#[test]
fn edge_021b_an_orphan_link_whose_target_is_gone() {
    let (env, _orphan) = setup_orphan_env("status_orphan_broken", false);
    let out = env.run(&["ls", "--color", "always"]);
    assert!(out.success, "{}", out.stderr);
    let row = table_row(&out.stdout, "repoA/libs/c");
    assert!(row.trim_end().ends_with("orphan, broken"), "{}", row);
    assert_eq!(
        icon_and_colour(&out.stdout, "repoA/libs/c"),
        ("⊘".to_string(), "91".to_string())
    );
    let json = env.run(&["ls", "--color", "always", "--format", "json"]);
    let rows = json_rows(&json.stdout);
    assert!(
        rows.contains(
            &serde_json::json!({"directory": "repoA/libs/c", "orphan": true, "broken": true})
        ),
        "{}",
        json.stdout
    );
}

/// A directory named with the trailing slash a shell completes is the same
/// checkout to `git explain`.
#[test]
fn edge_022_explain_takes_a_directory_with_a_trailing_slash() {
    let env = TestEnv::new("status_why_slash");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.write_config(&mylib_config(&bare, "main"));
    assert!(env.run(&["sync"]).success);
    let out = env.run(&["explain", "libs/mylib/"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.starts_with("libs/mylib  "), "{}", out.stdout);
}

/// A clone standing where a link belongs is `unlinked`, and that is the whole
/// story: the path is kept out of the owner's git status, so the owner is not
/// called `dirty` for it.
#[test]
fn edge_023_an_unlinked_clone_in_a_repository_that_does_not_ignore_it() {
    let (env, _link) = setup_unlinked_env("status_unlinked_alone");
    let out = env.run(&["ls"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(
        status_cell(&out.stdout, "repoA"),
        "unlinked",
        "{}",
        out.stdout
    );
}

/// A remote that cannot be reached when `--fetch` asks for it: `ls` still
/// reports, from what was fetched before, and says so on stderr.
#[test]
fn edge_024_fetch_falls_back_to_what_was_fetched_before() {
    let env = TestEnv::new("status_fetch_git_failure");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.write_config(&mylib_config(&bare, "main"));
    assert!(env.run(&["sync"]).success);
    std::fs::rename(&bare, bare.with_extension("moved")).unwrap();

    // A subprocess: the warning goes straight to the process's stderr.
    let out = gitscale(&env, &[], &["ls", "--fetch"]);
    assert_eq!(out.code, Some(0), "{}", out.said());
    assert!(
        out.stderr.contains("using what was fetched before"),
        "{}",
        out.stderr
    );
    assert_eq!(
        status_cell(&out.stdout, "libs/mylib"),
        "ok",
        "{}",
        out.stdout
    );
}

/// `stale` stands in for a behind count where a depth-1 checkout has no
/// history to count: a shallow checkout on a branch whose upstream has moved
/// is `stale`, with `≠` in bright red, and `"stale": true` in the JSON.
/// gitscale's own CI checkouts are detached and so have no upstream; this
/// puts one on a branch by hand to reach the flag.
#[test]
fn edge_031_a_shallow_checkout_behind_its_upstream_is_stale() {
    let env = TestEnv::new("status_stale");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.write_config(&mylib_config(&bare, "main"));
    env.init_playground_git();
    let ci = [("CI", "true")];
    let pull = env.run_with_env(&ci, &["sync", "--no-cache"]);
    assert!(pull.success, "{}{}", pull.stdout, pull.stderr);
    let checkout = env.playground.join("libs/mylib");
    let track = [
        "fetch",
        "-q",
        "--depth",
        "1",
        "origin",
        "+refs/heads/main:refs/remotes/origin/main",
    ];
    support::run_git_pub(&checkout, &track);
    support::run_git_pub(
        &checkout,
        &["switch", "-q", "-c", "main", "--track", "origin/main"],
    );
    let before = env.run_with_env(&ci, &["ls", "--color", "always"]);
    assert_eq!(
        status_cell(&before.stdout, "libs/mylib"),
        "ok",
        "{}",
        before.stdout
    );

    env.push_commit(&bare, "main", "a.txt", "moved");
    support::run_git_pub(&checkout, &track);
    let out = env.run_with_env(&ci, &["ls", "--color", "always"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(
        status_cell(&out.stdout, "libs/mylib"),
        "stale",
        "{}",
        out.stdout
    );
    assert_eq!(
        icon_and_colour(&out.stdout, "libs/mylib"),
        ("≠".to_string(), "91".to_string())
    );
    let json = env.run_with_env(&ci, &["ls", "--color", "always", "--format", "json"]);
    let rows = json_rows(&json.stdout);
    let row = rows
        .iter()
        .find(|r| r["directory"] == "libs/mylib")
        .unwrap();
    assert_eq!(row["stale"], true, "{}", row);
}

/// A checkout git cannot read — its `.git` names a git directory that is
/// gone — is not `ok`: git could not say whether it is clean, or where its
/// HEAD is, and `ls` must not claim either.
#[test]
#[ignore = "bug: ls reads a failed git status as clean, so a checkout git cannot read shows ok"]
fn edge_032_a_checkout_git_cannot_read_is_not_ok() {
    let env = TestEnv::new("status_unreadable_checkout");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.write_config(&mylib_config(&bare, "main"));
    env.init_playground_git();
    let ci = [("CI", "true")];
    let pull = env.run_with_env(&ci, &["sync", "--no-cache"]);
    assert!(pull.success, "{}{}", pull.stdout, pull.stderr);
    let checkout = env.playground.join("libs/mylib");
    std::fs::remove_dir_all(checkout.join(".git")).unwrap();
    std::fs::write(checkout.join(".git"), "gitdir: /nonexistent/gitdir\n").unwrap();

    let out = env.run_with_env(&ci, &["ls"]);
    // REF is empty here, so the row is read whole rather than by cells.
    let row = table_row(&out.stdout, "libs/mylib");
    assert!(!row.trim_end().ends_with(" ok"), "{}", row);
    let json = env.run_with_env(&ci, &["ls", "--format", "json"]);
    let rows = json_rows(&json.stdout);
    let row = rows
        .iter()
        .find(|r| r["directory"] == "libs/mylib")
        .unwrap();
    assert_ne!(row["clean"], true, "{}", row);
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// `--fetch` asked for fresh state; one it could not get is said so, not
/// silently replaced by whatever the last fetch saw.
#[test]
fn error_009_fetch_reports_a_fetch_it_could_not_do() {
    let env = TestEnv::new("status_fetch_failure");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    env.prefer(&bare, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        r#"[repos]
"meta/app" = {{ url = "{}", revision = "v1.0.0" }}
"#,
        bare.display()
    ));
    let out = env.run(&["ls", "--fetch"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("fetch meta/app: no registry is known"),
        "stderr: {}",
        out.stderr
    );
    // It still reports, on what it has.
    assert!(
        out.stderr.contains("(showing the last fetched state)"),
        "stderr: {}",
        out.stderr
    );
    assert_eq!(
        status_cell(&out.stdout, "meta/app"),
        "missed",
        "{}",
        out.stdout
    );
}

/// A graph that cannot be resolved as a whole still gets its table: the error
/// on stderr as `error: …`, and every declared entry marked `unresolved`,
/// pointing at it. JSON stays JSON. This pins the current exit status, 0;
/// whether a failed resolution should fail `ls` is a decision for the
/// owner.
#[test]
fn error_025_a_graph_that_cannot_be_resolved_still_gets_a_table() {
    let env = TestEnv::new("status_resolution_fails");
    conflicted_workspace(&env);

    let out = env.run(&["ls"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stderr.contains("error: "), "{}", out.stderr);
    assert!(out.stderr.contains("override conflict"), "{}", out.stderr);
    for dir in ["imports/b", "imports/c"] {
        let row = table_row(&out.stdout, dir);
        assert!(row.contains("unresolved"), "{}", row);
        assert!(row.trim_end().ends_with("see the error above"), "{}", row);
    }

    let json = env.run(&["ls", "--format", "json"]);
    assert!(json.success, "{}", json.stderr);
    let rows = json_rows(&json.stdout);
    let b = rows.iter().find(|r| r["directory"] == "imports/b").unwrap();
    assert_eq!(b["resolution"], "unresolved", "{}", b);
    assert!(b["unresolved"]
        .as_str()
        .is_some_and(|r| r.contains("override conflict")));
}

/// `-v` names each artefact as `--fetch` asks the registry about it — on
/// stderr, or anywhere but in the JSON a consumer parses.
#[test]
#[ignore = "bug: ls -v --fetch --format json prints 'Fetching …' into the JSON on stdout"]
fn error_026_verbose_fetch_keeps_json_parseable() {
    let env = TestEnv::new("status_verbose_json");
    let bare = env.artefact_repo("app", &[("app.bin", "v1")]);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);

    let out = env.run(&["ls", "-v", "--fetch", "--format", "json"]);
    assert!(out.success, "{}", out.stderr);
    let parsed = serde_json::from_str::<serde_json::Value>(&out.stdout);
    assert!(parsed.is_ok(), "not JSON:\n{}", out.stdout);
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

/// Without `--fetch`, `ls` reads only this machine: not one registry
/// request, and no attempt at a remote that has gone away.
#[test]
fn perf_027_without_fetch_ls_asks_nothing_of_the_network() {
    let env = TestEnv::new("status_offline");
    let app = env.artefact_repo("app", &[("app.bin", "v1")]);
    let lib = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.prefer(&app, gitscale::prefer::Form::Artefact);
    env.write_config(&format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n\
         \"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        env.registries(),
        app.display(),
        lib.display()
    ));
    assert!(env.run(&["sync"]).success);
    std::fs::rename(&lib, lib.with_extension("moved")).unwrap();
    env.registry().clear_log();

    for format in ["table", "json"] {
        let out = env.run(&["ls", "--format", format]);
        assert!(out.success, "{}", out.stderr);
        assert!(out.stderr.is_empty(), "{}: {}", format, out.stderr);
    }
    assert!(
        env.registry().log().is_empty(),
        "{:?}",
        env.registry().log()
    );
}
