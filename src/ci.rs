//! Credentials that a CI runner hands its own jobs.
//!
//! A job on GitLab or GitHub gets a short-lived token for the forge it is
//! running on, but no SSH key — so an entry declared as `git@host:group/repo`
//! in `.gitscale.toml` cannot be cloned there. Everything needed to fix that
//! is already in the job environment, so gitscale derives it rather than
//! asking the pipeline to rewrite URLs with `git config insteadOf`.
//!
//! Two properties matter, and both are the reason this is not just an
//! `insteadOf` line generated at runtime:
//!
//! * The token never lands on disk or in argv. Only the *name* of the
//!   environment variable appears in the git invocation; a credential helper
//!   reads the variable when git asks for the password. A `git config` write
//!   would persist the token into `.git/config` in a working tree that CI
//!   then caches, archives as artefacts, and mounts into containers.
//! * The credential is scoped to the CI server's own host. A dependency
//!   hosted anywhere else is fetched exactly as configured, and never sees
//!   the token.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::urls::{extract_hostname, extract_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forge {
    GitLab,
    GitHub,
}

/// A CI server that will authenticate us for repos it hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiAuth {
    pub forge: Forge,
    /// `scheme://host[:port]` of the CI server, as git credential config
    /// wants it: no trailing slash, no path.
    pub base_url: String,
    /// Hostname of the CI server, lowercased and without a port. Ports are
    /// left out of the comparison on purpose: an SSH remote carries the SSH
    /// port, which says nothing about the HTTPS endpoint.
    pub host: String,
    /// The fixed username the forge expects alongside the token.
    pub username: String,
    /// Name of the environment variable holding the token — never its value.
    pub token_env: String,
}

/// The CI credentials for this process, detected once.
pub fn active() -> Option<&'static CiAuth> {
    static ACTIVE: OnceLock<Option<CiAuth>> = OnceLock::new();
    ACTIVE.get_or_init(detect).as_ref()
}

fn detect() -> Option<CiAuth> {
    CiAuth::from_vars(&|name| std::env::var(name).ok())
}

/// The URL to actually talk to for `repo_url`: an HTTPS URL on the CI server
/// when CI credentials cover it, otherwise the configured URL unchanged.
pub fn remote_url(repo_url: &str) -> String {
    active()
        .and_then(|auth| auth.remote_url(repo_url))
        .unwrap_or_else(|| repo_url.to_string())
}

impl CiAuth {
    /// Detect CI credentials from a variable lookup. Takes the lookup as an
    /// argument so tests need not mutate the process environment.
    pub fn from_vars(get: &dyn Fn(&str) -> Option<String>) -> Option<CiAuth> {
        let var = |name: &str| get(name).filter(|v| !v.is_empty());
        if var("GITSCALE_NO_CI_AUTH").is_some() {
            return None;
        }
        gitlab(&var).or_else(|| github(&var))
    }

    /// Convenience wrapper over [`CiAuth::from_vars`] for a plain map.
    pub fn from_map(vars: &HashMap<&str, &str>) -> Option<CiAuth> {
        CiAuth::from_vars(&|name| vars.get(name).map(|v| v.to_string()))
    }

    /// True when this CI server hosts `repo_url`.
    pub fn covers(&self, repo_url: &str) -> bool {
        extract_hostname(repo_url)
            .map(|h| h.to_lowercase() == self.host)
            .unwrap_or(false)
    }

    /// The HTTPS URL to fetch `repo_url` over, or `None` when it is not on
    /// this CI server. An entry already spelled with the CI server's scheme
    /// and authority is returned as-is rather than rebuilt.
    pub fn remote_url(&self, repo_url: &str) -> Option<String> {
        if !self.covers(repo_url) {
            return None;
        }
        if let Some(rest) = repo_url.strip_prefix(&format!("{}/", self.base_url)) {
            return Some(format!("{}/{}", self.base_url, rest));
        }
        let path = extract_path(repo_url).ok()?;
        Some(format!("{}/{}", self.base_url, path))
    }

    /// `git -c` arguments that let git authenticate to this CI server.
    ///
    /// The first, empty, helper resets any helper inherited from the user or
    /// system config; the second reads the token from the environment when
    /// git asks for it. Both are scoped to the CI server's URL, so no other
    /// host is ever offered the token, and neither carries the token itself.
    pub fn git_config_args(&self) -> Vec<String> {
        let key = format!("credential.{}", self.base_url);
        vec![
            "-c".to_string(),
            format!("{}.helper=", key),
            "-c".to_string(),
            format!("{}.helper={}", key, self.helper_script()),
            "-c".to_string(),
            format!("{}.username={}", key, self.username),
        ]
    }

