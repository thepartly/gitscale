//! Helpers for the registry tests: stored logins, CI job environments, and
//! a second registry to play "another host".

use super::registry::{Auth, FakeRegistry};
use super::TestEnv;
use std::path::{Path, PathBuf};

/// `user:password`, as `docker login` stores it in `auths`.
pub fn encoded(user: &str, password: &str) -> String {
    base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        format!("{}:{}", user, password),
    )
}

/// A `DOCKER_CONFIG` directory whose `config.json` is `json`.
pub fn docker_config(env: &TestEnv, name: &str, json: &str) -> PathBuf {
    let dir = env.repos_remote.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.json"), json).unwrap();
    dir
}

/// A `DOCKER_CONFIG` holding a login for `registry`.
pub fn docker_login(env: &TestEnv, registry: &str, user: &str, password: &str) -> PathBuf {
    docker_config(
        env,
        "docker-config",
        &format!(
            r#"{{"auths": {{"{}": {{"auth": "{}"}}}}}}"#,
            registry,
            encoded(user, password)
        ),
    )
}

pub fn auth(user: &str, password: &str) -> Auth {
    Auth {
        user: user.into(),
        password: password.into(),
        realm_host: "127.0.0.1".into(),
    }
}

/// A second registry, on 127.0.0.2: a different host to gitscale, which
/// counts only localhost, 127.0.0.1 and [::1] as this machine.
pub fn elsewhere() -> FakeRegistry {
    FakeRegistry::start_on("127.0.0.2")
}

/// Keep requests to 127.0.0.2 off any proxy the environment names.
pub const NO_PROXY: [(&str, &str); 2] = [("NO_PROXY", "127.0.0.2"), ("no_proxy", "127.0.0.2")];

/// A GitLab job's variables for a CI server at `server` (`host:port`) that
/// owns the registry `registry`.
pub fn gitlab_vars(server: &str, registry: &str) -> Vec<(&'static str, String)> {
    vec![
        ("CI_JOB_TOKEN", "job-token-value".to_string()),
        ("CI_SERVER_URL", format!("http://{}", server)),
        ("CI_REGISTRY", registry.to_string()),
    ]
}

/// `(name, value)` pairs as `run_with_env` takes them.
pub fn borrowed<'a>(vars: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    vars.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

/// A path string.
pub fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}
