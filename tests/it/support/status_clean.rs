//! Helpers for the ls, clean and check tests.

use super::{run_git_pub, strip_ansi, TestEnv};
use std::path::{Path, PathBuf};

/// What a gitscale subprocess printed, and the exit code it ended with.
pub struct Exit {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Exit {
    pub fn said(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// Every variable that tells gitscale it runs in a pipeline, or what that
/// pipeline merges into: removed from a subprocess before a test sets its own,
/// so a developer's shell or a CI runner cannot leak into the case.
const PIPELINE_VARS: &[&str] = &[
    "CI",
    "GITLAB_CI",
    "GITHUB_ACTIONS",
    "CI_COMMIT_SHA",
    "CI_COMMIT_BRANCH",
    "CI_DEFAULT_BRANCH",
    "CI_MERGE_REQUEST_SOURCE_BRANCH_NAME",
    "CI_MERGE_REQUEST_TARGET_BRANCH_NAME",
    "CI_JOB_TOKEN",
    "GITHUB_SHA",
    "GITHUB_HEAD_REF",
    "GITHUB_BASE_REF",
    "GITHUB_REF_TYPE",
    "GITHUB_REF_NAME",
    "GITHUB_EVENT_PATH",
    "GITHUB_TOKEN",
];

/// The gitscale binary run as `gitscale -C dir <args>` with
/// `vars` set on top of an environment scrubbed of pipeline variables, the
/// CI cache pointed at `cache`.
pub fn gitscale_at(dir: &Path, cache: &Path, vars: &[(&str, &str)], args: &[&str]) -> Exit {
    let mut full: Vec<&str> = vec!["-C", dir.to_str().unwrap()];
    full.extend_from_slice(args);
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"));
    cmd.args(&full).env("GITSCALE_CACHE_DIR", cache);
    for var in PIPELINE_VARS {
        cmd.env_remove(var);
    }
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let output = cmd.output().expect("failed to run the gitscale binary");
    Exit {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The same, in the test's playground.
pub fn gitscale(env: &TestEnv, vars: &[(&str, &str)], args: &[&str]) -> Exit {
    gitscale_at(&env.playground, &env.cache, vars, args)
}

/// A root repository whose `.gitscale.toml` is `config`, cloned the way a
/// runner checks it out — detached at its commit — at `repos_remote/job`.
pub fn job_checkout(env: &TestEnv, config: &str) -> PathBuf {
    let root = env.create_bare_repo("root", "main", &[(".gitscale.toml", config)]);
    let job = env.repos_remote.join("job");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), job.to_str().unwrap()],
    );
    run_git_pub(&job, &["checkout", "-q", "--detach"]);
    job
}

/// The variables of a GitLab merge request pipeline from `source` into
/// `target`, for the root checked out at `job`.
pub fn gitlab_merge_request(job: &Path, source: &str, target: &str) -> Vec<(String, String)> {
    vec![
        ("CI".into(), "true".into()),
        ("GITLAB_CI".into(), "true".into()),
        (
            "CI_COMMIT_SHA".into(),
            super::git_stdout(job, &["rev-parse", "HEAD"]),
        ),
        ("CI_MERGE_REQUEST_SOURCE_BRANCH_NAME".into(), source.into()),
        ("CI_MERGE_REQUEST_TARGET_BRANCH_NAME".into(), target.into()),
        ("CI_DEFAULT_BRANCH".into(), "main".into()),
    ]
}

/// The variables of a GitLab branch pipeline on `branch`.
pub fn gitlab_branch(job: &Path, branch: &str) -> Vec<(String, String)> {
    vec![
        ("CI".into(), "true".into()),
        ("GITLAB_CI".into(), "true".into()),
        (
            "CI_COMMIT_SHA".into(),
            super::git_stdout(job, &["rev-parse", "HEAD"]),
        ),
        ("CI_COMMIT_BRANCH".into(), branch.into()),
        ("CI_DEFAULT_BRANCH".into(), "main".into()),
    ]
}

/// Borrow owned variable pairs the way `gitscale_at` takes them.
pub fn borrowed(vars: &[(String, String)]) -> Vec<(&str, &str)> {
    vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// `git scale check` in the job checkout `job`, with `vars`.
pub fn check_job(env: &TestEnv, job: &Path, vars: &[(String, String)]) -> Exit {
    gitscale_at(job, &env.cache, &borrowed(vars), &["check"])
}

/// A repository with `main` tagged `v1.0.0`, and a branch `feat/x` one commit
/// ahead of it.
pub fn repo_with_topic(env: &TestEnv, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let bare = env.create_bare_repo(name, "main", files);
    run_git_pub(&bare, &["tag", "v1.0.0", "main"]);
    run_git_pub(&bare, &["branch", "feat/x", "main"]);
    env.push_commit(&bare, "feat/x", "topic.txt", "topic work");
    bare
}

/// The rows of `ls --format json`, which must be a JSON array.
pub fn json_rows(stdout: &str) -> Vec<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("status printed no JSON ({}):\n{}", e, stdout));
    parsed.as_array().expect("a JSON array").clone()
}

/// The JSON row for `dir`, which must exist.
pub fn json_row(env: &TestEnv, dir: &str) -> serde_json::Value {
    let out = env.run(&["ls", "--format", "json"]);
    assert!(out.success, "{}", out.stderr);
    json_rows(&out.stdout)
        .into_iter()
        .find(|r| r["directory"] == dir)
        .unwrap_or_else(|| panic!("no JSON row for {}:\n{}", dir, out.stdout))
}

/// The table row for `dir` in `stdout`, colour stripped, which must exist.
pub fn table_row(stdout: &str, dir: &str) -> String {
    let plain = strip_ansi(stdout);
    plain
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(dir))
        .unwrap_or_else(|| panic!("no row for {}:\n{}", dir, plain))
        .to_string()
}