    fn helper_script(&self) -> String {
        format!(
            "!f() {{ test \"$1\" = get && printf 'username={}\\npassword=%s\\n' \"${}\"; }}; f",
            self.username, self.token_env
        )
    }

    /// What to tell the user when the forge answers 403: the token was
    /// accepted as a token but is not allowed to read that project.
    pub fn forbidden_hint(&self) -> String {
        match self.forge {
            Forge::GitLab => format!(
                "the GitLab job token ({}) was rejected. Add this project to the \
                 target project's Settings -> CI/CD -> 'Job token permissions' allowlist, \
                 or give the job a token with read access.",
                self.token_env
            ),
            Forge::GitHub => format!(
                "the GitHub Actions token ({}) was rejected. It only covers the \
                 workflow's own repository — for another repository use a PAT or a \
                 GitHub App token with contents:read.",
                self.token_env
            ),
        }
    }
}

fn gitlab(var: &dyn Fn(&str) -> Option<String>) -> Option<CiAuth> {
    // CI_JOB_TOKEN exists only inside a job, so it doubles as the detector.
    // GITLAB_CI is deliberately not required: a job that runs gitscale in a
    // nested container may forward the token without the rest of the CI_* set.
    let token_env = "CI_JOB_TOKEN";
    var(token_env)?;
    let server = var("CI_SERVER_URL").or_else(|| {
        // Older runners, and jobs that only forward part of the CI_* set.
        let host = var("CI_SERVER_HOST")?;
        let scheme = var("CI_SERVER_PROTOCOL").unwrap_or_else(|| "https".to_string());
        match var("CI_SERVER_PORT") {
            Some(port) => Some(format!("{}://{}:{}", scheme, host, port)),
            None => Some(format!("{}://{}", scheme, host)),
        }
    })?;
    let (base_url, host) = normalize_server(&server)?;
    Some(CiAuth {
        forge: Forge::GitLab,
        base_url,
        host,
        username: "gitlab-ci-token".to_string(),
        token_env: token_env.to_string(),
    })
}

fn github(var: &dyn Fn(&str) -> Option<String>) -> Option<CiAuth> {
    // GITHUB_TOKEN and GH_TOKEN are commonly exported on developer machines,
    // so unlike GitLab this needs the runner's own marker to avoid quietly
    // rewriting SSH remotes during ordinary local use.
    var("GITHUB_ACTIONS")?;
    let token_env = ["GITHUB_TOKEN", "GH_TOKEN"]
        .into_iter()
        .find(|name| var(name).is_some())?;
    let server = var("GITHUB_SERVER_URL").unwrap_or_else(|| "https://github.com".to_string());
    let (base_url, host) = normalize_server(&server)?;
    Some(CiAuth {
        forge: Forge::GitHub,
        base_url,
        host,
        username: "x-access-token".to_string(),
        token_env: token_env.to_string(),
    })
}

