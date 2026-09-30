//! Explaining SSH authentication failures.
//!
//! GitScale runs git in parallel with every prompt disabled, so a private key
//! protected by a passphrase is only usable through an ssh agent. When ssh
//! finds no usable key, git reports "Could not read from remote repository",
//! which says nothing about that; the hint here says what to fix, based on the
//! state of the agent.

use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// What `ssh-add -l` says about the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Agent {
    /// The agent holds at least one key.
    Keys,
    /// An agent is running but holds no keys.
    Empty,
    /// `SSH_AUTH_SOCK` is not set: no agent was ever started.
    Missing,
    /// `SSH_AUTH_SOCK` is set but nothing answers on it — typically a tmux or
    /// screen session outliving the login that forwarded its agent.
    Stale,
    /// `ssh-add` is not installed, or answered something unexpected.
    Unknown,
}

/// True if ssh gave up because no key it could use was accepted. A key whose
/// passphrase it could not ask for ends up here too: ssh skips it silently.
fn is_auth_failure(stderr: &str) -> bool {
    stderr
        .lines()
        .any(|l| l.contains("Permission denied (") && l.contains("publickey"))
}

/// The `user@host` ssh reported the failure for, as in
/// `git@github.com: Permission denied (publickey).`
fn remote_of(stderr: &str) -> Option<&str> {
    stderr
        .lines()
        .find_map(|l| l.split_once(": Permission denied ("))
        .map(|(remote, _)| remote.trim())
        .filter(|r| !r.is_empty() && !r.contains(char::is_whitespace))
}

/// The hint for a failed git call, if ssh authentication is what failed.
pub(crate) fn failure_hint(stderr: &str) -> Option<String> {
    if !is_auth_failure(stderr) {
        return None;
    }
    let in_ssh_session = std::env::var_os("SSH_CONNECTION").is_some();
    hint(stderr, agent(), in_ssh_session)
}

fn hint(stderr: &str, agent: Agent, in_ssh_session: bool) -> Option<String> {
    if !is_auth_failure(stderr) {
        return None;
    }
    let passphrase = "gitscale cannot ask for a key passphrase, \
                      so a key that has one must be loaded into an ssh agent.";
    let fix = match agent {
        Agent::Keys => {
            let check = match remote_of(stderr) {
                Some(remote) => format!("`ssh -T {}`", remote),
                None => "`ssh -T <user>@<host>`".to_string(),
            };
            return Some(format!(
                "the ssh agent holds keys, but the server accepted none of them.\n\
                 hint: check that one is registered with the git host; {} shows which key is used.",
                check
            ));
        }
        Agent::Empty => "the ssh agent holds no keys: run `ssh-add`, then retry.".to_string(),
        Agent::Missing if in_ssh_session => "no ssh agent in this SSH session: reconnect with \
             `ForwardAgent yes` for this host in ~/.ssh/config on the machine you connect from, \
             or start one here with `eval \"$(ssh-agent -s)\" && ssh-add`."
            .to_string(),
        Agent::Missing => "no ssh agent is running: start one and load the key with \
             `eval \"$(ssh-agent -s)\" && ssh-add`."
            .to_string(),
        Agent::Stale => "SSH_AUTH_SOCK names an ssh agent that does not answer (in tmux or \
             screen it may be left from an earlier login): point it at a live agent, then retry."
            .to_string(),
        Agent::Unknown => "load it with `ssh-add`, then retry.".to_string(),
    };
    Some(format!("{}\nhint: {}", passphrase, fix))
}

/// Ask the agent once per process: parallel failures all want the answer, and
/// it does not change in the meantime.
fn agent() -> Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    *AGENT.get_or_init(|| {
        if std::env::var_os("SSH_AUTH_SOCK").is_none_or(|s| s.is_empty()) {
            return Agent::Missing;
        }
        let status = Command::new("ssh-add")
            .arg("-l")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match status.ok().and_then(|s| s.code()) {
            Some(0) => Agent::Keys,
            Some(1) => Agent::Empty,
            Some(2) => Agent::Stale,
            _ => Agent::Unknown,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{hint, remote_of, Agent};

    const DENIED: &str = "** WARNING: connection is not using a post-quantum key exchange algorithm.\n\
                          git@github.com: Permission denied (publickey).\n\
                          fatal: Could not read from remote repository.";

    #[test]
    fn ignores_failures_other_than_ssh_auth() {
        let stderr = "fatal: repository 'https://example.com/x.git/' not found";
        assert_eq!(hint(stderr, Agent::Missing, false), None);
        assert_eq!(hint("Host key verification failed.", Agent::Empty, false), None);
    }

    #[test]
    fn empty_agent_asks_for_ssh_add() {
        let h = hint(DENIED, Agent::Empty, false).unwrap();
        assert!(h.contains("cannot ask for a key passphrase"));
        assert!(h.contains("run `ssh-add`"));
    }

    #[test]
    fn missing_agent_in_an_ssh_session_suggests_forwarding() {
        let h = hint(DENIED, Agent::Missing, true).unwrap();
        assert!(h.contains("ForwardAgent yes"));
        assert!(h.contains("ssh-agent -s"));
        let local = hint(DENIED, Agent::Missing, false).unwrap();
        assert!(!local.contains("ForwardAgent"));
        assert!(local.contains("ssh-agent -s"));
    }

    #[test]
    fn stale_socket_is_named() {
        let h = hint(DENIED, Agent::Stale, true).unwrap();
        assert!(h.contains("SSH_AUTH_SOCK"));
    }

    #[test]
    fn loaded_agent_points_at_the_registered_key() {
        let h = hint(DENIED, Agent::Keys, false).unwrap();
        assert!(!h.contains("passphrase"));
        assert!(h.contains("`ssh -T git@github.com`"));
    }

    #[test]
    fn finds_the_remote_that_refused() {
        assert_eq!(remote_of(DENIED), Some("git@github.com"));
        assert_eq!(
            remote_of("git@gitlab.example.com: Permission denied (publickey,password)."),
            Some("git@gitlab.example.com")
        );
        assert_eq!(remote_of("Permission denied (publickey)."), None);
    }
}
