//! The command line itself: global options and where they may go, `-C`,
//! `--help` and `--version`, unknown commands, and the interactive rendering a
//! terminal gets.
//!
//! Everything here runs the real binary as a subprocess, with `HOME` pointed
//! at a directory of the test's own: an interactive run refreshes the agent
//! skill under `HOME`, and the exit status is only visible from outside.

use crate::support::{strip_ansi, TestEnv};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What a subprocess run of the binary left behind.
struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// An empty `HOME` of the test's own, beside its playground.
fn home(env: &TestEnv) -> PathBuf {
    let home = env.repos_remote.join("home");
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// Run the binary with exactly `args`, from `cwd`, with an isolated `HOME`
/// and the test's own cache.
fn gitscale(env: &TestEnv, cwd: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home(env))
        .env("GITSCALE_CACHE_DIR", &env.cache)
        .env_remove("GITSCALE_HOOK_ALLOW")
        .output()
        .expect("failed to run the gitscale binary");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// Run the binary with `args` under a pseudo-terminal (`script`), so it takes
/// the interactive path: parallel progress lines instead of plain ones. The
/// terminal's transcript is everything it printed, stdout and stderr alike.
/// `None` when `script` is not available here.
fn gitscale_in_a_terminal(env: &TestEnv, args: &[&str]) -> Option<Run> {
    let quoted: Vec<String> = std::iter::once(env!("CARGO_BIN_EXE_gitscale"))
        .chain(args.iter().copied())
        .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
        .collect();
    let output = Command::new("script")
        .args(["-qec", &quoted.join(" "), "/dev/null"])
        .current_dir(&env.playground)
        .env("HOME", home(env))
        .env("GITSCALE_CACHE_DIR", &env.cache)
        .env_remove("GITSCALE_HOOK_ALLOW")
        .output()
        .ok()?;
    Some(Run {
        stdout: strip_ansi(&String::from_utf8_lossy(&output.stdout)).replace('\r', ""),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    })
}

/// A workspace with one entry pulled from a local bare repository.
fn one_entry(env: &TestEnv) {
    let bare = env.create_bare_repo("lib", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
}

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// `--help` and `--version` are answers, not failures: they print to stdout
/// and exit 0, at the top level and for a subcommand's help.
#[test]
fn normal_001_help_and_version_succeed_on_stdout() {
    let env = TestEnv::new("cli_help_version");
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["pull", "--help"],
        vec!["sync", "-h"],
    ] {
        let run = gitscale(&env, &env.playground, &args);
        assert_eq!(run.code, Some(0), "{:?}: {}", args, run.stderr);
        assert!(run.stdout.contains("Usage"), "{:?}: {}", args, run.stdout);
        assert!(run.stderr.is_empty(), "{:?}: {}", args, run.stderr);
    }
    for flag in ["--version", "-V"] {
        let run = gitscale(&env, &env.playground, &[flag]);
        assert_eq!(run.code, Some(0), "{}: {}", flag, run.stderr);
        assert_eq!(
            run.stdout.trim(),
            format!("gitscale {}", env!("CARGO_PKG_VERSION")),
            "{}",
            flag
        );
    }
}

/// `-v` and `--no-cache` are global: accepted before the subcommand or after
/// it, as docs/cli.md says.
#[test]
fn normal_002_global_flags_go_before_or_after_the_subcommand() {
    let env = TestEnv::new("cli_global_flags");
    one_entry(&env);
    let root = env.playground.to_str().unwrap();
    for args in [
        vec!["-v", "pull", "-C", root],
        vec!["pull", "-v", "-C", root],
        vec!["pull", "-C", root, "--verbose"],
        vec!["--no-cache", "pull", "-C", root],
        vec!["pull", "--no-cache", "-C", root],
    ] {
        let run = gitscale(&env, &env.playground, &args);
        assert_eq!(
            run.code,
            Some(0),
            "{:?}: {}{}",
            args,
            run.stdout,
            run.stderr
        );
    }
    assert!(env.playground.join("libs/lib/a.txt").is_file());
}

/// `-C` before the subcommand is what docs/cli.md shows
/// (`gitscale -C /path/to/project status`), and its table says the global
/// options are accepted before or after the subcommand.
#[test]
#[ignore = "bug: -C is defined per subcommand, so `gitscale -C <dir> status` is a usage error"]
fn normal_003_root_option_before_the_subcommand_is_accepted() {
    let env = TestEnv::new("cli_root_before");
    one_entry(&env);
    let elsewhere = env.repos_remote.clone();
    let run = gitscale(
        &env,
        &elsewhere,
        &["-C", env.playground.to_str().unwrap(), "pull"],
    );
    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert!(env.playground.join("libs/lib/a.txt").is_file());
}

/// Without `-C` the config is found from the working directory, searching
/// upward: a command run from a directory inside the workspace acts on the
/// whole workspace.
#[test]
fn normal_004_the_config_is_found_upward_from_the_working_directory() {
    let env = TestEnv::new("cli_cwd_upward");
    one_entry(&env);
    let inside = env.playground.join("some/where");
    std::fs::create_dir_all(&inside).unwrap();
    let run = gitscale(&env, &inside, &["pull"]);
    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert!(env.playground.join("libs/lib/a.txt").is_file());
    assert!(
        !inside.join("libs").exists(),
        "nothing is placed below the cwd"
    );
}

/// On a terminal the per-repository lines are live progress lines, and a
/// failure still fails the run: the transcript shows each repository's
/// outcome and the summary, and the exit status is 1.
#[test]
fn normal_005_a_terminal_gets_progress_lines_and_the_same_exit_status() {
    let env = TestEnv::new("cli_interactive");
    let good = env.create_bare_repo("good", "main", &[("a.txt", "a")]);
    let other = env.create_bare_repo("other", "main", &[("b.txt", "b")]);
    env.write_config(&format!(
        "[repos]\n\"libs/good\" = {{ url = \"{}\", revision = \"main\" }}\n\
         \"libs/stray\" = {{ url = \"{}\", revision = \"main\" }}\n",
        good.display(),
        other.display()
    ));
    // A directory holding something else: that one entry fails.
    std::fs::create_dir_all(env.playground.join("libs/stray")).unwrap();
    std::fs::write(env.playground.join("libs/stray/notes.txt"), "mine").unwrap();
    let Some(run) = gitscale_in_a_terminal(&env, &["pull"]) else {
        eprintln!("skipped: `script` is not available");
        return;
    };
    assert_ne!(run.code, Some(0), "{}", run.stdout);
    assert!(
        env.playground.join("libs/good/a.txt").is_file(),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("ok    libs/good")
            && run.stdout.contains("FAIL  libs/stray")
            && run.stdout.contains("1 repo(s) failed to pull"),
        "{}",
        run.stdout
    );
}

/// A terminal run is parallel. Two entries where one sits inside the other
/// (`deps` and `deps/inner`, which config allows) must both be placed there
/// as they are on the plain, sequential path: the inner one's directory must
/// not get in the way of the outer one. A race: the test tries many times.
#[test]
#[ignore = "bug: on a terminal nested entries are placed in parallel; when deps/inner lands first, deps fails with git worktree add: already exists (intermittent)"]
fn edge_006_a_terminal_places_nested_entries_like_a_plain_run() {
    for round in 0..100 {
        let env = TestEnv::new(&format!("cli_interactive_nested_{}", round));
        let outer = env.create_bare_repo("outer", "main", &[("o.txt", "o")]);
        let inner = env.create_bare_repo("inner", "main", &[("i.txt", "i")]);
        env.write_config(&format!(
            "[repos]\n\"deps\" = {{ url = \"{}\", revision = \"main\" }}\n\
             \"deps/inner\" = {{ url = \"{}\", revision = \"main\" }}\n",
            outer.display(),
            inner.display()
        ));
        let Some(run) = gitscale_in_a_terminal(&env, &["pull"]) else {
            eprintln!("skipped: `script` is not available");
            return;
        };
        assert_eq!(run.code, Some(0), "round {}: {}", round, run.stdout);
        assert!(
            env.playground.join("deps/o.txt").is_file(),
            "{}",
            run.stdout
        );
        assert!(env.playground.join("deps/inner/i.txt").is_file());
    }
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// An unknown subcommand is a usage error: exit status 1, the reason on
/// stderr, nothing on stdout.
#[test]
fn error_007_an_unknown_subcommand_fails_with_usage() {
    let env = TestEnv::new("cli_unknown_subcommand");
    let run = gitscale(&env, &env.playground, &["frobnicate"]);
    assert_eq!(run.code, Some(1));
    assert!(run.stderr.contains("frobnicate"), "{}", run.stderr);
    assert!(run.stderr.contains("Usage"), "{}", run.stderr);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
}

/// `commit` needs `-m`: without it nothing runs and the usage error names the
/// option.
#[test]
fn error_008_commit_without_a_message_is_a_usage_error() {
    let env = TestEnv::new("cli_commit_no_message");
    env.write_config("[repos]\n");
    let run = gitscale(&env, &env.playground, &["commit"]);
    assert_eq!(run.code, Some(1));
    assert!(run.stderr.contains("--message"), "{}", run.stderr);
}

/// A `-C` path that does not exist is named in the error, so the user can see
/// which of their arguments was wrong.
#[test]
#[ignore = "bug: the error is only 'cannot resolve start path': the CLI prints the outermost context and the path is never in it"]
fn error_009_a_nonexistent_root_path_is_named() {
    let env = TestEnv::new("cli_root_missing");
    let missing = env.playground.join("no/such/dir");
    let run = gitscale(
        &env,
        &env.playground,
        &["status", "-C", missing.to_str().unwrap()],
    );
    assert_eq!(run.code, Some(1));
    assert!(
        run.stderr.contains(missing.to_str().unwrap()),
        "{}",
        run.stderr
    );
}

/// Outside any workspace there is nothing to act on: the command fails, says
/// no config was found and where the search started, and creates nothing.
#[test]
fn error_010_no_config_found_says_where_it_searched() {
    let env = TestEnv::new("cli_no_config");
    let outside = env.repos_remote.join("empty");
    std::fs::create_dir_all(&outside).unwrap();
    for command in ["pull", "fetch", "sync", "push", "status"] {
        let run = gitscale(
            &env,
            &env.playground,
            &[command, "-C", outside.to_str().unwrap()],
        );
        assert_eq!(run.code, Some(1), "{}", command);
        assert!(
            run.stderr.contains("No .gitscale.toml config found")
                && run.stderr.contains(outside.to_str().unwrap()),
            "{}: {}",
            command,
            run.stderr
        );
    }
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}
