//! `gitscale skill`: the agent skill, installed into a home of the test's own.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn home(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/playground")
        .join(format!("{}-skill-{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn skill(home: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .arg("skill")
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn install_status_and_remove() {
    let home = home("cli");
    fs::create_dir_all(home.join(".claude")).unwrap();
    let shared = home.join(".agents/skills/gitscale/SKILL.md");
    let claude = home.join(".claude/skills/gitscale/SKILL.md");

    let (ok, out, err) = skill(&home, &["status"]);
    assert!(ok, "{}", err);
    assert!(out.contains("SKILL.md: not installed"), "{}", out);

    let (ok, out, err) = skill(&home, &["install"]);
    assert!(ok, "{}", err);
    assert!(
        out.contains(&format!("install  {}", shared.display())),
        "{}",
        out
    );
    assert!(
        out.contains(&format!("install  {}", claude.display())),
        "{}",
        out
    );
    let text = fs::read_to_string(&shared).unwrap();
    assert!(text.starts_with("---\nname: gitscale\n"), "{}", text);
    assert!(text.contains("gitscale develop <dir>"), "{}", text);
    assert_eq!(text, fs::read_to_string(&claude).unwrap());

    let (_, out, _) = skill(&home, &["install"]);
    assert!(out.contains("(already current)"), "{}", out);
    let (_, out, _) = skill(&home, &["status"]);
    assert!(
        out.contains(&format!("installed, {}", env!("CARGO_PKG_VERSION"))),
        "{}",
        out
    );

    // Edited by hand: install and remove keep their hands off it.
    fs::write(
        &shared,
        text.replace("Never edit a pin", "Do not edit a pin"),
    )
    .unwrap();
    let (ok, _, err) = skill(&home, &["install"]);
    assert!(!ok);
    assert!(err.contains("edited by hand; pass --force"), "{}", err);
    let (ok, _, err) = skill(&home, &["remove"]);
    assert!(!ok, "{}", err);
    let (ok, out, err) = skill(&home, &["remove", "--force"]);
    assert!(ok, "{}", err);
    assert!(out.contains("remove  "), "{}", out);
    assert!(!shared.exists() && !claude.exists());
    let (_, out, _) = skill(&home, &["remove"]);
    assert!(out.contains("Nothing to remove"), "{}", out);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn without_claude_code_only_the_shared_location() {
    let home = home("shared");
    let (ok, _, err) = skill(&home, &["install"]);
    assert!(ok, "{}", err);
    assert!(home.join(".agents/skills/gitscale/SKILL.md").is_file());
    assert!(!home.join(".claude").exists(), "no ~/.claude made");
    let _ = fs::remove_dir_all(&home);
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A root with one commit and a second worktree beside it.
fn root_with_worktree(base: &Path) -> (PathBuf, PathBuf) {
    let root = base.join("root");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    fs::write(root.join(".gitscale.toml"), "").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "root"]);
    let second = base.join("second");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/x",
            second.to_str().unwrap(),
        ],
    );
    (root, second)
}

#[test]
fn the_hint_shows_once_for_every_worktree_of_a_root() {
    let base = home("hint");
    let (root, second) = root_with_worktree(&base);
    let user = base.join("user");

    let mut err = Vec::new();
    assert!(gitscale::skill::hint(&user, &root, &mut err));
    assert_eq!(
        String::from_utf8(err).unwrap(),
        "tip: gitscale skill install teaches coding agents this workflow\n"
    );
    assert!(root.join(".git/gitscale/skill-hint").is_file());

    let mut err = Vec::new();
    assert!(!gitscale::skill::hint(&user, &root, &mut err));
    assert!(!gitscale::skill::hint(&user, &second, &mut err));
    assert!(err.is_empty());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn no_hint_once_a_skill_is_installed() {
    let base = home("hint-installed");
    let (root, _) = root_with_worktree(&base);
    let user = base.join("user");
    gitscale::skill::install(&user, false).unwrap();

    let mut err = Vec::new();
    assert!(!gitscale::skill::hint(&user, &root, &mut err));
    assert!(err.is_empty());
    assert!(!root.join(".git/gitscale/skill-hint").exists());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn pull_and_status_stay_quiet_when_nobody_is_watching() {
    let base = home("hint-plain");
    let (root, _) = root_with_worktree(&base);
    for args in [&["status"][..], &["pull"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_gitscale"))
            .args(args)
            .current_dir(&root)
            .env("HOME", base.join("user"))
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(!err.contains("tip:"), "{:?}: {}", args, err);
    }
    assert!(!root.join(".git/gitscale/skill-hint").exists());
    let _ = fs::remove_dir_all(&base);
}