/// Reduce a CI server URL to the `scheme://host[:port]` form git credential
/// config keys use, plus the bare hostname for matching. Rejects anything
/// that is not plain http(s), so a stray value cannot end up in a config key.
fn normalize_server(server: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(server.trim_end_matches('/')).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_lowercase();
    let base_url = match parsed.port() {
        Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
        None => format!("{}://{}", parsed.scheme(), host),
    };
    Some((base_url, host))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A distinctive token value, so a test can prove it never reaches argv.
    const TOKEN: &str = "s3cr3t-job-token";

    fn gitlab_auth() -> CiAuth {
        CiAuth::from_map(&HashMap::from([
            ("GITLAB_CI", "true"),
            ("CI_JOB_TOKEN", TOKEN),
            ("CI_SERVER_URL", "https://gitlab.example.com"),
        ]))
        .expect("gitlab job env should be detected")
    }

    #[test]
    fn detects_gitlab_job() {
        let auth = gitlab_auth();
        assert_eq!(auth.forge, Forge::GitLab);
        assert_eq!(auth.base_url, "https://gitlab.example.com");
        assert_eq!(auth.username, "gitlab-ci-token");
        assert_eq!(auth.token_env, "CI_JOB_TOKEN");
    }

    #[test]
    fn keeps_the_port_the_ci_server_url_carries() {
        let auth = CiAuth::from_map(&HashMap::from([
            ("CI_JOB_TOKEN", TOKEN),
            ("CI_SERVER_URL", "https://gitlab.example.com:8443/"),
        ]))
        .unwrap();
        assert_eq!(auth.base_url, "https://gitlab.example.com:8443");
        assert_eq!(auth.host, "gitlab.example.com");
    }

    #[test]
    fn falls_back_to_ci_server_host_parts() {
        let auth = CiAuth::from_map(&HashMap::from([
            ("CI_JOB_TOKEN", TOKEN),
            ("CI_SERVER_PROTOCOL", "http"),
            ("CI_SERVER_HOST", "gitlab.internal"),
            ("CI_SERVER_PORT", "8080"),
        ]))
        .unwrap();
        assert_eq!(auth.base_url, "http://gitlab.internal:8080");
    }

    #[test]
    fn no_token_means_no_credentials() {
        assert!(CiAuth::from_map(&HashMap::from([(
            "CI_SERVER_URL",
            "https://gitlab.example.com"
        )]))
        .is_none());
    }

    #[test]
    fn opt_out_disables_detection() {
        assert!(CiAuth::from_map(&HashMap::from([
            ("GITSCALE_NO_CI_AUTH", "1"),
            ("CI_JOB_TOKEN", TOKEN),
            ("CI_SERVER_URL", "https://gitlab.example.com"),
        ]))
        .is_none());
    }

    #[test]
    fn github_needs_the_runner_marker_not_just_a_token() {
        let vars = HashMap::from([("GITHUB_TOKEN", TOKEN)]);
        assert!(CiAuth::from_map(&vars).is_none());

        let mut vars = vars;
        vars.insert("GITHUB_ACTIONS", "true");
        let auth = CiAuth::from_map(&vars).unwrap();
        assert_eq!(auth.forge, Forge::GitHub);
        assert_eq!(auth.base_url, "https://github.com");
        assert_eq!(auth.username, "x-access-token");
    }

    #[test]
    fn rewrites_both_ssh_spellings() {
        let auth = gitlab_auth();
        assert_eq!(
            auth.remote_url("git@gitlab.example.com:acme/payments.git")
                .as_deref(),
            Some("https://gitlab.example.com/acme/payments.git")
        );
        assert_eq!(
            auth.remote_url("ssh://git@gitlab.example.com/acme/payments.git")
                .as_deref(),
            Some("https://gitlab.example.com/acme/payments.git")
        );
        // An SSH port says nothing about the HTTPS endpoint and is dropped.
        assert_eq!(
            auth.remote_url("ssh://git@gitlab.example.com:2222/acme/payments.git")
                .as_deref(),
            Some("https://gitlab.example.com/acme/payments.git")
        );
    }

    #[test]
    fn keeps_nested_subgroups() {
        assert_eq!(
            gitlab_auth()
                .remote_url("git@gitlab.example.com:acme/group/sub/repo.git")
                .as_deref(),
            Some("https://gitlab.example.com/acme/group/sub/repo.git")
        );
    }

    #[test]
    fn leaves_matching_https_urls_alone() {
        assert_eq!(
            gitlab_auth()
                .remote_url("https://gitlab.example.com/acme/payments.git")
                .as_deref(),
            Some("https://gitlab.example.com/acme/payments.git")
        );
    }

    #[test]
    fn never_touches_another_host() {
        let auth = gitlab_auth();
        assert!(auth
            .remote_url("git@github.com:acme/gitscale.git")
            .is_none());
        assert!(auth
            .remote_url("https://gitlab.other.example/acme/repo.git")
            .is_none());
        assert!(auth.remote_url("/local/path/to/repo.git").is_none());
    }

    #[test]
    fn config_args_scope_to_the_ci_host_and_omit_the_token() {
        let args = gitlab_auth().git_config_args();
        assert_eq!(
            args.iter().filter(|a| *a == "-c").count() * 2,
            args.len(),
            "every value should be preceded by -c"
        );
        for arg in &args {
            assert!(
                arg == "-c" || arg.starts_with("credential.https://gitlab.example.com."),
                "unscoped config arg: {arg}"
            );
            assert!(!arg.contains(TOKEN), "token value leaked into argv: {arg}");
        }
        assert!(args.contains(&"credential.https://gitlab.example.com.helper=".to_string()));
    }

    #[test]
    fn forbidden_hint_points_at_the_allowlist() {
        assert!(gitlab_auth()
            .forbidden_hint()
            .contains("Job token permissions"));
    }
}
