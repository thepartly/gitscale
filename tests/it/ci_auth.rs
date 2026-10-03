//! Authenticating to the CI server with the job token, and to nothing else.

use crate::support::ci_auth::*;
use crate::support::git_http::GitHttp;
use crate::support::TestEnv;

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// The point of the helper: git gets the token by reading the environment at
/// the moment it needs it, with nothing secret in the config or command line.
#[test]
fn normal_001_git_takes_the_token_from_the_environment() {
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

#[test]
fn normal_002_an_ssh_entry_on_the_ci_server_is_cloned_over_https() {
    let env = TestEnv::new("ci_auth_clone_rewrite");
    env.write_config(&format!(
        r#"[repos]
"libs/mylib" = {{ url = "git@{}:acme/mylib.git", revision = "main" }}
"#,
        CI_HOST
    ));

    let out = run_in_ci_job(&env, &["pull"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "a pull of an unresolvable host should fail"
    );
    assert!(
        stderr.contains(&format!("https://{}/acme/mylib.git", CI_HOST)),
        "expected the HTTPS rewrite in git's error, got: {stderr}"
    );
    assert!(
        !stderr.contains("job-token-value"),
        "token leaked into output: {stderr}"
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("job-token-value"),
        "token leaked into stdout"
    );
    // git's HTTP transport made the attempt: the failure is curl's, not ssh's.
    assert!(
        stderr.contains(&format!(
            "unable to access 'https://{}/acme/mylib.git/'",
            CI_HOST
        )),
        "{stderr}"
    );
    assert!(
        !stderr.contains("Could not read from remote repository"),
        "{stderr}"
    );
}

/// End to end over real HTTP: in a GitLab job an entry declared over SSH on
/// the CI server's own host is fetched from that server over HTTP(S), as
/// `gitlab-ci-token` with the job token — and its `origin` is the plain URL.
#[test]
fn normal_005_a_gitlab_job_fetches_its_servers_repositories_with_the_job_token() {
    let env = TestEnv::new("ci_auth_http_gitlab");
    env.create_bare_repo("lib", "main", &[("f.txt", "from the ci server")]);
    let server = GitHttp::start(&env.repos_remote);
    let (home, _) = job_home(&env);
    env.write_config(
        "[repos]\n\"libs/lib\" = { url = \"git@localhost:lib.git\", revision = \"main\" }\n",
    );

    let ci_server = format!("http://localhost:{}", server.port);
    let out = job(
        &env,
        &home,
        &[
            ("CI", "true"),
            ("GITLAB_CI", "true"),
            ("CI_JOB_TOKEN", TOKEN),
            ("CI_SERVER_URL", &ci_server),
        ],
        &["pull", "-C", env.playground.to_str().unwrap()],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        std::fs::read_to_string(env.playground.join("libs/lib/f.txt")).unwrap(),
        "from the ci server"
    );
    let authed: Vec<_> = server
        .requests()
        .into_iter()
        .filter_map(|r| r.credentials)
        .collect();
    assert!(
        !authed.is_empty(),
        "no authenticated request reached the server"
    );
    assert!(
        authed
            .iter()
            .all(|c| c == &format!("gitlab-ci-token:{}", TOKEN)),
        "{:?}",
        authed
    );
    assert_eq!(
        crate::support::git_stdout(
            &env.playground.join("libs/lib"),
            &["remote", "get-url", "origin"]
        ),
        server.url("localhost", "lib.git")
    );
    assert!(
        ssh_calls(&home).is_empty(),
        "ssh was used: {}",
        ssh_calls(&home)
    );
}

/// The same on GitHub Actions: the runner marker plus `GITHUB_TOKEN` — or
/// `GH_TOKEN` when that is the one exported — authenticate as
/// `x-access-token` to `GITHUB_SERVER_URL`.
#[test]
fn normal_006_a_github_actions_job_fetches_with_its_token() {
    for token_var in ["GITHUB_TOKEN", "GH_TOKEN"] {
        let env = TestEnv::new(&format!("ci_auth_http_github_{}", token_var.to_lowercase()));
        env.create_bare_repo("lib", "main", &[("f.txt", "from github")]);
        let server = GitHttp::start(&env.repos_remote);
        let (home, _) = job_home(&env);
        env.write_config(
            "[repos]\n\"libs/lib\" = { url = \"git@localhost:lib.git\", revision = \"main\" }\n",
        );

        let ci_server = format!("http://localhost:{}", server.port);
        let out = job(
            &env,
            &home,
            &[
                ("CI", "true"),
                ("GITHUB_ACTIONS", "true"),
                (token_var, TOKEN),
                ("GITHUB_SERVER_URL", &ci_server),
            ],
            &["pull", "-C", env.playground.to_str().unwrap()],
        );
        assert!(out.success, "{}: {}{}", token_var, out.stdout, out.stderr);
        assert!(
            env.playground.join("libs/lib/f.txt").is_file(),
            "{}",
            token_var
        );
        let authed: Vec<_> = server
            .requests()
            .into_iter()
            .filter_map(|r| r.credentials)
            .collect();
        assert!(!authed.is_empty(), "{}", token_var);
        assert!(
            authed
                .iter()
                .all(|c| c == &format!("x-access-token:{}", TOKEN)),
            "{}: {:?}",
            token_var,
            authed
        );
    }
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// The host check is the whole safety property: a dependency hosted elsewhere
/// must never be offered the job token.
#[test]
fn edge_003_another_host_is_never_offered_the_token() {
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

#[test]
fn edge_004_an_entry_on_another_host_keeps_its_ssh_url() {
    let env = TestEnv::new("ci_auth_other_host");
    env.write_config(
        r#"[repos]
"libs/mylib" = { url = "git@github.invalid:acme/mylib.git", revision = "main" }
"#,
    );

    let out = run_in_ci_job(&env, &["pull"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        !stderr.contains("https://github.invalid"),
        "a third-party host must be fetched as configured, got: {stderr}"
    );
    // What was fetched instead: the entry's own SSH URL, over ssh.
    assert!(
        stderr.contains("cannot fetch git@github.invalid:acme/mylib.git"),
        "{stderr}"
    );
    assert!(
        stderr.contains("fatal: Could not read from remote repository"),
        "the failure should be ssh's: {stderr}"
    );
}

/// The safety property over real HTTP: in a job on `localhost`, an entry on
/// `127.0.0.1` — the same server, another host to git — is asked for
/// credentials and never offered the job token, while the CI server's own
/// entry in the same config is.
#[test]
fn edge_007_the_job_token_never_reaches_another_host() {
    let env = TestEnv::new("ci_auth_http_other_host");
    env.create_bare_repo("lib", "main", &[("f.txt", "lib")]);
    env.create_bare_repo("other", "main", &[("f.txt", "other")]);
    let server = GitHttp::start(&env.repos_remote);
    let (home, _) = job_home(&env);
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"git@localhost:lib.git\", revision = \"main\" }}\n\"libs/other\" = {{ url = \"{}\", revision = \"main\" }}\n",
        server.url("127.0.0.1", "other.git")
    ));

    let ci_server = format!("http://localhost:{}", server.port);
    let out = job(
        &env,
        &home,
        &[
            ("CI", "true"),
            ("GITLAB_CI", "true"),
            ("CI_JOB_TOKEN", TOKEN),
            ("CI_SERVER_URL", &ci_server),
        ],
        &["pull", "-C", env.playground.to_str().unwrap()],
    );
    assert!(
        !out.success,
        "the other host demands credentials it never gets"
    );
    let requests = server.requests();
    assert!(
        requests.iter().any(|r| r.host == "127.0.0.1"),
        "the other host was never tried: {:?}",
        requests
    );
    let leaked: Vec<_> = requests
        .iter()
        .filter(|r| r.host != "localhost" && r.credentials.is_some())
        .collect();
    assert!(
        leaked.is_empty(),
        "credentials sent elsewhere: {:?}",
        leaked
    );
    assert!(!format!("{}{}", out.stdout, out.stderr).contains(TOKEN));
}

/// The token is read from the environment when git asks, and lands nowhere
/// on disk: not in a checkout's or the cache's git config, not in the
/// breadcrumb a failed hook pull leaves, and not in the file of a
/// `credential.helper = store` inherited from the user's git config — which
/// git would hand every credential it approves, were the helper list not
/// reset for the CI host.
#[test]
fn edge_008_the_job_token_is_written_nowhere_on_disk() {
    let env = TestEnv::new("ci_auth_http_no_trace");
    env.create_bare_repo("lib", "main", &[("f.txt", "lib")]);
    let server = GitHttp::start(&env.repos_remote);
    let (home, store) = job_home(&env);
    let ci_server = format!("http://localhost:{}", server.port);
    let vars = [
        ("CI", "true"),
        ("GITLAB_CI", "true"),
        ("CI_JOB_TOKEN", TOKEN),
        ("CI_SERVER_URL", ci_server.as_str()),
        ("GIT_CLEAN_FLAGS", "none"),
    ];
    let root = env.playground.to_str().unwrap();

    // A job that works...
    env.write_config(
        "[repos]\n\"libs/lib\" = { url = \"git@localhost:lib.git\", revision = \"main\" }\n",
    );
    let first = job(&env, &home, &vars, &["pull", "-C", root]);
    assert!(first.success, "{}{}", first.stdout, first.stderr);
    assert!(
        server.requests().iter().any(|r| r.credentials.is_some()),
        "the token was never used, so nothing could leak"
    );

    // ...and a hook pull that fails, leaving a breadcrumb.
    env.write_config(&format!(
        "[repos]\n\"libs/lib\" = {{ url = \"git@localhost:lib.git\", revision = \"main\" }}\n\"libs/other\" = {{ url = \"{}\", revision = \"main\" }}\n",
        server.url("127.0.0.1", "other.git")
    ));
    let mut hook_vars = vars.to_vec();
    hook_vars.extend([
        ("GITSCALE_HOOK", "post-checkout"),
        ("GITSCALE_HOOK_ALLOW", "*"),
    ]);
    let second = job(
        &env,
        &home,
        &hook_vars,
        &["hook", "run", "post-checkout", "-C", root],
    );
    assert!(!second.success, "CI fails a failed hook pull");
    assert!(env.playground.join(".git/gitscale-pull-failed").is_file());

    for output in [&first, &second] {
        assert!(!format!("{}{}", output.stdout, output.stderr).contains(TOKEN));
    }
    let mut leaks = Vec::new();
    for dir in [&env.playground, &env.cache, &home] {
        leaks.extend(files_containing(dir, TOKEN));
    }
    assert!(leaks.is_empty(), "the token is on disk in {:?}", leaks);
    assert!(
        !std::fs::read_to_string(&store)
            .unwrap_or_default()
            .contains(TOKEN),
        "the inherited credential helper stored the token"
    );
}

/// Where the CI server is, and so which entries go to it: a port in
/// `CI_SERVER_URL` is kept; without it the host, protocol and port variables
/// are used; and a value that is not a plain http(s) URL turns the rewrite
/// off rather than ending up in a git config key.
#[test]
fn edge_009_the_ci_server_url_decides_where_entries_on_its_host_are_fetched() {
    let env = TestEnv::new("ci_auth_server_url_forms");
    let (home, _) = job_home(&env);
    env.write_config(&format!(
        "[repos]\n\"libs/x\" = {{ url = \"git@{}:acme/x.git\", revision = \"main\" }}\n",
        CI_HOST
    ));
    let root = env.playground.to_str().unwrap();
    type Vars<'a> = &'a [(&'a str, &'a str)];
    let cases: &[(Vars, Option<&str>)] = &[
        (
            &[("CI_SERVER_URL", "https://gitlab.invalid:8443/")],
            Some("https://gitlab.invalid:8443/acme/x.git"),
        ),
        (
            &[
                ("CI_SERVER_HOST", "gitlab.invalid"),
                ("CI_SERVER_PROTOCOL", "http"),
                ("CI_SERVER_PORT", "8080"),
            ],
            Some("http://gitlab.invalid:8080/acme/x.git"),
        ),
        (&[("CI_SERVER_URL", "ftp://gitlab.invalid")], None),
        (&[("CI_SERVER_URL", "gitlab.invalid")], None),
    ];
    for (server, rewritten) in cases {
        let _ = std::fs::remove_file(fake_ssh(&home).1);
        let mut vars = vec![
            ("CI", "true"),
            ("GITLAB_CI", "true"),
            ("CI_JOB_TOKEN", TOKEN),
        ];
        vars.extend_from_slice(server);
        let out = job(&env, &home, &vars, &["pull", "-C", root]);
        assert!(!out.success, "{:?}", server);
        match rewritten {
            Some(url) => {
                assert!(out.stderr.contains(url), "{:?}: {}", server, out.stderr);
                assert!(ssh_calls(&home).is_empty(), "{:?}", server);
            }
            None => {
                assert!(
                    out.stderr
                        .contains(&format!("cannot fetch git@{}:acme/x.git", CI_HOST)),
                    "{:?}: {}",
                    server,
                    out.stderr
                );
                assert!(!out.stderr.contains("://gitlab.invalid"), "{:?}", server);
                assert!(ssh_calls(&home).contains(CI_HOST), "{:?}", server);
            }
        }
    }
}

/// GitLab can be served under a path (`https://host/gitlab`), and
/// `CI_SERVER_URL` then carries it. Repositories live below that path, so the
/// HTTPS URL an SSH entry is rewritten to must keep it.
#[test]
#[ignore = "bug: the path in CI_SERVER_URL is dropped, so a GitLab under a relative URL root gets 404s"]
fn edge_010_a_ci_server_under_a_path_keeps_the_path() {
    let env = TestEnv::new("ci_auth_server_url_path");
    let (home, _) = job_home(&env);
    env.write_config(&format!(
        "[repos]\n\"libs/x\" = {{ url = \"git@{}:acme/x.git\", revision = \"main\" }}\n",
        CI_HOST
    ));
    let out = job(
        &env,
        &home,
        &[
            ("CI", "true"),
            ("CI_JOB_TOKEN", TOKEN),
            ("CI_SERVER_URL", "https://gitlab.invalid/gitlab"),
        ],
        &["pull", "-C", env.playground.to_str().unwrap()],
    );
    assert!(
        out.stderr
            .contains("https://gitlab.invalid/gitlab/acme/x.git"),
        "{}",
        out.stderr
    );
}

/// `GITHUB_TOKEN` and `GH_TOKEN` are often exported on developer machines;
/// without the runner's own `GITHUB_ACTIONS` marker they change nothing, and
/// an SSH entry stays SSH.
#[test]
fn edge_011_github_tokens_without_the_runner_marker_change_nothing() {
    let env = TestEnv::new("ci_auth_github_no_marker");
    let (home, _) = job_home(&env);
    env.write_config(
        "[repos]\n\"libs/x\" = { url = \"git@github.invalid:acme/x.git\", revision = \"main\" }\n",
    );
    let out = job(
        &env,
        &home,
        &[
            ("GITHUB_TOKEN", TOKEN),
            ("GH_TOKEN", TOKEN),
            ("GITHUB_SERVER_URL", "https://github.invalid"),
        ],
        &["pull", "-C", env.playground.to_str().unwrap()],
    );
    assert!(!out.success);
    assert!(
        out.stderr
            .contains("cannot fetch git@github.invalid:acme/x.git"),
        "{}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("https://github.invalid"),
        "{}",
        out.stderr
    );
    assert!(
        ssh_calls(&home).contains("github.invalid"),
        "{}",
        ssh_calls(&home)
    );
}

/// `GITSCALE_NO_CI_AUTH` turns the whole thing off: in a GitLab job an SSH
/// entry on the CI server is fetched exactly as configured.
#[test]
fn edge_012_gitscale_no_ci_auth_turns_the_rewrite_off() {
    let env = TestEnv::new("ci_auth_opt_out");
    let (home, _) = job_home(&env);
    env.write_config(&format!(
        "[repos]\n\"libs/x\" = {{ url = \"git@{}:acme/x.git\", revision = \"main\" }}\n",
        CI_HOST
    ));
    let out = job(
        &env,
        &home,
        &[
            ("CI", "true"),
            ("GITLAB_CI", "true"),
            ("CI_JOB_TOKEN", TOKEN),
            ("CI_SERVER_URL", "https://gitlab.invalid"),
            ("GITSCALE_NO_CI_AUTH", "1"),
        ],
        &["pull", "-C", env.playground.to_str().unwrap()],
    );
    assert!(!out.success);
    assert!(
        !out.stderr.contains("https://gitlab.invalid"),
        "{}",
        out.stderr
    );
    assert!(ssh_calls(&home).contains(CI_HOST), "{}", ssh_calls(&home));
}

/// A job that looks like both forges — a GitLab job token next to the GitHub
/// runner marker — is a GitLab job, as documented: entries on the GitLab
/// server are rewritten, entries on the GitHub one are not.
#[test]
fn edge_013_gitlab_wins_when_both_forges_are_detected() {
    let env = TestEnv::new("ci_auth_both_forges");
    let (home, _) = job_home(&env);
    let vars = [
        ("CI", "true"),
        ("CI_JOB_TOKEN", TOKEN),
        ("CI_SERVER_URL", "https://gitlab.invalid"),
        ("GITHUB_ACTIONS", "true"),
        ("GITHUB_TOKEN", TOKEN),
        ("GITHUB_SERVER_URL", "https://github.invalid"),
    ];
    let root = env.playground.to_str().unwrap();

    env.write_config(
        "[repos]\n\"libs/x\" = { url = \"git@gitlab.invalid:acme/x.git\", revision = \"main\" }\n",
    );
    let out = job(&env, &home, &vars, &["pull", "-C", root]);
    assert!(
        out.stderr.contains("https://gitlab.invalid/acme/x.git"),
        "{}",
        out.stderr
    );

    env.write_config(
        "[repos]\n\"libs/y\" = { url = \"git@github.invalid:acme/y.git\", revision = \"main\" }\n",
    );
    let out = job(&env, &home, &vars, &["pull", "-C", root]);
    assert!(
        out.stderr
            .contains("cannot fetch git@github.invalid:acme/y.git"),
        "{}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("https://github.invalid"),
        "{}",
        out.stderr
    );
    assert!(
        ssh_calls(&home).contains("github.invalid"),
        "{}",
        ssh_calls(&home)
    );
}

/// A checkout the runner kept from an earlier job may still have an SSH
/// `origin` — restored from a cache, or set by hand. Before the next fetch it
/// is repointed at the CI server's URL, and the pin moves.
#[test]
fn edge_014_an_ssh_origin_left_in_a_checkout_is_repointed_before_fetching() {
    let env = TestEnv::new("ci_auth_repoint_origin");
    crate::support::ci_checkout::tagged_remote(&env);
    let server = GitHttp::start(&env.repos_remote);
    let (home, _) = job_home(&env);
    let ci_server = format!("http://localhost:{}", server.port);
    let vars = [
        ("CI", "true"),
        ("GITLAB_CI", "true"),
        ("CI_JOB_TOKEN", TOKEN),
        ("CI_SERVER_URL", ci_server.as_str()),
    ];
    let root = env.playground.to_str().unwrap();
    let pin = |rev: &str| {
        env.write_config(&format!(
            "[repos]\n\"libs/lib\" = {{ url = \"git@localhost:lib.git\", revision = \"{}\" }}\n",
            rev
        ))
    };
    let lib = env.playground.join("libs/lib");

    pin("v1");
    let out = job(&env, &home, &vars, &["pull", "-C", root]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    crate::support::run_git_pub(
        &lib,
        &["remote", "set-url", "origin", "git@localhost:lib.git"],
    );

    pin("v2");
    let out = job(&env, &home, &vars, &["pull", "-C", root]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        crate::support::git_stdout(&lib, &["log", "-1", "--format=%s"]),
        "c2"
    );
    assert_eq!(
        crate::support::git_stdout(&lib, &["remote", "get-url", "origin"]),
        server.url("localhost", "lib.git")
    );
    assert!(
        ssh_calls(&home).is_empty(),
        "ssh was used: {}",
        ssh_calls(&home)
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// A 403 from the CI server means the token was accepted but may not read
/// that project. The failure says what to change, for the forge in question:
/// GitLab's job token allowlist, or a better token on GitHub.
#[test]
fn error_015_a_403_from_the_ci_server_says_how_to_get_access() {
    for (forge, hint) in [
        ("gitlab", "Job token permissions"),
        ("github", "GitHub Actions token (GITHUB_TOKEN) was rejected"),
    ] {
        let env = TestEnv::new(&format!("ci_auth_http_403_{}", forge));
        let server = GitHttp::start(&env.repos_remote);
        let (home, _) = job_home(&env);
        env.write_config("[repos]\n\"libs/x\" = { url = \"git@localhost:forbidden.git\", revision = \"main\" }\n");
        let ci_server = format!("http://localhost:{}", server.port);
        let vars: Vec<(&str, &str)> = match forge {
            "gitlab" => vec![
                ("CI", "true"),
                ("CI_JOB_TOKEN", TOKEN),
                ("CI_SERVER_URL", &ci_server),
            ],
            _ => vec![
                ("CI", "true"),
                ("GITHUB_ACTIONS", "true"),
                ("GITHUB_TOKEN", TOKEN),
                ("GITHUB_SERVER_URL", &ci_server),
            ],
        };
        let out = job(
            &env,
            &home,
            &vars,
            &["pull", "-C", env.playground.to_str().unwrap()],
        );
        assert!(!out.success, "{}", forge);
        assert!(out.stderr.contains("403"), "{}: {}", forge, out.stderr);
        assert!(out.stderr.contains(hint), "{}: {}", forge, out.stderr);
    }
}

/// Only a 403 from the server is a refused token. A failure that merely has
/// `403` somewhere in its text — here in a repository's name — gets git's own
/// message, not advice about job token permissions.
#[test]
#[ignore = "bug: any \"403\" in git's stderr, even in a path, is taken for a refused job token"]
fn error_016_a_403_in_a_repository_name_is_not_a_refused_token() {
    let env = TestEnv::new("ci_auth_403_in_name");
    let (home, _) = job_home(&env);
    let missing = env.repos_remote.join("svc-403.git");
    env.write_config(&format!(
        "[repos]\n\"libs/x\" = {{ url = \"{}\", revision = \"main\" }}\n",
        missing.display()
    ));
    let out = job(
        &env,
        &home,
        &[
            ("CI", "true"),
            ("CI_JOB_TOKEN", TOKEN),
            ("CI_SERVER_URL", "https://gitlab.invalid"),
        ],
        &["pull", "-C", env.playground.to_str().unwrap()],
    );
    assert!(!out.success);
    assert!(out.stderr.contains("svc-403.git"), "{}", out.stderr);
    assert!(
        !out.stderr.contains("Job token permissions"),
        "{}",
        out.stderr
    );
}
