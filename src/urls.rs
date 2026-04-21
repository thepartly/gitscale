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
