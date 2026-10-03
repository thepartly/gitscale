//! Helpers for the ci auth tests.

use super::{CliOutput, TestEnv};
use gitscale::ci::CiAuth;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
/// A host that cannot resolve, so the network operations below fail fast while
/// still reporting the URL git actually tried — which is what is under test.
pub const CI_HOST: &str = "gitlab.invalid";

/// A job token distinctive enough to search every output and file for.
pub const TOKEN: &str = "j0b-t0ken-5ecret-7c1d";

pub fn gitlab_auth() -> CiAuth {
    CiAuth::from_map(&HashMap::from([
        ("CI_JOB_TOKEN", "unused-here"),
        ("CI_SERVER_URL", "https://gitlab.example.com"),
    ]))
    .expect("gitlab job env should be detected")
}

/// Ask git itself for the credentials it would use for `host`.
pub fn credential_fill(auth: &CiAuth, host: &str, token: &str) -> Output {
    Command::new("git")
        .args(auth.git_config_args())
        .args(["credential", "fill"])
        .env("CI_JOB_TOKEN", token)
        .env("GIT_TERMINAL_PROMPT", "0")
        // An inherited askpass (e.g. an editor's helper) would let git block
        // on a GUI prompt for an unscoped host instead of failing fast, which
        // is exactly what the "other host" case must exercise.
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            write!(
                child.stdin.as_mut().unwrap(),
                "protocol=https\nhost={}\n\n",
                host
            )?;
            child.wait_with_output()
        })
        .expect("failed to run git credential fill")
}

/// Run the gitscale binary with a GitLab job environment.
pub fn run_in_ci_job(env: &TestEnv, args: &[&str]) -> Output {
    run_in_ci_job_with(env, &[], args)
}

/// [`run_in_ci_job`] with extra environment on top.
pub fn run_in_ci_job_with(env: &TestEnv, vars: &[(&str, &str)], args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gitscale"));
    cmd.args(args)
        .args(["-C", env.playground.to_str().unwrap()])
        .env("CI", "true")
        .env("GITLAB_CI", "true")
        .env("CI_JOB_TOKEN", "job-token-value")
        .env("CI_SERVER_URL", format!("https://{}", CI_HOST));
    for (name, value) in vars {
        cmd.env(name, value);
    }
    cmd.output().expect("failed to run gitscale")
}

/// Every variable that tells gitscale it runs in a CI job, or which forge.
const CI_VARS: &[&str] = &[
    "CI",
    "GITLAB_CI",
    "CI_JOB_TOKEN",
    "CI_SERVER_URL",
    "CI_SERVER_HOST",
    "CI_SERVER_PORT",
    "CI_SERVER_PROTOCOL",
    "CI_REGISTRY",
    "GITHUB_ACTIONS",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "GITHUB_SERVER_URL",
    "GITSCALE_NO_CI_AUTH",
    "GIT_CLEAN_FLAGS",
    "GITSCALE_HOOK",
    "GITSCALE_HOOK_ALLOW",
];

/// Run the gitscale binary as a job would: exactly the CI variables in
/// `vars` and no others, `home` as HOME (so the git config it inherits is the
/// test's), the test's own cache, and `ssh` standing in for the ssh client.
pub fn job(env: &TestEnv, home: &Path, vars: &[(&str, &str)], args: &[&str]) -> CliOutput {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gitscale"));
    cmd.args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GITSCALE_CACHE_DIR", &env.cache)
        .env("GIT_SSH_COMMAND", fake_ssh(home).0)
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS");
    for name in CI_VARS {
        cmd.env_remove(name);
    }
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let out = cmd.output().expect("failed to run gitscale");
    CliOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        success: out.status.success(),
    }
}

/// A HOME for [`job`], under the test's remote directory. Its git config
/// carries a credential helper that writes every credential git approves to
/// `credentials`, the way a developer's or a runner's `credential.helper =
/// store` would.
pub fn job_home(env: &TestEnv) -> (PathBuf, PathBuf) {
    let home = env.repos_remote.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = home.join("credentials");
    std::fs::write(
        home.join(".gitconfig"),
        format!(
            "[credential]\n\thelper = store --file={}\n",
            store.display()
        ),
    )
    .unwrap();
    (home, store)
}

/// A stand-in for ssh, for `GIT_SSH_COMMAND`: it records each connection's
/// arguments in the returned log and fails as a refused key does.
pub fn fake_ssh(dir: &Path) -> (PathBuf, PathBuf) {
    let script = dir.join("fake-ssh");
    let log = dir.join("fake-ssh.log");
    if !script.exists() {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\necho 'git@fake: Permission denied (publickey).' >&2\nexit 255\n",
                log.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    (script, log)
}

/// What [`fake_ssh`] recorded: one line per connection.
pub fn ssh_calls(home: &Path) -> String {
    std::fs::read_to_string(fake_ssh(home).1).unwrap_or_default()
}

/// Every file under `dir` whose bytes contain `needle`.
pub fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                if let Ok(bytes) = std::fs::read(&path) {
                    if bytes.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                        found.push(path);
                    }
                }
            }
        }
    }
    found
}
