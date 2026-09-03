// Each integration test binary compiles `helpers` separately; this one uses
// only part of it.
#[allow(dead_code)]
mod helpers;

use std::collections::HashMap;
use std::process::{Command, Output};

use gitscale::ci::CiAuth;
use helpers::TestEnv;

/// A host that cannot resolve, so the network operations below fail fast while
/// still reporting the URL git actually tried — which is what is under test.
const CI_HOST: &str = "gitlab.invalid";

fn gitlab_auth() -> CiAuth {
    CiAuth::from_map(&HashMap::from([
        ("CI_JOB_TOKEN", "unused-here"),
        ("CI_SERVER_URL", "https://gitlab.example.com"),
    ]))
    .expect("gitlab job env should be detected")
}

/// Ask git itself for the credentials it would use for `host`.
fn credential_fill(auth: &CiAuth, host: &str, token: &str) -> Output {
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

/// The point of the helper: git gets the token by reading the environment at
/// the moment it needs it, with nothing secret in the config or command line.
#[test]
fn git_takes_the_token_from_the_environment() {
    let auth = gitlab_auth();
    let out = credential_fill(&auth, "gitlab.example.com", "job-token-value");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("username=gitlab-ci-token"), "got: {stdout}");
    assert!(stdout.contains("password=job-token-value"), "got: {stdout}");
}

/// The host check is the whole safety property: a dependency hosted elsewhere
/// must never be offered the job token.
#[test]
fn another_host_is_never_offered_the_token() {
    let auth = gitlab_auth();
    let out = credential_fill(&auth, "gitlab.other.example", "job-token-value");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !combined.contains("job-token-value"),
        "token offered to a third-party host: {combined}"
    );
}

/// Run the gitscale binary with a GitLab job environment.
fn run_in_ci_job(env: &TestEnv, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(args)
        .args(["-C", env.playground.to_str().unwrap()])
        .env("CI", "true")
        .env("GITLAB_CI", "true")
        .env("CI_JOB_TOKEN", "job-token-value")
        .env("CI_SERVER_URL", format!("https://{}", CI_HOST))
        .output()
        .expect("failed to run gitscale")
}

#[test]
fn ssh_entry_on_the_ci_server_is_cloned_over_https() {
    let env = TestEnv::new("ci_auth_clone_rewrite");
    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "git@{}:acme/mylib.git", revision = "main" }}
"#,
        CI_HOST
    ));

    let out = run_in_ci_job(&env, &["clone"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "clone of an unresolvable host should fail"
    );
    assert!(
        stderr.contains(&format!("https://{}/acme/mylib.git", CI_HOST)),
        "expected the HTTPS rewrite in git's error, got: {stderr}"
    );
    assert!(
        !stderr.contains("job-token-value"),
        "token leaked into output: {stderr}"
    );
}

#[test]
fn entry_on_another_host_keeps_its_ssh_url() {
    let env = TestEnv::new("ci_auth_other_host");
    env.write_config(
        r#"[repos]
"libs/mylib" = { url = "git@github.invalid:acme/mylib.git", revision = "main" }
"#,
    );

    let out = run_in_ci_job(&env, &["clone"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        !stderr.contains("https://github.invalid"),
        "a third-party host must be fetched as configured, got: {stderr}"
    );
}

/// A workspace that already has an SSH `origin` — restored from cache, or
/// cloned before the job — is repointed before the network operation runs.
#[test]
fn existing_ssh_remote_is_repointed_under_ci() {
    let env = TestEnv::new("ci_auth_repoint");
    let bare = env.create_bare_repo("mylib", "main", &[("README.md", "# mylib\n")]);
    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    // Clone from the local bare repo, then point origin at the CI server the
    // way a runner's own SSH-based checkout would have.
    let clone = env.run(&["clone"]);
    assert!(clone.success, "stderr: {}", clone.stderr);
    let dest = env.playground.join("libs/mylib");
    helpers::run_git_pub(
        &dest,
        &[
            "remote",
            "set-url",
            "origin",
            &format!("git@{}:acme/mylib.git", CI_HOST),
        ],
    );

    // The config still names the SSH URL, so gitscale must rewrite it.
    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "git@{}:acme/mylib.git", revision = "main" }}
"#,
        CI_HOST
    ));
    let out = run_in_ci_job(&env, &["fetch"]);
    assert!(!out.status.success());

    let origin = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(&dest)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&origin.stdout).trim(),
        format!("https://{}/acme/mylib.git", CI_HOST)
    );
}
