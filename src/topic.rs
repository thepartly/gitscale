//! Topics: one branch name shared by every repository a change touches.
//!
//! The root's current branch is the topic, unless the root's config pins it —
//! by default only the remote's default branch is pinned, and `[branches]
//! pinned` names the rest. Nothing about a topic is stored apart from the
//! branches themselves: a child is on the topic when its store has a branch of
//! that name, or its remote does — the rule a CI pipeline on the branch
//! applies too, so a workspace and its pipeline resolve alike. `git switch` on
//! the root is how a topic is entered and left; see [`crate::resolution`] for
//! how a topic slot resolves.

use std::path::Path;

use crate::config::GitScaleConfig;

/// What the root's branch makes of the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Root {
    /// On a branch the root does not pin: the topic.
    Topic(String),
    /// On a branch the root pins: every child at its pin.
    Pinned(String),
    /// On no branch at all.
    Detached,
}

impl Root {
    pub fn topic(&self) -> Option<&str> {
        match self {
            Root::Topic(branch) => Some(branch),
            _ => None,
        }
    }
}

/// The branch the workspace at `config_root` is on, and whether it is a topic.
///
/// In a CI pipeline for a branch the root is checked out detached at the
/// pipeline's commit, so the branch comes from the pipeline instead — but only
/// when the root *is* that checkout, at that commit: anything else the job
/// builds is not what the pipeline is for.
pub fn root(config: &GitScaleConfig, config_root: &Path, online: bool) -> Root {
    let (branch, default) = match Pipeline::detect(&|name| std::env::var(name).ok()) {
        Some(pipeline)
            if crate::git::resolve_ref(config_root, "HEAD")
                .is_some_and(|head| head.eq_ignore_ascii_case(&pipeline.commit)) =>
        {
            (pipeline.branch, pipeline.default)
        }
        _ => (
            crate::git::query(config_root, &["symbolic-ref", "--quiet", "--short", "HEAD"]),
            None,
        ),
    };
    let Some(branch) = branch else {
        return Root::Detached;
    };
    let default = default.or_else(|| default_branch(config_root, online));
    if is_pinned(
        &branch,
        config.branches.pinned.as_deref(),
        default.as_deref(),
    ) {
        Root::Pinned(branch)
    } else {
        Root::Topic(branch)
    }
}

/// Whether the root pins `branch`: it is in `pinned` when that is written,
/// otherwise it is the default branch. Unknown — no remote to ask — `main`
/// and `master` are both taken for the default.
pub fn is_pinned(branch: &str, pinned: Option<&[String]>, default: Option<&str>) -> bool {
    match (pinned, default) {
        (Some(patterns), _) => pins(patterns, branch),
        (None, Some(default)) => branch == default,
        (None, None) => branch == "main" || branch == "master",
    }
}

/// Whether `patterns`, as `[branches] pinned` writes them, name `branch`.
pub fn pins(patterns: &[String], branch: &str) -> bool {
    patterns
        .iter()
        .any(|p| crate::trust::wildcard_match(p, branch))
}

/// The default branch of the repository at `repo`'s `origin`, as its
/// `refs/remotes/origin/HEAD` names it. That ref is what `git clone` leaves
/// behind; a repository without it asks the remote once, when `online`, and
/// keeps the answer there for every later, offline, read.
pub fn default_branch(repo: &Path, online: bool) -> Option<String> {
    let read = || {
        crate::git::query(
            repo,
            &[
                "symbolic-ref",
                "--quiet",
                "--short",
                "refs/remotes/origin/HEAD",
            ],
        )
        .and_then(|name| name.strip_prefix("origin/").map(str::to_string))
    };
    if let Some(branch) = read() {
        return Some(branch);
    }
    if online && crate::git::origin_url(repo).is_some() {
        let _ = crate::git::run_git(
            &["remote", "set-head", "origin", "--auto"],
            Some(repo),
            false,
        );
        return read();
    }
    None
}

