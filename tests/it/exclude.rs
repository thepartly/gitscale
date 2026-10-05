//! What GitScale puts in a working tree — checkout directories in the root,
//! dependency links in each checkout — kept out of git's sight in each
//! repository's `info/exclude`, one block per worktree.

use crate::support::resolution::repos;
use crate::support::worktrees::{fixture, git, gs, ok};
use crate::support::{run_git_pub, TestEnv};
use std::path::Path;

fn exclude_file(worktree: &Path) -> std::path::PathBuf {
    let path = git(worktree, &["rev-parse", "--git-path", "info/exclude"]);
    worktree.join(path)
}

/// With no `.gitignore` entry for them, the checkouts are not untracked
/// files of the root: `git status` is clean, and `git scale add -A` stages
/// none of them into the root.
#[test]
fn normal_001_checkouts_are_not_untracked_files_of_the_root() {
    let f = fixture("exclude_root", "");
    let ws = f.clone_root("ws");
    assert!(ws.join("imports/core/lib.txt").is_file());
    assert!(!std::fs::read_to_string(ws.join(".gitignore"))
        .unwrap_or_default()
        .contains("imports"));
    assert_eq!(git(&ws, &["status", "--porcelain"]), "");

    std::fs::write(ws.join("notes.txt"), "mine").unwrap();
    ok(&gs(&ws, &["--for", ".", "add", "-A"]));
    assert_eq!(git(&ws, &["diff", "--cached", "--name-only"]), "notes.txt");
}

/// Lines of one's own in `info/exclude` stay; every root worktree has a block
/// of its own; the block of a worktree that is gone goes.
#[test]
fn normal_002_each_worktree_has_its_own_block_and_other_lines_stay() {
    let f = fixture("exclude_blocks", "");
    let ws = f.clone_root("ws");
    let file = exclude_file(&ws);
    let mine = format!("*.local\n{}", std::fs::read_to_string(&file).unwrap());
    std::fs::write(&file, &mine).unwrap();

    let other = f.env.repos_remote.join("other");
    run_git_pub(
        &ws,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/o",
            other.to_str().unwrap(),
        ],
    );
    ok(&gs(&other, &["sync"]));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("*.local\n"), "{}", text);
    for dir in [&ws, &other] {
        let key = format!("# gitscale: {}", dir.canonicalize().unwrap().display());
        assert!(text.contains(&key), "{}", text);
    }
    assert_eq!(text.matches("/imports/core\n").count(), 2, "{}", text);

    std::fs::remove_dir_all(&other).unwrap();
    ok(&gs(&ws, &["sync"]));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains(&other.display().to_string()), "{}", text);
    assert!(text.contains("*.local"), "{}", text);
}

/// A checkout's own block lists the checkouts inside it, and a name with
/// pattern characters in it is taken literally.
#[test]
fn edge_003_nested_checkouts_and_odd_names_are_excluded_literally() {
    let env = TestEnv::new("exclude_nested");
    let outer = env.create_bare_repo("outer", "main", &[("o.txt", "o")]);
    let inner = env.create_bare_repo("inner", "main", &[("i.txt", "i")]);
    env.write_config(&repos(&[
        ("deps", &outer, ", revision = \"main\""),
        ("deps/in[1]", &inner, ", revision = \"main\""),
    ]));
    env.init_playground_git();
    assert!(env.run(&["sync"]).success);
    // Only the config, not committed here, is untracked in the root.
    assert_eq!(
        git(&env.playground, &["status", "--porcelain"]),
        "?? .gitscale.toml"
    );
    assert_eq!(
        git(&env.playground.join("deps"), &["status", "--porcelain"]),
        ""
    );
    let text = std::fs::read_to_string(exclude_file(&env.playground.join("deps"))).unwrap();
    assert!(text.contains("/in\\[1\\]\n"), "{}", text);
}
