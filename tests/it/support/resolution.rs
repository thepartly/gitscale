//! Helpers for the resolution tests.

use super::{git_stdout, run_git_pub, strip_ansi, TestEnv};
use std::path::{Path, PathBuf};
/// A repository whose `main` gains one commit per `(tag, config)`, each
/// tagged; a non-empty config becomes that commit's `.gitscale.toml`.
pub fn tagged(env: &TestEnv, name: &str, versions: &[(&str, &str)]) -> PathBuf {
    let bare = env.create_bare_repo(name, "main", &[("README.md", name)]);
    for (tag, config) in versions {
        let mut commit = env.push_commit(&bare, "main", "VERSION", tag);
        if !config.is_empty() {
            commit = env.push_commit(&bare, "main", ".gitscale.toml", config);
        }
        run_git_pub(&bare, &["tag", tag, &commit]);
    }
    bare
}

/// A `[repos]` table: `(directory, repository, extra keys)`.
pub fn repos(entries: &[(&str, &Path, &str)]) -> String {
    let mut text = String::from("[repos]\n");
    for (dir, bare, extra) in entries {
        text.push_str(&format!(
            "\"{}\" = {{ url = \"{}\"{} }}\n",
            dir,
            bare.display(),
            extra
        ));
    }
    text
}

/// Lets implicit dependencies come from this environment's repositories,
/// which are local paths — never allowed without saying so.
pub fn allow(env: &TestEnv) -> String {
    format!(
        "[resolve]\nallow = [\"{}/*\"]\n\n",
        env.repos_remote.display()
    )
}

pub fn tag_commit(bare: &Path, tag: &str) -> String {
    git_stdout(bare, &["rev-parse", &format!("{}^{{commit}}", tag)])
}

pub fn head(env: &TestEnv, dir: &str) -> String {
    git_stdout(&env.playground.join(dir), &["rev-parse", "HEAD"])
}

pub fn status_row(env: &TestEnv, dir: &str) -> String {
    let out = env.run(&["status"]);
    assert!(out.success, "{}", out.stderr);
    strip_ansi(&out.stdout)
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(dir))
        .unwrap_or_default()
        .to_string()
}

/// D at v1.2.0, v1.5.0 and v2.0.0; B v1.0.0 needs D v1.5.0.
pub fn diamond(env: &TestEnv) -> (PathBuf, PathBuf) {
    let d = tagged(env, "d", &[("v1.2.0", ""), ("v1.5.0", ""), ("v2.0.0", "")]);
    let b = tagged(
        env,
        "b",
        &[(
            "v1.0.0",
            &repos(&[("libs/d", &d, ", revision = \"v1.5.0\"")]),
        )],
    );
    (b, d)
}

/// A b whose v1.0.0 needs d at v1.5.0, with nothing declaring d at the root:
/// d is an implicit checkout.
pub fn implicit_d(env: &TestEnv) -> (PathBuf, PathBuf) {
    let (b, d) = diamond(env);
    env.write_config(&format!(
        "{}{}",
        allow(env),
        repos(&[("imports/b", &b, ", revision = \"v1.0.0\"")])
    ));
    (b, d)
}

pub fn set_writable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(perms.mode() | 0o200);
    std::fs::set_permissions(path, perms).unwrap();
}

/// A child of `name` at v1.0.0 whose config is `entries`.
pub fn dependant(env: &TestEnv, name: &str, entries: &[(&str, &Path, &str)]) -> PathBuf {
    tagged(env, name, &[("v1.0.0", &repos(entries))])
}

/// A root repository whose `.gitscale.toml` is `config`, cloned to `ws` with
/// an identity: a workspace on its default branch, not pulled yet. Topics
/// need one, since the root's branch is the topic.
pub fn root_workspace(env: &TestEnv, config: &str) -> PathBuf {
    let root = env.create_bare_repo(
        "root",
        "main",
        &[("README.md", "root"), (".gitscale.toml", config)],
    );
    let ws = env.repos_remote.join("ws");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), ws.to_str().unwrap()],
    );
    run_git_pub(&ws, &["config", "user.email", "t@t.com"]);
    run_git_pub(&ws, &["config", "user.name", "T"]);
    ws
}

