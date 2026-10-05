//! How output looks: colour and icons only for a terminal, unless `--color`
//! says otherwise; `NO_COLOR` and `TERM=dumb` turn them off; and git, run
//! across the workspace, colours exactly when GitScale does.

use crate::support::TestEnv;
use std::process::Command;

/// A workspace of one entry, placed.
fn one_entry(name: &str) -> TestEnv {
    let env = TestEnv::new(name);
    let lib = env.create_bare_repo("lib", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[repos]\n\"imports/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        lib.display()
    ));
    env.init_playground_git();
    assert!(env.run(&["sync"]).success);
    env
}

/// The binary with `args`, from the playground, `HOME` the test's own and
/// `vars` set; stdout piped.
fn piped(env: &TestEnv, args: &[&str], vars: &[(&str, &str)]) -> (String, String) {
    let home = env.repos_remote.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gitscale"));
    cmd.args(args)
        .current_dir(&env.playground)
        .env("HOME", &home)
        .env_remove("NO_COLOR");
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let out = cmd.output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The same under a pseudo-terminal: the transcript. `None` without
/// `script`.
fn on_a_terminal(env: &TestEnv, args: &[&str], vars: &[(&str, &str)]) -> Option<String> {
    let home = env.repos_remote.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let quoted: Vec<String> = std::iter::once(env!("CARGO_BIN_EXE_gitscale"))
        .chain(args.iter().copied())
        .map(|a| format!("'{}'", a))
        .collect();
    let mut cmd = Command::new("script");
    cmd.args(["-qec", &quoted.join(" "), "/dev/null"])
        .current_dir(&env.playground)
        .env("HOME", &home)
        .env("TERM", "xterm")
        .env_remove("NO_COLOR");
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let out = cmd.output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

const ESC: &str = "\x1b[";

/// The table piped — into a file, a CI log — has no escape codes.
#[test]
fn normal_001_a_piped_table_has_no_escape_codes() {
    let env = one_entry("out_ls_piped");
    let (stdout, _) = piped(&env, &["ls"], &[]);
    assert!(stdout.contains("imports/lib"), "{}", stdout);
    assert!(!stdout.contains(ESC), "{:?}", stdout);
}

/// Without colour the result lines are exactly the words: `  ok    …`.
#[test]
fn normal_002_without_colour_result_lines_are_the_words_alone() {
    let env = one_entry("out_plain_lines");
    let (stdout, _) = piped(&env, &["sync"], &[]);
    assert!(
        stdout.lines().any(|l| l == "  ok    imports/lib"),
        "{:?}",
        stdout
    );
    assert!(!stdout.contains('✔'), "{:?}", stdout);
}

/// `--color always` colours piped output, icons included.
#[test]
fn normal_003_color_always_colours_piped_output() {
    let env = one_entry("out_color_always");
    let (stdout, _) = piped(&env, &["--color", "always", "sync"], &[]);
    assert!(stdout.contains('✔') && stdout.contains(ESC), "{:?}", stdout);
    let (stdout, _) = piped(&env, &["ls", "--color", "always"], &[]);
    assert!(stdout.contains(ESC), "{:?}", stdout);
}

/// On a terminal, `NO_COLOR`, `TERM=dumb` and `--color never` each turn
/// colour and icons off; with none of them they are on.
#[test]
fn normal_004_no_color_dumb_terminals_and_never_turn_colour_off() {
    let env = one_entry("out_terminal_off");
    let Some(on) = on_a_terminal(&env, &["ls"], &[]) else {
        return;
    };
    assert!(on.contains(ESC), "{:?}", on);
    for (args, vars) in [
        (&["ls"][..], &[("NO_COLOR", "1")][..]),
        (&["ls"], &[("TERM", "dumb")]),
        (&["--color", "never", "ls"], &[]),
    ] {
        let said = on_a_terminal(&env, args, vars).unwrap();
        assert!(said.contains("imports/lib"), "{:?}", said);
        assert!(!said.contains(ESC), "{:?} {:?}: {:?}", args, vars, said);
    }
}

/// Git is told `color.ui=always` exactly when GitScale colours its own
/// output, so the two agree.
#[test]
fn normal_005_git_colours_exactly_when_gitscale_does() {
    let env = one_entry("out_git_colour");
    let (stdout, _) = piped(
        &env,
        &["--color", "always", "config", "--get", "color.ui"],
        &[],
    );
    assert!(stdout.contains("always"), "{:?}", stdout);
    let (stdout, _) = piped(&env, &["config", "--get", "color.ui"], &[]);
    assert!(!stdout.contains("always"), "{:?}", stdout);
}
