//! `git scale <git command>`: a git command run across the workspace — which
//! repositories, in which order, what git is given, how output is shown, and
//! what is placed afterwards.

use crate::support::resolution::{repos, tag_commit, tagged};
use crate::support::worktrees::{branch, git, gs, head, identity, ok};
use crate::support::{run_git_pub, TestEnv};
use std::path::{Path, PathBuf};
use std::process::Command;

/// d at v1.0.0; b at v1.0.0 asking for d at v1.0.0 under `libs/d`; and a
/// clone of a root declaring both at v1.0.0, `imports/` ignored, placed.
struct Ws {
    env: TestEnv,
    ws: PathBuf,
    b: PathBuf,
    d: PathBuf,
}

fn workspace(name: &str) -> Ws {
    workspace_with(name, "")
}

fn workspace_with(name: &str, extra: &str) -> Ws {
    let env = TestEnv::new(name);
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let b = tagged(
        &env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.0.0\"")]),
        )],
    );
    let ws = root_clone(
        &env,
        &format!(
            "{}{}",
            extra,
            repos(&[
                ("imports/b", &b, ", revision = \"v1.0.0\""),
                ("imports/d", &d, ", revision = \"v1.0.0\""),
            ])
        ),
    );
    ok(&gs(&ws, &["sync"]));
    Ws { env, ws, b, d }
}

/// A clone of a new root repository whose config is `config`, `imports/`
/// ignored, with an identity.
fn root_clone(env: &TestEnv, config: &str) -> PathBuf {
    let root = env.create_bare_repo(
        "root",
        "main",
        &[
            ("README.md", "root"),
            (".gitignore", "imports/\n"),
            (".gitscale.toml", config),
        ],
    );
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    identity(&ws);
    ws
}

impl Ws {
    /// On topic `feat`, with b and d joined.
    fn on_topic(self) -> Self {
        ok(&gs(&self.ws, &["topic", "start", "feat"]));
        ok(&gs(&self.ws, &["topic", "join", "imports/b", "imports/d"]));
        for dir in ["imports/b", "imports/d"] {
            identity(&self.ws.join(dir));
        }
        self
    }

    fn child(&self, dir: &str) -> PathBuf {
        self.ws.join(dir)
    }
}

/// The headers of the repositories in `stdout`, as `N/TOTAL NAME`, colour
/// taken off. The placement's header is not among them.
fn headers(stdout: &str) -> Vec<String> {
    crate::support::strip_ansi(stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("── "))
        .map(|l| l.trim_end_matches('─').trim_end().to_string())
        .filter(|h| h != "placing" && h != "refreshing")
        .collect()
}

/// The binary run with `args` from `cwd`, `vars` set, under a pseudo-terminal
/// (`script`): the transcript of everything it printed. `None` when `script`
/// is not available.
fn in_terminal(cwd: &Path, args: &[&str], vars: &[(&str, &str)]) -> Option<(String, bool)> {
    let quoted: Vec<String> = std::iter::once(env!("CARGO_BIN_EXE_gitscale"))
        .chain(args.iter().copied())
        .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
        .collect();
    let mut cmd = Command::new("script");
    cmd.args(["-qec", &quoted.join(" "), "/dev/null"])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null());
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let output = cmd.output().ok()?;
    Some((
        String::from_utf8_lossy(&output.stdout).replace('\r', ""),
        output.status.success(),
    ))
}

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// By default a git command runs in the root and every checkout on the
/// topic: dependencies before the repositories that ask for them, the root
/// last.
#[test]
fn normal_001_runs_on_the_topic_dependencies_first_and_the_root_last() {
    let w = workspace("fwd_default").on_topic();
    let out = gs(&w.ws, &["rev-parse", "--abbrev-ref", "HEAD"]);
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec!["1/3 imports/d", "2/3 imports/b", "3/3 ."],
        "{}",
        out.stdout
    );
    assert_eq!(out.stdout.matches("\nfeat\n").count(), 3, "{}", out.stdout);
}

