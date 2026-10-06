//! A topic's branch on a remote, and deleting it. `git upgrade` deletes a
//! promoted slot's before writing its release in; `git topic finish`, every
//! one its default branch already holds.

use anyhow::Result;
use std::path::PathBuf;

/// A branch on a remote, at the tip checked.
pub struct RemoteBranch {
    /// Where it is, as the command names it: `.`, or a checkout's directory.
    pub dir: String,
    pub url: String,
    pub branch: String,
    pub tip: String,
    /// The repository the deletion is pushed from: the root, or the store.
    pub repo: PathBuf,
}

impl RemoteBranch {
    /// Delete it on its remote — only while it is still at the tip checked,
    /// so a push made since fails the deletion instead of being lost. As the
    /// user's own `git push` would: with their credential helpers. The error
    /// is git's own reason.
    pub fn delete(&self) -> Result<()> {
        let lease = format!("--force-with-lease=refs/heads/{}:{}", self.branch, self.tip);
        crate::git::run_git_as_user(
            &[
                "push",
                "--quiet",
                &lease,
                &self.url,
                &format!(":refs/heads/{}", self.branch),
            ],
            Some(&self.repo),
            true,
        )?;
        Ok(())
    }
}
