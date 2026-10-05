//! Helpers for the ci cache tests.

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
        "git {:?} in {} failed:\n{}{}",
        args,
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn git_ok(cwd: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn alternates_of(repo: &Path) -> Option<String> {
    std::fs::read_to_string(repo.join(".git/objects/info/alternates"))
        .ok()
        .map(|s| s.trim().to_string())
}

/// Add a commit to a bare repo through a throwaway clone, and return its SHA.
pub fn commit_to_bare(bare: &Path, branch: &str, file: &str, content: &str) -> String {
    let tmp = bare.with_extension("edit");
    let _ = std::fs::remove_dir_all(&tmp);
    git(
        bare.parent().unwrap(),
        &[
            "clone",
            "--quiet",
            bare.to_str().unwrap(),
            tmp.to_str().unwrap(),
        ],
    );
    git(&tmp, &["config", "user.email", "t@t"]);
    git(&tmp, &["config", "user.name", "T"]);
    git(&tmp, &["checkout", "--quiet", branch]);
    std::fs::write(tmp.join(file), content).unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "--quiet", "-m", "another"]);
    git(&tmp, &["push", "--quiet", "origin", branch]);
    let sha = git(&tmp, &["rev-parse", "HEAD"]);
    let _ = std::fs::remove_dir_all(&tmp);
    sha
}

/// Backdate a file, so eviction can be tested without waiting for a month to
/// pass.
pub fn age(path: &Path, when: &str) {
    let ok = std::process::Command::new("touch")
        .args(["-d", when])
        .arg(path)
        .status()
        .expect("failed to run touch")
        .success();
    assert!(ok, "could not backdate {}", path.display());
}

pub fn config_for(url: &str) -> String {
    format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        url
    )
}

/// A sync as a CI job would run it, against this test's own cache.
pub fn ci_pull(env: &TestEnv) -> super::CliOutput {
    env.run_with_env(&[("CI", "1")], &["sync"])
}

/// A `gitscale cache …` command against this test's own cache, CI or not.
pub fn cache_cmd(env: &TestEnv, args: &[&str]) -> super::CliOutput {
    let mut full = vec!["cache"];
    full.extend_from_slice(args);
    env.run_with_env(&[], &full)
}
