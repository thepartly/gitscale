use anyhow::{bail, Result};
use regex::Regex;
use std::sync::OnceLock;
use url::Url;

/// An scp-style SSH address, `user@host:path`: the host, then the path.
fn ssh_address() -> &'static Regex {
    static SSH: OnceLock<Regex> = OnceLock::new();
    SSH.get_or_init(|| Regex::new(r"^[\w-]+@([\w.\-]+):(.*)").unwrap())
}

pub fn extract_hostname(repo_url: &str) -> Result<String> {
    // SSH: git@hostname:owner/repo.git
    if let Some(caps) = ssh_address().captures(repo_url) {
        return Ok(caps[1].to_string());
    }

    // HTTPS or other scheme
    if let Ok(parsed) = Url::parse(repo_url) {
        if let Some(host) = parsed.host_str() {
            return Ok(host.to_string());
        }
    }

    bail!("Cannot extract hostname from URL: {}", repo_url)
}

/// The repository path of a URL, without a leading slash and with any `.git`
/// suffix kept — the part that survives a change of transport.
pub fn extract_path(repo_url: &str) -> Result<String> {
    // SSH: git@hostname:owner/repo.git
    let path = if let Some(caps) = ssh_address().captures(repo_url) {
        caps[2].to_string()
    } else if let Ok(parsed) = Url::parse(repo_url) {
        parsed.path().to_string()
    } else {
        bail!("Cannot extract path from URL: {}", repo_url)
    };
    let path = path.trim_matches('/');
    if path.is_empty() {
        bail!("Cannot extract path from URL: {}", repo_url);
    }
    Ok(path.to_string())
}

/// The owner and repository of a URL: the first segment of its path, and
/// everything after it, so a nested GitLab group stays whole in the second.
pub fn extract_owner_repo(repo_url: &str) -> Result<(String, String)> {
    let path = extract_path(repo_url)
        .map_err(|_| anyhow::anyhow!("Cannot extract owner/repo from URL: {}", repo_url))?;
    let path = path.strip_suffix(".git").unwrap_or(&path);
    match path.split_once('/') {
        Some((owner, repo)) if !owner.is_empty() && !repo.is_empty() => {
            Ok((owner.to_string(), repo.to_string()))
        }
        _ => bail!("Cannot extract owner/repo from URL: {}", repo_url),
    }
}

/// Canonicalize a git remote URL so that different transport forms of the same
/// repository (e.g. `git@github.com:org/repo.git` and
/// `https://github.com/org/repo`) compare as equal.
///
/// When the host and owner/repo can be extracted, the canonical form is
/// `host/owner/repo` (lowercased). Otherwise we fall back to stripping a
/// trailing `.git` and lowercasing.
pub fn normalize(url: &str) -> String {
    match (extract_hostname(url), extract_owner_repo(url)) {
        (Ok(host), Ok((owner, repo))) => format!("{}/{}/{}", host, owner, repo).to_lowercase(),
        _ => url.strip_suffix(".git").unwrap_or(url).to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_and_repo_survive_every_transport() {
        let pair = |o: &str, r: &str| (o.to_string(), r.to_string());
        for url in [
            "git@github.com:org/repo.git",
            "https://github.com/org/repo",
            "https://github.com/org/repo.git/",
            "ssh://git@github.com/org/repo.git",
        ] {
            assert_eq!(
                extract_owner_repo(url).unwrap(),
                pair("org", "repo"),
                "{}",
                url
            );
        }
        // A nested group keeps everything after the owner.
        assert_eq!(
            extract_owner_repo("git@gitlab.com:group/sub/repo.git").unwrap(),
            pair("group", "sub/repo")
        );
        assert!(extract_owner_repo("https://github.com/repo").is_err());
        assert!(extract_owner_repo("not a url").is_err());
    }

    #[test]
    fn test_normalize_url() {
        // SSH and HTTPS forms of the same repo canonicalize identically.
        assert_eq!(
            normalize("git@github.com:org/repo.git"),
            "github.com/org/repo"
        );
        assert_eq!(
            normalize("https://github.com/ORG/Repo"),
            "github.com/org/repo"
        );
        assert_eq!(
            normalize("git@github.com:org/repo.git"),
            normalize("https://github.com/org/repo.git")
        );
        // Unparseable URLs fall back to strip-.git + lowercase.
        assert_eq!(normalize("file:///Tmp/Repo.git"), "file:///tmp/repo");
    }
}
