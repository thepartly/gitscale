//! `gitscale skill`: the agent skill, installed into a home of the test's own.

use crate::support::skill::*;
use std::fs;
use std::process::Command;

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_install_status_and_remove() {
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
    assert!(text.contains("git topic join <dir>"), "{}", text);
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
    // Refused before anything was written: the edit stays, and so does the
    // other copy.
    assert!(fs::read_to_string(&shared)
        .unwrap()
        .contains("Do not edit a pin"));
    assert_eq!(fs::read_to_string(&claude).unwrap(), text);
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
fn normal_002_the_hint_shows_once_for_every_worktree_of_a_root() {
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
fn normal_003_no_hint_once_a_skill_is_installed() {
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

/// An interactive run outside CI rewrites an installed skill older than the
/// binary, and says so on stderr: the skill follows the binary without
/// anyone reinstalling it.
#[test]
fn normal_009_an_interactive_run_refreshes_an_older_skill() {
    if !have_script() {
        eprintln!("skipped: script(1) is not installed");
        return;
    }
    let base = home("refresh-pty");
    let (root, _) = root_with_worktree(&base);
    let user = base.join("user");
    let (ok, _, err) = skill(&user, &["install"]);
    assert!(ok, "{}", err);
    let path = user.join(".agents/skills/gitscale/SKILL.md");
    restamp(&path, "0.0.1");
    let (_, out, _) = skill(&user, &["status"]);
    assert!(out.contains("installed, 0.0.1 (older"), "{}", out);

    let (ok, printed) = interactive(&user, &root, &["ls"], &[]);
    assert!(ok, "{}", printed);
    assert!(
        printed.contains(&format!(
            "updated the gitscale agent skill at {}",
            path.display()
        )),
        "{}",
        printed
    );
    let (_, out, _) = skill(&user, &["status"]);
    assert!(
        out.contains(&format!(
            "{}: installed, {}",
            path.display(),
            env!("CARGO_PKG_VERSION")
        )),
        "{}",
        out
    );
    let _ = fs::remove_dir_all(&base);
}

/// The hint, end to end: an interactive `ls` table with no skill installed
/// prints it once, and every later run in that root stays quiet.
#[test]
fn normal_010_an_interactive_ls_table_hints_once() {
    if !have_script() {
        eprintln!("skipped: script(1) is not installed");
        return;
    }
    let base = home("hint-pty");
    let (root, second) = root_with_worktree(&base);
    let user = base.join("user");

    let (ok, printed) = interactive(&user, &root, &["ls"], &[]);
    assert!(ok, "{}", printed);
    assert!(
        printed.contains("tip: gitscale skill install teaches coding agents this workflow"),
        "{}",
        printed
    );
    assert!(root.join(".git/gitscale/skill-hint").is_file());
    for dir in [&root, &second] {
        let (ok, printed) = interactive(&user, dir, &["ls"], &[]);
        assert!(ok, "{}", printed);
        assert!(!printed.contains("tip:"), "{}", printed);
    }
    assert!(!user.join(".agents").exists(), "the hint installs nothing");
    let _ = fs::remove_dir_all(&base);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn edge_004_without_claude_code_only_the_shared_location() {
    let home = home("shared");
    let (ok, _, err) = skill(&home, &["install"]);
    assert!(ok, "{}", err);
    assert!(home.join(".agents/skills/gitscale/SKILL.md").is_file());
    assert!(!home.join(".claude").exists(), "no ~/.claude made");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn edge_005_sync_and_ls_stay_quiet_when_nobody_is_watching() {
    let base = home("hint-plain");
    let (root, _) = root_with_worktree(&base);
    for args in [&["ls"][..], &["sync"]] {
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

/// The skill's frontmatter is part of what gitscale wrote: a copy whose
/// `description` was edited by hand is edited, and neither `status` nor
/// `install` may treat it as untouched — `install` would overwrite the edit.
#[test]
#[ignore = "bug: the skill's hash leaves out its frontmatter, so edits there are overwritten"]
fn edge_008_a_copy_edited_in_its_frontmatter_counts_as_edited_by_hand() {
    let home = home("frontmatter");
    let (ok, _, err) = skill(&home, &["install"]);
    assert!(ok, "{}", err);
    let path = home.join(".agents/skills/gitscale/SKILL.md");
    restamp(&path, "0.0.1");
    let edited = fs::read_to_string(&path).unwrap().replacen(
        "description: ",
        "description: Tuned by hand. ",
        1,
    );
    fs::write(&path, &edited).unwrap();

    let (_, out, _) = skill(&home, &["status"]);
    assert!(out.contains("edited by hand"), "{}", out);
    let (ok, _, _) = skill(&home, &["install"]);
    assert!(!ok);
    assert_eq!(fs::read_to_string(&path).unwrap(), edited);
    let _ = fs::remove_dir_all(&home);
}

/// No hint where nobody should see one: `ls` as JSON, a run in CI, and a
/// command that failed. None of them uses up the one hint either.
#[test]
fn edge_011_no_hint_for_json_in_ci_or_after_a_failure() {
    if !have_script() {
        eprintln!("skipped: script(1) is not installed");
        return;
    }
    let base = home("hint-quiet-pty");
    let (root, _) = root_with_worktree(&base);
    let user = base.join("user");
    let marker = root.join(".git/gitscale/skill-hint");

    let (ok, printed) = interactive(&user, &root, &["ls", "--format", "json"], &[]);
    assert!(ok, "{}", printed);
    assert!(!printed.contains("tip:"), "{}", printed);
    let (ok, printed) = interactive(&user, &root, &["ls"], &[("CI", "true")]);
    assert!(ok, "{}", printed);
    assert!(!printed.contains("tip:"), "{}", printed);

    fs::write(
        root.join(".gitscale.toml"),
        "[repos]\n\"libs/gone\" = { url = \"/nonexistent/gone.git\", revision = \"main\" }\n",
    )
    .unwrap();
    let (ok, printed) = interactive(&user, &root, &["sync"], &[]);
    assert!(!ok, "{}", printed);
    assert!(!printed.contains("tip:"), "{}", printed);
    assert!(!marker.exists());
    let _ = fs::remove_dir_all(&base);
}

/// An explicit `install` replaces a copy a newer gitscale wrote.
///
/// Current behaviour, pinned: only the implicit refresh promises never to
/// downgrade; whether `install` should refuse a newer copy without `--force`
/// is the owner's call.
#[test]
fn edge_013_install_replaces_a_newer_copy() {
    let home = home("newer");
    let (ok, _, err) = skill(&home, &["install"]);
    assert!(ok, "{}", err);
    let path = home.join(".agents/skills/gitscale/SKILL.md");
    restamp(&path, "99.0.0");
    let (_, out, _) = skill(&home, &["status"]);
    assert!(out.contains("newer than this gitscale"), "{}", out);

    let (ok, out, err) = skill(&home, &["install"]);
    assert!(ok, "{}", err);
    assert!(
        out.contains(&format!("install  {}", path.display())),
        "{}",
        out
    );
    let (_, out, _) = skill(&home, &["status"]);
    assert!(
        out.contains(&format!("installed, {}", env!("CARGO_PKG_VERSION"))),
        "{}",
        out
    );
    let _ = fs::remove_dir_all(&home);
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// A file at the skill's path that gitscale did not write is somebody
/// else's: `install` refuses to replace it until told to with `--force`.
#[test]
fn error_006_install_replaces_a_file_gitscale_did_not_write_only_with_force() {
    let home = home("foreign");
    let path = home.join(".agents/skills/gitscale/SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let theirs = "---\nname: gitscale\n---\nsomebody else's\n";
    fs::write(&path, theirs).unwrap();

    let (ok, _, err) = skill(&home, &["install"]);
    assert!(!ok);
    assert!(
        err.contains("a file gitscale did not write; pass --force"),
        "{}",
        err
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs);

    let (ok, _, err) = skill(&home, &["install", "--force"]);
    assert!(ok, "{}", err);
    let (_, out, _) = skill(&home, &["status"]);
    assert!(
        out.contains(&format!("installed, {}", env!("CARGO_PKG_VERSION"))),
        "{}",
        out
    );
    let _ = fs::remove_dir_all(&home);
}

/// Without a home there is nowhere to put the skill: each `skill` command
/// says so, for `HOME` unset and for `HOME` empty alike.
#[test]
fn error_007_skill_commands_need_a_home() {
    for home in [None, Some("")] {
        for action in ["install", "status", "remove"] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_gitscale"));
            cmd.args(["skill", action]);
            match home {
                None => cmd.env_remove("HOME"),
                Some(value) => cmd.env("HOME", value),
            };
            let output = cmd.output().unwrap();
            let err = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{:?} {}", home, action);
            assert!(
                err.contains("HOME is not set"),
                "{:?} {}: {}",
                home,
                action,
                err
            );
        }
    }
}

/// `skill remove` removes all of what gitscale wrote or none of it: a file
/// gitscale did not write at one path stops the whole removal before any
/// copy is deleted, as `install` checks every path before writing any.
#[test]
#[ignore = "bug: skill remove deletes the first copy, then fails on a foreign second one"]
fn error_012_remove_with_a_foreign_copy_removes_nothing() {
    let home = home("remove-foreign");
    fs::create_dir_all(home.join(".claude")).unwrap();
    let (ok, _, err) = skill(&home, &["install"]);
    assert!(ok, "{}", err);
    let shared = home.join(".agents/skills/gitscale/SKILL.md");
    let claude = home.join(".claude/skills/gitscale/SKILL.md");
    fs::write(&claude, "---\nname: gitscale\n---\nsomebody else's\n").unwrap();

    let (ok, _, err) = skill(&home, &["remove"]);
    assert!(!ok);
    assert!(err.contains("a file gitscale did not write"), "{}", err);
    assert!(claude.is_file());
    assert!(shared.is_file(), "the shared copy was removed anyway");
    let _ = fs::remove_dir_all(&home);
}