/// Off a topic, the root alone: every checkout sits at its pin.
#[test]
fn normal_002_off_a_topic_runs_in_the_root_alone() {
    let w = workspace("fwd_off_topic");
    let out = gs(&w.ws, &["status", "--short"]);
    ok(&out);
    assert_eq!(headers(&out.stdout), vec!["1/1 ."], "{}", out.stdout);
}

/// `--for` names exactly the repositories to run in — `.` is the root — and
/// `--foreach` adds the checkouts at their pins.
#[test]
fn normal_003_for_names_the_repositories_and_foreach_adds_the_pinned_ones() {
    let w = workspace("fwd_select");
    let out = gs(&w.ws, &["--for", ".", "rev-parse", "HEAD"]);
    ok(&out);
    assert_eq!(headers(&out.stdout), vec!["1/1 ."], "{}", out.stdout);

    let out = gs(&w.ws, &["--foreach", "rev-parse", "HEAD"]);
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec!["1/3 imports/d", "2/3 imports/b", "3/3 ."],
        "{}",
        out.stdout
    );

    let out = gs(
        &w.ws,
        &["--foreach", "--for", "imports/d", "rev-parse", "HEAD"],
    );
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec!["1/1 imports/d"],
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&tag_commit(&w.d, "v1.0.0")),
        "{}",
        out.stdout
    );
}

/// `--for` takes a path from the current directory, and a link path names the
/// checkout it points at.
#[test]
fn normal_004_for_takes_paths_from_the_current_directory() {
    let w = workspace("fwd_select_relative").on_topic();
    let imports = w.ws.join("imports");
    let mut args = vec!["gitscale", "-C", imports.to_str().unwrap()];
    args.extend(["--for", "d", "rev-parse", "--show-toplevel"]);
    let out = gitscale::run_cli_with(&args, false);
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec!["1/1 imports/d"],
        "{}",
        out.stdout
    );

    let b = w.child("imports/b");
    let mut args = vec!["gitscale", "-C", b.to_str().unwrap()];
    args.extend(["--for", "libs/d", "--for", ".", "rev-parse", "HEAD"]);
    let out = gitscale::run_cli_with(&args, false);
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec!["1/2 imports/d", "2/2 imports/b"],
        "{}",
        out.stdout
    );
}

/// `commit` skips a repository with nothing to commit, saying so in its
/// header, which still counts; `--amend` is never skipped.
#[test]
fn normal_005_commit_skips_a_repository_with_nothing_to_commit_but_not_amend() {
    let w = workspace("fwd_commit_skip").on_topic();
    std::fs::write(w.child("imports/d/work.txt"), "work").unwrap();
    ok(&gs(&w.ws, &["--for", "imports/d", "add", "-A"]));
    let before = head(&w.child("imports/b"));

    let out = gs(&w.ws, &["commit", "-m", "work"]);
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec![
            "1/3 imports/d",
            "2/3 imports/b ── skip (nothing to commit)",
            "3/3 . ── skip (nothing to commit)"
        ],
        "{}",
        out.stdout
    );
    assert_eq!(
        git(&w.child("imports/d"), &["log", "-1", "--format=%s"]),
        "work"
    );
    assert_eq!(head(&w.child("imports/b")), before);

    let out = gs(
        &w.ws,
        &["--for", "imports/b", "commit", "--amend", "-m", "amended"],
    );
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec!["1/1 imports/b"],
        "{}",
        out.stdout
    );
    assert_eq!(
        git(&w.child("imports/b"), &["log", "-1", "--format=%s"]),
        "amended"
    );
}