/// Git configuration, as `GIT_CONFIG_*` variables for a subprocess, that
/// sends every URL starting with one of `prefixes` to this environment's
/// repositories: `https://example.com/org/d.git` reaches `d.git` here.
pub fn rewritten_to_local(env: &TestEnv, prefixes: &[&str]) -> Vec<(String, String)> {
    let local = format!("{}/", env.repos_remote.display());
    let mut vars = vec![("GIT_CONFIG_COUNT".to_string(), prefixes.len().to_string())];
    for (i, prefix) in prefixes.iter().enumerate() {
        vars.push((
            format!("GIT_CONFIG_KEY_{}", i),
            format!("url.{}.insteadOf", local),
        ));
        vars.push((format!("GIT_CONFIG_VALUE_{}", i), prefix.to_string()));
    }
    vars
}

/// `vars` borrowed, as `TestEnv::run_with_env` takes them.
pub fn borrowed(vars: &[(String, String)]) -> Vec<(&str, &str)> {
    vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// Run gitscale as a subprocess with `vars`, recording every git process it
/// starts through git's trace2 events: the output, and each process's argv.
pub fn traced(
    env: &TestEnv,
    vars: &[(&str, &str)],
    args: &[&str],
) -> (super::CliOutput, Vec<Vec<String>>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static RUN: AtomicUsize = AtomicUsize::new(0);
    let dir = env
        .repos_remote
        .join(format!("trace-{}", RUN.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut all: Vec<(&str, &str)> = vars.to_vec();
    let target = dir.to_str().unwrap().to_string();
    all.push(("GIT_TRACE2_EVENT", &target));
    let out = env.run_with_env(&all, args);
    let mut argvs = Vec::new();
    for file in std::fs::read_dir(&dir).unwrap().flatten() {
        let text = std::fs::read_to_string(file.path()).unwrap_or_default();
        for line in text.lines() {
            let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if event["event"] != "start" {
                continue;
            }
            if let Some(argv) = event["argv"].as_array() {
                argvs.push(
                    argv.iter()
                        .map(|a| a.as_str().unwrap_or_default().to_string())
                        .collect(),
                );
            }
        }
    }
    (out, argvs)
}

/// The `git fetch` processes among `argvs`.
pub fn fetches(argvs: &[Vec<String>]) -> Vec<&Vec<String>> {
    argvs
        .iter()
        .filter(|argv| {
            argv.first().is_some_and(|g| g == "git") && argv.iter().any(|a| a == "fetch")
        })
        .collect()
}

/// How many `git fetch` processes ran in the root's store for `url`.
pub fn store_fetches(argvs: &[Vec<String>], url: &Path) -> usize {
    let entry = gitscale::store::entry_name(url.to_str().unwrap());
    fetches(argvs)
        .into_iter()
        .filter(|argv| argv.iter().any(|a| a.ends_with(&entry)))
        .count()
}

/// `n` repositories `c000`, `c001`, …, each at v1.0.0 asking for the next
/// one at v1.0.0 under `libs/<its name>`; the last asks for nothing. Returns
/// them in chain order.
pub fn chain(env: &TestEnv, n: usize) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for i in (0..n).rev() {
        let config = match out.first() {
            Some(next) => repos(&[(
                &format!("libs/c{:03}", i + 1),
                next,
                ", revision = \"v1.0.0\"",
            )]),
            None => String::new(),
        };
        let repo = tagged(env, &format!("c{:03}", i), &[("v1.0.0", &config)]);
        out.insert(0, repo);
    }
    out
}

/// The `.gitscale.toml` of the workspace at `dir`, as it is on disk.
pub fn config_text(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(".gitscale.toml")).unwrap()
}

/// What `upgrade <dir>` raises: D at v1.0.0 and v1.1.0, and B at v1.0.0
/// asking for D v1.0.0 under a comment. Returns B, D, and a root config
/// declaring both at v1.0.0.
pub fn raise_graph(env: &TestEnv) -> (PathBuf, PathBuf, String) {
    let d = tagged(env, "d", &[("v1.0.0", ""), ("v1.1.0", "")]);
    let b = tagged(
        env,
        "b",
        &[(
            "v1.0.0",
            &format!(
                "# what b builds on\n{}",
                repos(&[("libs/d", &d, ", revision = \"v1.0.0\"")])
            ),
        )],
    );
    let config = repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.0.0\""),
    ]);
    (b, d, config)
}
