//! What GitScale puts in a working tree, kept out of git's way: the checkout
//! directories in the root, and the dependency links planted in each
//! checkout. Both are untracked files to the repository holding them, which
//! `git status` would list and `git add -A` would stage.
//!
//! They go in git's own per-repository ignore list, `info/exclude` — read like
//! a `.gitignore`, never committed, and shared by every worktree of the
//! repository. Each worktree has a block of its own in it, so two root
//! worktrees with different checkouts, or two checkouts of one store, keep
//! each other's lines; a block whose worktree is gone is dropped. Lines
//! outside GitScale's blocks are never touched.

use anyhow::{Context, Result};
use std::path::Path;

use crate::resolution::Resolution;

const BEGIN: &str = "# gitscale: ";
const END: &str = "# gitscale: end";

/// Write the blocks for the workspace at `config_root` as `resolution`
/// places it: the root's, listing every checkout, and each git checkout's,
/// listing the links planted in it and the checkouts inside it.
pub fn update(config_root: &Path, resolution: &Resolution) -> Result<()> {
    let locks = crate::git::common_dir(config_root)
        .unwrap_or_else(|| config_root.join(".git"))
        .join("gitscale/locks");
    let dirs: Vec<&str> = resolution
        .slots
        .iter()
        .map(|s| s.directory.as_str())
        .collect();
    write_block(
        &locks,
        config_root,
        dirs.iter().map(|d| anchored(d)).collect(),
    )?;
    for slot in &resolution.slots {
        let dest = config_root.join(&slot.directory);
        if !crate::git::is_checkout(&dest) || dest.is_symlink() {
            continue;
        }
        let mut patterns: Vec<String> = resolution
            .planted_in(&slot.directory)
            .into_iter()
            .map(|link| anchored(&link))
            .collect();
        let inside = Path::new(&slot.directory);
        for dir in &dirs {
            if let Ok(rest) = Path::new(dir).strip_prefix(inside) {
                if !rest.as_os_str().is_empty() {
                    patterns.push(anchored(&rest.to_string_lossy()));
                }
            }
        }
        write_block(&locks, &dest, patterns)?;
    }
    Ok(())
}

/// `path` as a pattern matching exactly it, at the top of its repository:
/// anchored, and every character a pattern gives a meaning taken literally.
fn anchored(path: &str) -> String {
    let mut out = String::from("/");
    for c in path.trim_start_matches('/').chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\' | '!' | '#') {
            out.push('\\');
        }
        out.push(c);
    }
    out.trim_end().to_string()
}

/// Replace the block of the worktree at `worktree` in its repository's
/// `info/exclude` with `patterns`.
fn write_block(locks: &Path, worktree: &Path, mut patterns: Vec<String>) -> Result<()> {
    let Some(file) = crate::git::git_path(worktree, "info/exclude") else {
        return Ok(());
    };
    patterns.sort();
    patterns.dedup();
    let key = worktree
        .canonicalize()
        .unwrap_or_else(|_| worktree.to_path_buf());
    let name = format!(
        "exclude-{}",
        crate::store::entry_name(&file.to_string_lossy())
    );
    let _lock = crate::store::Lock::acquire(locks, Path::new(&name))?;
    let before = std::fs::read_to_string(&file).unwrap_or_default();
    let after = rewrite(&before, &key, &patterns);
    if after == before {
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let partial = file.with_extension("gitscale-partial");
    std::fs::write(&partial, &after)
        .with_context(|| format!("cannot write {}", partial.display()))?;
    std::fs::rename(&partial, &file).with_context(|| format!("cannot write {}", file.display()))
}

/// `text` with the block for `key` holding `patterns` — none when empty —
/// and without the blocks of worktrees that are gone.
fn rewrite(text: &str, key: &Path, patterns: &[String]) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        match line.strip_prefix(BEGIN).filter(|_| line != END) {
            Some(owner) => {
                let owner = Path::new(owner);
                let skip = owner == key || !owner.exists();
                let mut block = vec![line];
                for inner in lines.by_ref() {
                    block.push(inner);
                    if inner == END {
                        break;
                    }
                }
                if !skip {
                    kept.extend(block);
                }
            }
            None => kept.push(line),
        }
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    if !patterns.is_empty() {
        out.push_str(&format!("{}{}\n", BEGIN, key.display()));
        for pattern in patterns {
            out.push_str(pattern);
            out.push('\n');
        }
        out.push_str(END);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_is_replaced_and_everything_else_kept() {
        let here = std::env::temp_dir();
        let key = here.as_path();
        let text = format!(
            "# mine\n*.log\n{b}{k}\n/old\n{e}\n{b}/no/such/worktree\n/gone\n{e}\nlast\n",
            b = BEGIN,
            k = key.display(),
            e = END
        );
        let out = rewrite(&text, key, &["/imports/core".to_string()]);
        assert_eq!(
            out,
            format!(
                "# mine\n*.log\nlast\n{}{}\n/imports/core\n{}\n",
                BEGIN,
                key.display(),
                END
            )
        );
        // Nothing to list: the block goes.
        assert_eq!(rewrite(&out, key, &[]), "# mine\n*.log\nlast\n");
    }

    #[test]
    fn a_path_is_matched_literally_at_the_top() {
        assert_eq!(anchored("imports/core"), "/imports/core");
        assert_eq!(anchored("libs/[slug]*"), "/libs/\\[slug\\]\\*");
        assert_eq!(anchored("#odd"), "/\\#odd");
    }
}