/// The topic the root just branched from, when its current branch was created
/// a moment ago from that topic — `git switch -c feat/y` while on `feat/x`.
/// The children on `feat/x` then come along to `feat/y`, rather than every one
/// of them going back to its pin.
///
/// Told from git's own records: the branch's reflog holds nothing but its
/// creation, from the old branch, and the worktree's last move was from the
/// old branch to it.
pub fn branched_from(config_root: &Path, branch: &str) -> Option<String> {
    let created = crate::git::query(
        config_root,
        &[
            "reflog",
            "show",
            "--format=%gs",
            &format!("refs/heads/{}", branch),
        ],
    )?;
    let mut entries = created.lines();
    let first = entries.next()?;
    if entries.next().is_some() {
        return None;
    }
    let source = first.strip_prefix("branch: Created from ")?;
    let moved = crate::git::query(
        config_root,
        &["reflog", "show", "-n1", "--format=%gs", "HEAD"],
    )?;
    let from = moved
        .strip_prefix("checkout: moving from ")?
        .strip_suffix(&format!(" to {}", branch))?;
    // Created from where the root was, not from somewhere else while it was
    // there: `git switch -c y origin/main` on topic x starts afresh.
    let from_it = source == "HEAD" || source == from || source == format!("refs/heads/{}", from);
    (from != branch && from_it).then(|| from.to_string())
}

/// The `[topic] prefix` with `{user}` filled in: git config `gitscale.user`,
/// else `$USER` (`USERNAME` on Windows). `None` without a prefix.
pub fn prefix(configured: Option<&str>, repo: &Path) -> anyhow::Result<Option<String>> {
    let Some(prefix) = configured else {
        return Ok(None);
    };
    if !prefix.contains("{user}") {
        return Ok(Some(prefix.to_string()));
    }
    let user = crate::git::query(repo, &["config", "--get", "gitscale.user"])
        .filter(|u| !u.is_empty())
        .or_else(|| std::env::var("USER").ok().filter(|u| !u.is_empty()))
        .or_else(|| std::env::var("USERNAME").ok().filter(|u| !u.is_empty()));
    match user {
        Some(user) => Ok(Some(prefix.replace("{user}", &user))),
        None => anyhow::bail!("set your name for branches: git config --global gitscale.user NAME"),
    }
}

/// The branch `name` stands for: `prefix` in front, unless it is there
/// already.
pub fn with_prefix(name: &str, prefix: Option<&str>) -> String {
    match prefix {
        Some(prefix) if !name.starts_with(prefix) => format!("{}{}", prefix, name),
        _ => name.to_string(),
    }
}

/// The directory name of a topic's worktree: the branch without the
/// prefix, every remaining `/` a `-`, so all topics sit at one level.
pub fn worktree_name(branch: &str, prefix: Option<&str>) -> String {
    let bare = prefix
        .and_then(|p| branch.strip_prefix(p))
        .unwrap_or(branch);
    bare.replace('/', "-")
}

/// A CI pipeline run for a branch: which branch, the commit it checked out,
/// and the project's default branch where the platform says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    /// `None` for a pipeline that is not for a branch — a tag.
    pub branch: Option<String>,
    pub commit: String,
    pub default: Option<String>,
}

impl Pipeline {
    /// The pipeline the environment `var` reads describes, if any.
    pub fn detect(var: &dyn Fn(&str) -> Option<String>) -> Option<Pipeline> {
        let get = |name: &str| var(name).filter(|v| !v.is_empty());
        if get("GITLAB_CI").as_deref() == Some("true") {
            return Some(Pipeline {
                // A merge request pipeline names its source branch; a branch
                // pipeline its branch; a tag pipeline neither.
                branch: get("CI_MERGE_REQUEST_SOURCE_BRANCH_NAME")
                    .or_else(|| get("CI_COMMIT_BRANCH")),
                commit: get("CI_COMMIT_SHA")?,
                default: get("CI_DEFAULT_BRANCH"),
            });
        }
        if get("GITHUB_ACTIONS").as_deref() == Some("true") {
            let branch = get("GITHUB_HEAD_REF").or_else(|| {
                (get("GITHUB_REF_TYPE").as_deref() == Some("branch"))
                    .then(|| get("GITHUB_REF_NAME"))
                    .flatten()
            });
            let default = get("GITHUB_EVENT_PATH")
                .and_then(|path| std::fs::read(path).ok())
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|event| {
                    event["repository"]["default_branch"]
                        .as_str()
                        .map(str::to_string)
                });
            return Some(Pipeline {
                branch,
                commit: get("GITHUB_SHA")?,
                default,
            });
        }
        None
    }

    /// The branch a merge request of this pipeline targets, when it is one.
    pub fn target(var: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        let get = |name: &str| var(name).filter(|v| !v.is_empty());
        get("CI_MERGE_REQUEST_TARGET_BRANCH_NAME").or_else(|| get("GITHUB_BASE_REF"))
    }
}