/// A first `push` of a topic branch sets its upstream: git is given
/// `push.autoSetupRemote`.
#[test]
fn normal_006_a_first_push_sets_the_upstream() {
    let w = workspace("fwd_push_upstream").on_topic();
    let out = gs(&w.ws, &["push", "--quiet"]);
    ok(&out);
    for dir in ["imports/b", "imports/d"] {
        assert_eq!(
            git(&w.child(dir), &["rev-parse", "--abbrev-ref", "@{upstream}"]),
            "origin/feat",
            "{}",
            dir
        );
    }
    assert_eq!(
        git(&w.ws, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/feat"
    );
    assert!(git(&w.b, &["branch", "--list", "feat"]).contains("feat"));
}

/// A checkout with changes that the command did not run in — not on the
/// topic — gets one line on stderr saying how to bring it in. Links planted
/// in a checkout are not changes.
#[test]
fn normal_007_changes_outside_the_selection_are_named() {
    let w = workspace("fwd_outside");
    ok(&gs(&w.ws, &["topic", "start", "feat"]));
    ok(&gs(&w.ws, &["topic", "join", "imports/d"]));
    let file = w.child("imports/b/README.md");
    crate::support::edit(&file, "changed");

    let out = gs(&w.ws, &["status", "--short"]);
    ok(&out);
    assert!(
        out.stderr
            .contains("imports/b has changes but is not on the topic: git topic join imports/b"),
        "{}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("imports/d has changes"),
        "{}",
        out.stderr
    );
}

/// `pull` brings every checkout up to date, detached ones included: a
/// checkout pinned to a branch that moved, and one whose remote has the
/// topic a colleague started there.
#[test]
fn normal_008_pull_places_detached_checkouts() {
    let env = TestEnv::new("fwd_pull_places");
    let lib = env.create_bare_repo("lib", "main", &[("a.txt", "v1")]);
    let d = tagged(&env, "d", &[("v1.0.0", "")]);
    let ws = root_clone(
        &env,
        &repos(&[
            ("imports/lib", &lib, ", revision = \"main\""),
            ("imports/d", &d, ", revision = \"v1.0.0\""),
        ]),
    );
    ok(&gs(&ws, &["sync"]));
    ok(&gs(&ws, &["topic", "start", "feat"]));
    let root_remote = env.repos_remote.join("root.git");
    run_git_pub(&ws, &["push", "-q", "-u", "origin", "feat"]);
    let moved = env.push_commit(&lib, "main", "a.txt", "v2");
    run_git_pub(&d, &["branch", "feat", "v1.0.0"]);
    let started = env.push_commit(&d, "feat", "d.txt", "colleague");
    assert!(root_remote.is_dir());

    let out = gs(&ws, &["pull", "--quiet"]);
    ok(&out);
    assert_eq!(head(&ws.join("imports/lib")), moved, "{}", out.stdout);
    assert_eq!(head(&ws.join("imports/d")), started, "{}", out.stdout);
    assert_eq!(branch(&ws.join("imports/d")).as_deref(), Some("feat"));
    assert!(out.stdout.contains("── placing "), "{}", out.stdout);
}

/// `fetch` refreshes every store, including one no checkout uses any more,
/// and moves nothing.
#[test]
fn normal_009_fetch_refreshes_every_store_and_moves_nothing() {
    let w = workspace("fwd_fetch");
    let pinned = head(&w.child("imports/d"));
    let tip = w.env.push_commit(&w.d, "main", "d.txt", "new");

    let out = gs(&w.ws, &["fetch", "--quiet"]);
    ok(&out);
    assert!(out.stdout.contains("── refreshing "), "{}", out.stdout);
    assert!(!out.stdout.contains("── placing "), "{}", out.stdout);
    let store = crate::support::worktrees::common_dir(&w.ws)
        .join("gitscale/repos")
        .join(gitscale::store::entry_name(&w.d.display().to_string()));
    assert_eq!(
        git(
            &store,
            &[
                "--git-dir",
                store.to_str().unwrap(),
                "rev-parse",
                "refs/remotes/origin/main"
            ]
        ),
        tip
    );
    assert_eq!(head(&w.child("imports/d")), pinned);
}

/// Any other command places the workspace only when it moved a `HEAD`, and
/// then fetches only what resolution lacks: here, the tag a branch of the
/// root's newly asks for.
#[test]
fn normal_010_another_command_places_only_when_a_head_moved_fetching_on_miss() {
    let w = workspace("fwd_place_on_move");
    let out = gs(&w.ws, &["log", "--oneline", "-1"]);
    ok(&out);
    assert!(!out.stdout.contains("── placing "), "{}", out.stdout);

    // A tag the stores have not seen, and a branch of the root asking for it.
    let v11 = w.env.push_commit(&w.d, "main", "d.txt", "1.1");
    run_git_pub(&w.d, &["tag", "v1.1.0", &v11]);
    run_git_pub(&w.ws, &["switch", "-q", "-c", "bump"]);
    let pin = |revision: &str| format!("{}\", revision = \"{}\"", w.d.display(), revision);
    let config = std::fs::read_to_string(w.ws.join(".gitscale.toml"))
        .unwrap()
        .replace(&pin("v1.0.0"), &pin("v1.1.0"));
    std::fs::write(w.ws.join(".gitscale.toml"), config).unwrap();
    run_git_pub(&w.ws, &["commit", "-q", "-am", "bump d"]);
    run_git_pub(&w.ws, &["switch", "-q", "main"]);
    let pinned = head(&w.child("imports/d"));

    let out = gs(&w.ws, &["--for", ".", "switch", "-q", "bump"]);
    ok(&out);
    assert!(out.stdout.contains("── placing "), "{}", out.stdout);
    assert_ne!(head(&w.child("imports/d")), pinned);
    assert_eq!(head(&w.child("imports/d")), v11);
}

/// An alias of `pull` is `pull`: placed online afterwards. A shell alias is
/// a command of its own name.
#[test]
fn normal_011_an_alias_of_pull_counts_as_pull_and_a_shell_alias_does_not() {
    let w = workspace("fwd_alias");
    run_git_pub(&w.ws, &["config", "alias.up", "pull --quiet"]);
    run_git_pub(&w.ws, &["config", "alias.shup", "!true"]);
    let out = gs(&w.ws, &["up"]);
    ok(&out);
    assert!(out.stdout.contains("── placing "), "{}", out.stdout);
    let out = gs(&w.ws, &["shup"]);
    ok(&out);
    assert!(!out.stdout.contains("── placing "), "{}", out.stdout);
}

/// `--force-sync` makes the placement a forced one; `--force` is git's.
#[test]
fn normal_012_force_sync_relinks_and_force_goes_to_git() {
    let env = TestEnv::new("fwd_force");
    let b = env.create_bare_repo("b", "main", &[("b.txt", "b")]);
    let a = env.create_bare_repo(
        "a",
        "main",
        &[
            ("a.txt", "a"),
            (
                ".gitscale.toml",
                &repos(&[("libs/b", &b, ", revision = \"main\"")]),
            ),
        ],
    );
    let ws = root_clone(
        &env,
        &repos(&[
            ("imports/a", &a, ", revision = \"main\""),
            ("imports/b", &b, ", revision = \"main\""),
        ]),
    );
    ok(&gs(&ws, &["sync"]));
    let link = ws.join("imports/a/libs/b");
    assert!(link.is_symlink());
    // An unlinked checkout holding work.
    std::fs::remove_file(&link).unwrap();
    run_git_pub(
        &ws,
        &["clone", "-q", b.to_str().unwrap(), link.to_str().unwrap()],
    );
    std::fs::write(link.join("work.txt"), "mine").unwrap();

    let out = gs(&ws, &["pull", "--force", "--quiet"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stdout.contains("(modified, use --force to relink)"),
        "{}",
        out.stdout
    );
    assert!(out.stderr.contains("placement failed"), "{}", out.stderr);
    assert!(!link.is_symlink(), "kept: the --force was git's");

    let out = gs(&ws, &["--force-sync", "pull", "--quiet"]);
    ok(&out);
    assert!(link.is_symlink(), "{}", out.stdout);
}

/// `[forward] parallel` makes `pull` parallel, leaves `log` sequential, and
/// `--parallel=1` runs sequentially. A parallel run reports each failure on
/// a line of its own; a sequential one leaves that to git's output.
#[test]
fn normal_013_forward_parallel_is_the_default_for_pull_alone() {
    let w = workspace_with("fwd_parallel_default", "[forward]\nparallel = 4\n\n").on_topic();
    // The children's topic branches have no upstream: their pull fails.
    let out = gs(&w.ws, &["pull"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("FAIL  imports/d: exit 1"),
        "{}",
        out.stderr
    );

    let out = gs(&w.ws, &["--parallel=1", "pull"]);
    assert!(!out.success);
    assert!(!out.stderr.contains("FAIL  imports/d"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("3 of 3 repositories failed: imports/d, imports/b, ."),
        "{}",
        out.stderr
    );

    let out = gs(&w.ws, &["log", "no-such-ref"]);
    assert!(!out.success);
    assert!(!out.stderr.contains("FAIL  "), "{}", out.stderr);
}

/// In parallel, each repository's output is printed whole after its own
/// header, in running order.
#[test]
fn normal_014_parallel_output_is_kept_per_repository_in_order() {
    let w = workspace("fwd_parallel_order").on_topic();
    let out = gs(&w.ws, &["--parallel=4", "log", "--format=%H", "-1"]);
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec!["1/3 imports/d", "2/3 imports/b", "3/3 ."],
        "{}",
        out.stdout
    );
    let lines: Vec<&str> = out.stdout.lines().collect();
    for (i, dir) in [
        (0, &w.child("imports/d")),
        (2, &w.child("imports/b")),
        (4, &w.ws),
    ] {
        assert_eq!(lines[i + 1], head(dir), "{}", out.stdout);
    }
}

/// The exit status is 1 when a repository failed, with a last line naming
/// each, and the rest still ran.
#[test]
fn normal_015_a_failing_repository_fails_the_command_after_the_rest() {
    let w = workspace("fwd_exit").on_topic();
    // b has a config of its own; d has none.
    let out = gs(
        &w.ws,
        &[
            "--for",
            "imports/d",
            "--for",
            "imports/b",
            "ls-files",
            "--error-unmatch",
            ".gitscale.toml",
        ],
    );
    assert!(!out.success, "{}", out.stdout);
    let last = out.stderr.lines().last().unwrap_or_default();
    assert_eq!(
        last, "1 of 2 repositories failed: imports/d",
        "{}",
        out.stderr
    );
    assert_eq!(headers(&out.stdout).len(), 2, "{}", out.stdout);
}

/// Headers: the count includes skipped repositories, a count from 10 is
/// padded, a header off a terminal ends in 16 dashes, and `placing` comes
/// only when something is placed.
#[test]
fn normal_016_headers_count_pad_and_end_in_sixteen_dashes() {
    let env = TestEnv::new("fwd_headers");
    let mut entries = Vec::new();
    for i in 0..9 {
        entries.push((
            format!("imports/r{}", i),
            env.create_bare_repo(&format!("r{}", i), "main", &[("f", "x")]),
        ));
    }
    let table: Vec<(&str, &Path, &str)> = entries
        .iter()
        .map(|(dir, bare)| (dir.as_str(), bare.as_path(), ", revision = \"main\""))
        .collect();
    let ws = root_clone(&env, &repos(&table));
    ok(&gs(&ws, &["sync"]));

    let out = gs(&ws, &["--foreach", "rev-parse", "HEAD"]);
    ok(&out);
    let all = headers(&out.stdout);
    assert_eq!(all.len(), 10, "{}", out.stdout);
    assert_eq!(all[0], " 1/10 imports/r0", "{}", out.stdout);
    assert_eq!(all[9], "10/10 .", "{}", out.stdout);
    let first = out.stdout.lines().next().unwrap();
    assert!(
        first.ends_with(&format!(" {}", "─".repeat(16))) && !first.ends_with(&"─".repeat(17)),
        "{:?}",
        first
    );
    assert!(!out.stdout.contains("── placing "), "{}", out.stdout);

    let out = gs(&ws, &["--foreach", "commit", "-m", "nothing"]);
    ok(&out);
    let all = headers(&out.stdout);
    assert_eq!(all.len(), 10, "{}", out.stdout);
    assert!(
        all.iter().all(|h| h.contains("skip (nothing to commit)")),
        "{}",
        out.stdout
    );
}

/// Typed inside a child, a git command runs across the workspace the child
/// belongs to.
#[test]
fn normal_017_inside_a_child_the_workspace_is_addressed() {
    let w = workspace("fwd_from_child").on_topic();
    let src = w.child("imports/b");
    let out = gs(&src, &["rev-parse", "--abbrev-ref", "HEAD"]);
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec!["1/3 imports/d", "2/3 imports/b", "3/3 ."],
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// The terminal: editors, pagers, parallel runs
// ---------------------------------------------------------------------------

/// An editor gets the terminal, run in sequence: stdin and stdout are the
/// user's.
#[test]
fn normal_018_an_editor_gets_the_terminal() {
    let w = workspace("fwd_editor").on_topic();
    let editor =
        "sh -c 'if [ -t 0 ] && [ -t 1 ]; then echo from-a-terminal > \"$1\"; else exit 1; fi' --";
    let Some((said, success)) = in_terminal(
        &w.ws,
        &["--for", "imports/d", "commit", "--allow-empty"],
        &[("GIT_EDITOR", editor)],
    ) else {
        return;
    };
    assert!(success, "{}", said);
    assert_eq!(
        git(&w.child("imports/d"), &["log", "-1", "--format=%s"]),
        "from-a-terminal"
    );
}

/// `log` on a terminal goes through one pager, headers included; `commit`
/// never does.
#[test]
fn normal_019_log_goes_through_one_pager_and_commit_never_does() {
    let w = workspace("fwd_pager").on_topic();
    let record = w.env.repos_remote.join("paged");
    let starts = w.env.repos_remote.join("starts");
    let pager = format!(
        "echo start >> '{}'; cat >> '{}'",
        starts.display(),
        record.display()
    );
    let Some((_, success)) = in_terminal(
        &w.ws,
        &["log", "--format=%s", "-1"],
        &[("GIT_PAGER", &pager)],
    ) else {
        return;
    };
    assert!(success);
    let paged = std::fs::read_to_string(&record).unwrap();
    assert_eq!(headers(&paged).len(), 3, "{}", paged);
    assert_eq!(std::fs::read_to_string(&starts).unwrap(), "start\n");

    let _ = std::fs::remove_file(&starts);
    let Some((said, success)) = in_terminal(
        &w.ws,
        &["--for", "imports/d", "commit", "--allow-empty", "-m", "x"],
        &[("GIT_PAGER", &pager)],
    ) else {
        return;
    };
    assert!(success, "{}", said);
    assert!(!starts.exists(), "commit went through the pager");
}

/// Under `--parallel` nothing has a terminal: a command that needs an editor
/// fails, and says to run without the flag.
#[test]
fn normal_020_a_parallel_editor_fails_and_names_the_flag() {
    let w = workspace("fwd_parallel_editor").on_topic();
    let out = gs(
        &w.ws,
        &[
            "--parallel",
            "--for",
            "imports/d",
            "commit",
            "--allow-empty",
        ],
    );
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("(--parallel: no terminal; run without it)"),
        "{}",
        out.stderr
    );
}

/// Under `--parallel`, a remote that asks for credentials fails the fetch
/// at once rather than wait for an answer nobody can give.
#[test]
fn normal_021_a_parallel_credential_prompt_fails_instead_of_hanging() {
    let w = workspace("fwd_parallel_prompt").on_topic();
    let http = crate::support::git_http::GitHttp::start(&w.env.repos_remote);
    let store = crate::support::worktrees::common_dir(&w.ws)
        .join("gitscale/repos")
        .join(gitscale::store::entry_name(&w.d.display().to_string()));
    run_git_pub(
        &store,
        &[
            "--git-dir",
            store.to_str().unwrap(),
            "remote",
            "set-url",
            "origin",
            &http.url("127.0.0.1", "d.git"),
        ],
    );
    let home = w.env.repos_remote.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let started = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args([
            "-C",
            w.ws.to_str().unwrap(),
            "--parallel",
            "--for",
            "imports/d",
            "fetch",
        ])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("FAIL  imports/d"), "{}", stderr);
    assert!(started.elapsed() < std::time::Duration::from_secs(60));
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// Naming a checkout that is not on the topic fails before anything runs,
/// with the command that brings it in.
#[test]
fn error_022_for_refuses_a_checkout_off_the_topic() {
    let w = workspace("fwd_select_off");
    let out = gs(
        &w.ws,
        &["--for", "imports/d", "commit", "--allow-empty", "-m", "x"],
    );
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("imports/d is not on the topic: git topic join imports/d"),
        "{}",
        out.stderr
    );
    assert!(out.stdout.is_empty(), "{}", out.stdout);
}

/// `--force-sync` asks for a forced placement, which `fetch` does not make.
#[test]
fn error_023_force_sync_with_fetch_is_refused() {
    let w = workspace("fwd_force_fetch");
    let out = gs(&w.ws, &["--force-sync", "fetch"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("--force-sync has no effect with fetch"),
        "{}",
        out.stderr
    );
}

/// Forwarding options are for git commands: before a GitScale command they
/// are refused rather than ignored.
#[test]
fn error_024_forwarding_options_with_a_gitscale_command_are_refused() {
    let w = workspace("fwd_options_misplaced");
    let out = gs(&w.ws, &["--foreach", "sync"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("are for git commands"),
        "{}",
        out.stderr
    );
    let _ = w.b;
}

/// The links gitscale plants in a checkout are not its owner's work: staging
/// everything on the topic stages the real changes, never the links.
#[test]
fn edge_025_add_all_never_stages_the_links_gitscale_planted() {
    let w = workspace("fwd_planted_links").on_topic();
    let b = w.child("imports/b");
    assert!(b.join("libs/d").is_symlink(), "the link is planted");
    crate::support::edit(&b.join("README.md"), "real work");
    ok(&gs(&w.ws, &["--for", "imports/b", "add", "-A"]));
    let staged = git(&b, &["diff", "--cached", "--name-only"]);
    assert!(staged.contains("README.md"), "{}", staged);
    assert!(
        !staged.contains("libs/d"),
        "the planted link was staged: {}",
        staged
    );
}

/// The git processes the binary started for `args`, run against `ws`.
fn traced_in(ws: &Path, args: &[&str]) -> (bool, Vec<Vec<String>>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static RUN: AtomicUsize = AtomicUsize::new(0);
    let dir = ws.with_extension(format!("trace-{}", RUN.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(["-C", ws.to_str().unwrap()])
        .args(args)
        .env("GIT_TRACE2_EVENT", &dir)
        .output()
        .unwrap();
    let mut argvs = Vec::new();
    for file in std::fs::read_dir(&dir).unwrap().flatten() {
        for line in std::fs::read_to_string(file.path())
            .unwrap_or_default()
            .lines()
        {
            let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if event["event"] == "start" {
                if let Some(argv) = event["argv"].as_array() {
                    argvs.push(
                        argv.iter()
                            .map(|a| a.as_str().unwrap_or_default().to_string())
                            .collect(),
                    );
                }
            }
        }
    }
    (out.status.success(), argvs)
}

/// A command that moved a `HEAD` but left nothing for resolution to lack
/// places the workspace without fetching a single store; `pull` fetches
/// each one.
#[test]
fn perf_026_a_moved_head_places_without_fetching_what_is_not_missing() {
    let w = workspace("fwd_no_fetch");
    let (ok, argvs) = traced_in(&w.ws, &["commit", "-q", "--allow-empty", "-m", "here"]);
    assert!(ok);
    let fetches = crate::support::resolution::fetches(&argvs);
    assert!(fetches.is_empty(), "{:?}", fetches);

    let (ok, argvs) = traced_in(&w.ws, &["pull", "-q"]);
    assert!(ok);
    for bare in [&w.b, &w.d] {
        assert_eq!(
            crate::support::resolution::store_fetches(&argvs, bare),
            1,
            "{}",
            bare.display()
        );
    }
}

/// GitScale's options may follow the git command too, up to `--`; what is
/// left is git's.
#[test]
fn normal_027_gitscale_options_after_the_git_command_are_gitscales() {
    let w = workspace("fwd_options_after");
    let out = gs(
        &w.ws,
        &[
            "log",
            "--format=%H",
            "-1",
            "--foreach",
            "--for",
            "imports/d",
        ],
    );
    ok(&out);
    assert_eq!(
        headers(&out.stdout),
        vec!["1/1 imports/d"],
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&head(&w.child("imports/d"))),
        "{}",
        out.stdout
    );

    // After `--` they are git's: a path, here, that matches nothing.
    let out = gs(&w.ws, &["log", "--oneline", "-1", "--", "--foreach"]);
    ok(&out);
    assert_eq!(headers(&out.stdout), vec!["1/1 ."], "{}", out.stdout);
}
