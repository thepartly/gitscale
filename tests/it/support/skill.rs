//! Helpers for the skill tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
pub fn home(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/playground")
        .join(format!("{}-skill-{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn skill(home: &Path, args: &[&str]) -> (bool, String, String) {
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

pub fn git(cwd: &Path, args: &[&str]) {
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
pub fn root_with_worktree(base: &Path) -> (PathBuf, PathBuf) {
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

/// `gitscale <args>` run in `dir` on a pseudo-terminal, as a person at a
/// terminal would run it — the only way to reach what gitscale does only for
/// a run somebody is watching — with `HOME` pointed at `home` and `vars` on
/// top. Returns whether it succeeded and everything it printed, stdout and
/// stderr together.
pub fn interactive(
    home: &Path,
    dir: &Path,
    args: &[&str],
    vars: &[(&str, &str)],
) -> (bool, String) {
    let mut line = format!("'{}'", env!("CARGO_BIN_EXE_gitscale"));
    for arg in args {
        line.push_str(&format!(" '{}'", arg));
    }
    let mut cmd = Command::new("script");
    cmd.args(["-qec", &line, "/dev/null"])
        .current_dir(dir)
        .env("HOME", home)
        .stdin(std::process::Stdio::null());
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let output = cmd.output().expect("script(1) runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// Whether `script(1)` is there to give a command a pseudo-terminal.
pub fn have_script() -> bool {
    Command::new("script")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The skill file at `path`, with the version its header records replaced by
/// `version` — the hash covers only the text below the header, so it still
/// matches: what an older or newer gitscale would have written.
pub fn restamp(path: &Path, version: &str) {
    let text = fs::read_to_string(path).unwrap();
    let mark = format!("<!-- gitscale-skill {} ", env!("CARGO_PKG_VERSION"));
    assert!(text.contains(&mark), "{}", text);
    fs::write(
        path,
        text.replace(&mark, &format!("<!-- gitscale-skill {} ", version)),
    )
    .unwrap();
}
