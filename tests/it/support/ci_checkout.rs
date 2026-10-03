//! Helpers for the ci checkout tests.

use super::TestEnv;
use std::path::Path;
pub fn git(cwd: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {:?} in {} failed:\n{}",
        args,
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A bare repo whose `main` has commits `c1`..`c3`, tagged `v1`..`v3`, plus a
/// `feature` branch off `v1`.
pub fn tagged_remote(env: &TestEnv) -> std::path::PathBuf {
    let bare = env.create_bare_repo("lib", "main", &[("f.txt", "0")]);
    let work = env.repos_remote.join("lib-work");
    git(
        &env.repos_remote,
        &[
            "clone",
            "-q",
            "-b",
            "main",
            bare.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    git(&work, &["config", "user.email", "t@t"]);
    git(&work, &["config", "user.name", "T"]);
    for i in 1..=3 {
        std::fs::write(work.join("f.txt"), i.to_string()).unwrap();
        git(&work, &["commit", "-qam", &format!("c{}", i)]);
        git(&work, &["tag", &format!("v{}", i)]);
    }
    git(&work, &["checkout", "-q", "-b", "feature", "v1"]);
    std::fs::write(work.join("f.txt"), "feature").unwrap();
    git(&work, &["commit", "-qam", "feat"]);
    git(
        &work,
        &["push", "-q", "origin", "main", "feature", "--tags"],
    );
    bare
}

pub fn pin(env: &TestEnv, bare: &Path, revision: &str, cache: bool) {
    let cache = if cache {
        format!("[cache]\ndir = \"{}\"\n\n", env.cache.display())
    } else {
        "[cache]\nenabled = false\n\n".to_string()
    };
    env.write_config(&format!(
        // file:// — git ignores `--depth` for a plain-path remote, and these
        // tests are about the shallow checkout CI makes.
        "{}[repos]\n\"libs/lib\" = {{ url = \"file://{}\", revision = \"{}\" }}\n",
        cache,
        bare.display(),
        revision
    ));
}

pub fn subject(dir: &Path) -> String {
    git(dir, &["log", "-1", "--format=%s"])
}

pub fn gitlab_workspace(env: &TestEnv, extra_config: &str) {
    env.init_playground_git();
    // As the docs recommend — and what makes `-ffd`, without `-x`, keep them.
    std::fs::write(env.playground.join(".gitignore"), "/imports/\n").unwrap();
    let bare = env.create_bare_repo("lib", "main", &[("f.txt", "0")]);
    env.write_config(&format!(
        "{}[repos]\n\"imports/lib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        extra_config,
        bare.display()
    ));
}