/// The topic a store branch belongs to: `NAME` for `NAME` and for a
/// version branch `NAME@vN` or `NAME@v0.N`.
pub fn unversioned(branch: &str) -> &str {
    match branch.rsplit_once("@v") {
        Some((topic, v))
            if v.starts_with(|c: char| c.is_ascii_digit())
                && v.bytes().all(|b| b.is_ascii_digit() || b == b'.') =>
        {
            topic
        }
        _ => branch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn detect(vars: &[(&str, &str)]) -> Option<Pipeline> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Pipeline::detect(&|name| map.get(name).cloned())
    }

    #[test]
    fn a_gitlab_merge_request_pipeline_is_for_its_source_branch() {
        let p = detect(&[
            ("GITLAB_CI", "true"),
            ("CI_COMMIT_SHA", "abc"),
            ("CI_COMMIT_BRANCH", "ignored"),
            ("CI_MERGE_REQUEST_SOURCE_BRANCH_NAME", "feat/x"),
            ("CI_DEFAULT_BRANCH", "main"),
        ])
        .unwrap();
        assert_eq!(p.branch.as_deref(), Some("feat/x"));
        assert_eq!(p.default.as_deref(), Some("main"));
    }

    #[test]
    fn a_tag_pipeline_is_for_no_branch() {
        let tag = detect(&[
            ("GITLAB_CI", "true"),
            ("CI_COMMIT_SHA", "abc"),
            ("CI_COMMIT_TAG", "v1"),
        ])
        .unwrap();
        assert_eq!(tag.branch, None);
    }

    #[test]
    fn github_reads_a_pull_request_head_or_a_pushed_branch() {
        let pr = detect(&[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", "abc"),
            ("GITHUB_HEAD_REF", "feat/x"),
            ("GITHUB_REF_NAME", "12/merge"),
        ])
        .unwrap();
        assert_eq!(pr.branch.as_deref(), Some("feat/x"));
        let push = detect(&[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", "abc"),
            ("GITHUB_REF_TYPE", "branch"),
            ("GITHUB_REF_NAME", "staging"),
        ])
        .unwrap();
        assert_eq!(push.branch.as_deref(), Some("staging"));
        let tag = detect(&[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", "abc"),
            ("GITHUB_REF_TYPE", "tag"),
            ("GITHUB_REF_NAME", "v1"),
        ])
        .unwrap();
        assert_eq!(tag.branch, None);
    }

    #[test]
    fn no_pipeline_without_its_platform_or_commit() {
        assert_eq!(detect(&[("CI", "true")]), None);
        assert_eq!(detect(&[("GITLAB_CI", "true")]), None);
    }

    #[test]
    fn a_prefix_is_added_once_and_dropped_from_the_directory() {
        assert_eq!(with_prefix("feat", Some("andrey/")), "andrey/feat");
        assert_eq!(with_prefix("andrey/feat", Some("andrey/")), "andrey/feat");
        assert_eq!(with_prefix("feat", None), "feat");
        assert_eq!(
            worktree_name("andrey/feature-blah", Some("andrey/")),
            "feature-blah"
        );
        assert_eq!(
            worktree_name("andrey/feature/blah", Some("andrey/")),
            "feature-blah"
        );
        assert_eq!(
            worktree_name("andrey/feature-blah", None),
            "andrey-feature-blah"
        );
    }

    #[test]
    fn the_default_branch_is_pinned_unless_the_list_says_otherwise() {
        assert!(is_pinned("main", None, Some("main")));
        assert!(!is_pinned("feat/x", None, Some("main")));
        // Unknown default: main and master both.
        assert!(is_pinned("master", None, None));
        let list = vec!["staging".to_string(), "release/*".to_string()];
        assert!(is_pinned("release/2026.10", Some(&list), Some("main")));
        assert!(is_pinned("staging", Some(&list), Some("main")));
        // A written list is exactly what is pinned: the default is not implied.
        assert!(!is_pinned("main", Some(&list), Some("main")));
        assert!(!is_pinned("main", Some(&[]), Some("main")));
    }

    #[test]
    fn a_version_branch_belongs_to_its_topic() {
        assert_eq!(unversioned("feat"), "feat");
        assert_eq!(unversioned("feat@v2"), "feat");
        assert_eq!(unversioned("feat@v0.3"), "feat");
        assert_eq!(unversioned("feat@vnext"), "feat@vnext");
    }
}
