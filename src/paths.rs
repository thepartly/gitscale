//! Directory arguments: every argument naming a checkout is a path relative
//! to the current directory — `-C PATH` when given, as git's own `-C` — taken
//! lexically and then made relative to the workspace root.

use anyhow::{bail, Result};
use std::path::{Component, Path, PathBuf};

use crate::resolution::{Resolution, Slot};

/// Where a command runs: the directory its paths are relative to, and the
/// workspace root.
#[derive(Debug, Clone)]
pub struct Here {
    pub cwd: PathBuf,
    pub root: PathBuf,
}

/// What a directory argument names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// The root repository.
    Root,
    /// A checkout, by its slot's directory.
    Slot(String),
}

impl Here {
    /// For the workspace at `root`, from `start` (`-C`) or the current
    /// directory.
    pub fn new(start: Option<&Path>, root: &Path) -> Result<Here> {
        let cwd = match start {
            Some(p) => p.canonicalize().unwrap_or_else(|_| p.to_path_buf()),
            None => std::env::current_dir()?,
        };
        Ok(Here {
            cwd,
            root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        })
    }

    /// `arg` relative to the root, lexically: empty for the root itself.
    pub fn relative(&self, arg: &str) -> Result<PathBuf> {
        let joined = lexical(&self.cwd.join(arg));
        match joined.strip_prefix(&self.root) {
            Ok(rel) => Ok(rel.to_path_buf()),
            Err(_) => bail!("{} is outside the workspace", arg),
        }
    }

    /// The checkout `arg` names: its own path, or the path of a link a
    /// repository has to it. `.` at the root is the root repository where
    /// `accept_root` says a command takes it.
    pub fn name(&self, resolution: &Resolution, arg: &str, accept_root: bool) -> Result<Named> {
        let rel = self.relative(arg)?;
        if rel.as_os_str().is_empty() {
            if accept_root {
                return Ok(Named::Root);
            }
            bail!("the root is not a checkout");
        }
        match resolution.find(&rel.to_string_lossy()) {
            Some(slot) => Ok(Named::Slot(slot.directory.clone())),
            None => bail!("{} is not a checkout of this workspace", arg),
        }
    }

    /// The slot `arg` names; the root is refused.
    pub fn slot<'a>(&self, resolution: &'a Resolution, arg: &str) -> Result<&'a Slot> {
        match self.name(resolution, arg, false)? {
            Named::Slot(dir) => Ok(resolution
                .slot(&dir)
                .expect("named by the resolution itself")),
            Named::Root => unreachable!("the root is refused above"),
        }
    }

    /// The checkout the current directory is in, and the path from the
    /// current directory to its top: `None` in the root's own files.
    pub fn enclosing<'a>(&self, resolution: &'a Resolution) -> Option<(&'a Slot, String)> {
        let rel = self.cwd.strip_prefix(&self.root).ok()?;
        let slot = resolution
            .slots
            .iter()
            .filter(|s| rel.starts_with(&s.directory))
            .filter(|s| {
                let dest = self.root.join(&s.directory);
                crate::git::is_checkout(&dest) && !dest.is_symlink()
            })
            .max_by_key(|s| Path::new(&s.directory).components().count())?;
        let depth = rel
            .strip_prefix(&slot.directory)
            .map(|inner| inner.components().count())
            .unwrap_or(0);
        let up = if depth == 0 {
            ".".to_string()
        } else {
            vec![".."; depth].join("/")
        };
        Some((slot, up))
    }

    /// `path`, for showing: relative to the current directory.
    pub fn show(&self, path: &Path) -> String {
        relative_to(&self.cwd, path)
    }
}

/// Directory arguments made relative to the workspace root at `root`, for a
/// command that matches them against entries itself.
pub fn relative_names(start: Option<&Path>, root: &Path, args: &[String]) -> Result<Vec<String>> {
    let here = Here::new(start, root)?;
    args.iter()
        .map(|arg| {
            let rel = here.relative(arg)?;
            if rel.as_os_str().is_empty() {
                bail!("the root is not a checkout");
            }
            Ok(rel.to_string_lossy().into_owned())
        })
        .collect()
}

/// `path` with `.` and `..` taken out, without touching the filesystem.
pub fn lexical(path: &Path) -> PathBuf {
    let mut parts: Vec<Component> = Vec::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(parts.last(), Some(Component::Normal(_))) {
                    parts.pop();
                } else if !matches!(parts.last(), Some(Component::RootDir)) {
                    parts.push(part);
                }
            }
            other => parts.push(other),
        }
    }
    parts.iter().collect()
}

/// `to` as a path from the directory `from`: `.` when they are the same.
pub fn relative_to(from: &Path, to: &Path) -> String {
    let (from, to) = (lexical(from), lexical(to));
    let from: Vec<Component> = from.components().collect();
    let to_parts: Vec<Component> = to.components().collect();
    let common = from
        .iter()
        .zip(&to_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for part in &to_parts[common..] {
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        ".".to_string()
    } else {
        out.to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here(cwd: &str) -> Here {
        Here {
            cwd: PathBuf::from(cwd),
            root: PathBuf::from("/ws"),
        }
    }

    #[test]
    fn arguments_are_relative_to_the_current_directory() {
        assert_eq!(
            here("/ws/imports").relative("core").unwrap(),
            PathBuf::from("imports/core")
        );
        assert_eq!(
            here("/ws/imports/core").relative(".").unwrap(),
            PathBuf::from("imports/core")
        );
        assert_eq!(
            here("/ws/imports/core").relative("../b").unwrap(),
            PathBuf::from("imports/b")
        );
        assert_eq!(here("/ws").relative(".").unwrap(), PathBuf::new());
        assert!(here("/ws").relative("..").is_err());
        assert!(here("/ws/imports").relative("../../etc").is_err());
    }

    #[test]
    fn a_path_is_shown_from_the_current_directory() {
        assert_eq!(relative_to(Path::new("/a/b"), Path::new("/a/c")), "../c");
        assert_eq!(relative_to(Path::new("/a"), Path::new("/a/c/d")), "c/d");
        assert_eq!(relative_to(Path::new("/a"), Path::new("/a")), ".");
    }
}
