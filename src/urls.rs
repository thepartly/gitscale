use anyhow::{bail, Result};
use regex::Regex;
use url::Url;

pub fn extract_hostname(repo_url: &str) -> Result<String> {
    // SSH: git@hostname:owner/repo.git
    let ssh_re = Regex::new(r"^[\w-]+@([\w.\-]+):").unwrap();
    if let Some(caps) = ssh_re.captures(repo_url) {
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
    let ssh_re = Regex::new(r"^[\w-]+@[\w.\-]+:(.*)").unwrap();
    let path = if let Some(caps) = ssh_re.captures(repo_url) {
        caps[1].to_string()
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

pub fn extract_owner_repo(repo_url: &str) -> Result<(String, String)> {
    // SSH: git@hostname:owner/repo.git
    let ssh_re = Regex::new(r"^[\w-]+@[\w.\-]+:(.*)").unwrap();
    let path = if let Some(caps) = ssh_re.captures(repo_url) {
        caps[1].to_string()
    } else if let Ok(parsed) = Url::parse(repo_url) {
        parsed.path().to_string()
    } else {
        bail!("Cannot extract owner/repo from URL: {}", repo_url)
    };

    // Strip leading slash and .git suffix
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);

    let parts: Vec<&str> = path.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        bail!("Cannot extract owner/repo from URL: {}", repo_url);
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
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