/// The cells of a table row: columns are three or more spaces apart, and no
/// cell holds three spaces running.
pub fn cells(row: &str) -> Vec<String> {
    regex::Regex::new(r"\s{3,}")
        .unwrap()
        .split(row.trim_end())
        .map(str::to_string)
        .collect()
}

/// The STATUS cell of `dir`'s row in a table without orphans: the seventh.
pub fn status_cell(stdout: &str, dir: &str) -> String {
    let row = table_row(stdout, dir);
    cells(&row)
        .get(6)
        .cloned()
        .unwrap_or_else(|| panic!("no STATUS cell in {:?}", row))
}

/// The ANSI colour code the icon of `dir`'s row is painted in, with the icon:
/// `("32", "✔")` for a green tick.
pub fn icon_and_colour(stdout: &str, dir: &str) -> (String, String) {
    let line = stdout
        .lines()
        .find(|l| strip_ansi(l).split_whitespace().nth(1) == Some(dir))
        .unwrap_or_else(|| panic!("no row for {}:\n{}", dir, stdout));
    let re = regex::Regex::new(r"^\x1b\[(?:1;)?([0-9]+)m([^\s\x1b]+)").unwrap();
    let caps = re
        .captures(line)
        .unwrap_or_else(|| panic!("no coloured icon in {:?}", line));
    (caps[2].to_string(), caps[1].to_string())
}

/// A workspace whose graph cannot be resolved offline: `b` overrides `d` at
/// v1.5.0, and `c`'s checkout, edited on disk, asks for v1.6.0 although `b` is
/// not above it — an override conflict, read from what is already here.
/// `junk.txt` is left untracked at the root.
pub fn conflicted_workspace(env: &TestEnv) {
    use super::resolution::{allow, repos, tagged};
    let d = tagged(env, "d", &[("v1.3.1", ""), ("v1.5.0", ""), ("v1.6.0", "")]);
    let b = tagged(
        env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.5.0\", override = true")]),
        )],
    );
    let c = tagged(
        env,
        "c",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.3.1\"")]),
        )],
    );
    env.write_config(&format!(
        "{}{}",
        allow(env),
        repos(&[
            ("imports/b", &b, ", revision = \"v1.0.0\""),
            ("imports/c", &c, ", revision = \"v1.0.0\""),
        ])
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    super::edit(
        &env.playground.join("imports/c/.gitscale.toml"),
        &repos(&[("libs/d", &d, ", revision = \"v1.6.0\"")]),
    );
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
}

/// A root config declaring `libs/mylib` from `bare` at `revision`.
pub fn mylib_config(bare: &Path, revision: &str) -> String {
    format!(
        "[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"{}\" }}\n",
        bare.display(),
        revision
    )
}

/// Create `branch` in the bare repository `bare` at its `main`.
pub fn run_git_branch(bare: &Path, branch: &str) {
    run_git_pub(bare, &["branch", branch, "main"]);
}

/// Replace the checkout at `dir` — whatever is there — with a symlink to a
/// clone of `bare` outside the workspace, as somebody pointing an entry at
/// their own checkout does. Returns that clone.
pub fn symlink_entry(env: &TestEnv, bare: &Path, dir: &str) -> PathBuf {
    let checkout = env.playground.join(dir);
    if checkout.symlink_metadata().is_ok() {
        std::fs::remove_dir_all(&checkout).unwrap();
    }
    std::fs::create_dir_all(checkout.parent().unwrap()).unwrap();
    let own = env
        .repos_remote
        .join(format!("own-{}", dir.replace('/', "-")));
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", bare.to_str().unwrap(), own.to_str().unwrap()],
    );
    std::os::unix::fs::symlink(&own, &checkout).unwrap();
    own
}
