//! Helpers for the hooks tests.

use super::TestEnv;
use gitscale::trust::ALLOW_ENV;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run the CLI directly, with the arguments as given.
///
/// Never interactive: `run_cli` would sniff the terminal, and an interactive
/// run refreshes the agent skill in the developer's real HOME.
pub fn cli(args: &[&str]) -> super::CliOutput {
    let mut full = vec!["gitscale"];
    full.extend_from_slice(args);
    gitscale::run_cli_with(&full, false)
}

/// A git repo with an initial commit, optionally carrying a gitscale config.
pub fn repo_with(env: &TestEnv, config: Option<&str>) -> PathBuf {
    let repo = env.playground.clone();
    super::run_git_pub(&repo, &["init", "-q", "-b", "main"]);
    super::run_git_pub(&repo, &["config", "user.email", "t@t.com"]);
    super::run_git_pub(&repo, &["config", "user.name", "T"]);
    if let Some(text) = config {
        std::fs::write(repo.join(".gitscale.toml"), text).unwrap();
    }
    std::fs::write(repo.join("f.txt"), "v1").unwrap();
    super::run_git_pub(&repo, &["add", "-A"]);
    super::run_git_pub(&repo, &["commit", "-q", "-m", "init"]);
    repo
}

pub fn hooks_dir(repo: &Path) -> PathBuf {
    repo.join(".git/hooks")
}

pub fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

/// Execute an installed hook script the way git would.
pub fn run_hook(script: &Path, cwd: &Path, sentinel: Option<&str>) -> (i32, String) {
    let mut cmd = Command::new("sh");
    cmd.arg(script).current_dir(cwd);
    match sentinel {
        Some(v) => cmd.env("GITSCALE_HOOK", v),
        None => cmd.env_remove("GITSCALE_HOOK"),
    };
    let out = cmd.output().expect("failed to run hook");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

/// The `ALLOW='...'` line the install baked into a shim.
pub fn baked_allowlist(script: &Path) -> String {
    let body = std::fs::read_to_string(script).unwrap();
    body.lines()
        .find(|l| l.starts_with("ALLOW='"))
        .unwrap_or_else(|| panic!("no ALLOW line in {}", script.display()))
        .trim_start_matches("ALLOW='")
        .trim_end_matches('\'')
        .to_string()
}

/// A throwaway HOME, so a test never reads or writes the developer's own git
/// config.
pub fn isolated_home(env: &TestEnv) -> PathBuf {
    let home = env.playground.join("home");
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// Run the real binary with its own HOME, so a `--global` install writes to a
/// throwaway git config instead of the developer's.
pub fn cli_isolated(home: &Path, args: &[&str]) -> super::CliOutput {
    cli_isolated_with(home, &[], args)
}

/// [`cli_isolated`] with extra environment on top — `GIT_CONFIG_SYSTEM`, say,
/// to give `--system` reads a file of the test's own.
pub fn cli_isolated_with(home: &Path, vars: &[(&str, &str)], args: &[&str]) -> super::CliOutput {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gitscale"));
    isolate(&mut cmd, home);
    cmd.args(args);
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let out = cmd.output().expect("failed to run the gitscale binary");
    super::CliOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        success: out.status.success(),
    }
}

/// Point a command at a throwaway HOME: its git config, XDG config and the
/// hook variables a surrounding gitscale might have exported are the test's
/// own. `GIT_CONFIG_NOSYSTEM`, set by `.cargo/config.toml`, is inherited, so
/// the machine's /etc/gitconfig stays out of every read but `--system` ones.
pub fn isolate(cmd: &mut Command, home: &Path) {
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GITSCALE_HOOK")
        .env_remove(ALLOW_ENV);
}

/// Run git as a developer whose home is `home` — where a `--global` install
/// put its core.hooksPath — so the hooks it fires are the test's.
pub fn git_as(home: &Path, cwd: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("git");
    isolate(&mut cmd, home);
    cmd.args(args).current_dir(cwd);
    cmd.output().expect("failed to run git")
}

/// [`git_as`], asserting success; returns stdout and stderr together.
pub fn git_ok(home: &Path, cwd: &Path, args: &[&str]) -> String {
    let out = git_as(home, cwd, args);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "git {:?} failed:\n{}", args, text);
    text
}

/// Write an executable shell script.
pub fn write_script(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, format!("#!/bin/sh\n{}", body)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A stand-in for the gitscale binary that only records each call — its
/// arguments and the allowlist it was handed — one line per call in the
/// returned log.
pub fn fake_gitscale(dir: &Path) -> (PathBuf, PathBuf) {
    let bin = dir.join("fake-gitscale");
    let log = dir.join("fake-gitscale.log");
    write_script(
        &bin,
        &format!(
            "printf 'args=%s allow=%s\\n' \"$*\" \"${{{}-<unset>}}\" >> '{}'\n",
            ALLOW_ENV,
            log.display()
        ),
    );
    (bin, log)
}

/// The lines a [`fake_gitscale`] log holds; none when it was never called.
pub fn calls(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Rewrite an installed shim to call `bin` instead of the gitscale it was
/// installed with. An in-process install bakes in the test harness binary,
/// which cannot stand for gitscale.
pub fn point_shim_at(shim: &Path, bin: &Path) {
    let body = std::fs::read_to_string(shim).unwrap();
    let body: Vec<String> = body
        .lines()
        .map(|l| {
            if l.starts_with("GITSCALE_BIN=") {
                format!("GITSCALE_BIN='{}'", bin.display())
            } else {
                l.to_string()
            }
        })
        .collect();
    std::fs::write(shim, body.join("\n") + "\n").unwrap();
}

pub use gitscale::commands::hook::HOOKS;

/// Point every shim in `dir` at `bin`.
pub fn point_shims_at(dir: &Path, bin: &Path) {
    for hook in HOOKS {
        let shim = dir.join(hook);
        if shim.is_file() {
            point_shim_at(&shim, bin);
        }
    }
}

/// `hook install --local` in `repo`, with `extra` arguments — every hook,
/// unless they name `--hooks` — and every shim then pointed at the real
/// binary: the shim hands everything to `gitscale hook run`, and an
/// in-process install bakes in the test harness instead. Returns the
/// directory the shims are in.
pub fn install_local(repo: &Path, extra: &[&str]) -> PathBuf {
    let mut args = vec!["hook", "install", "--local", "-C", repo.to_str().unwrap()];
    args.extend_from_slice(extra);
    if !extra.contains(&"--hooks") {
        args.extend_from_slice(&["--hooks", "all"]);
    }
    let out = cli(&args);
    assert!(out.success, "install failed: {}{}", out.stdout, out.stderr);
    let configured = Command::new("git")
        .args(["config", "--local", "--get", "core.hooksPath"])
        .current_dir(repo)
        .output()
        .expect("failed to run git");
    let dir = match String::from_utf8_lossy(&configured.stdout).trim() {
        "" => hooks_dir(repo),
        path => repo.join(path),
    };
    point_shims_at(&dir, Path::new(env!("CARGO_BIN_EXE_gitscale")));
    dir
}

/// Run an installed hook script the way git would, with arguments, from `cwd`;
/// `sentinel` is `GITSCALE_HOOK`. Returns the exit code and all output.
pub fn run_hook_args(
    script: &Path,
    cwd: &Path,
    sentinel: Option<&str>,
    args: &[&str],
) -> (i32, String) {
    let mut cmd = Command::new("sh");
    cmd.arg(script).args(args).current_dir(cwd);
    match sentinel {
        Some(v) => cmd.env("GITSCALE_HOOK", v),
        None => cmd.env_remove("GITSCALE_HOOK"),
    };
    cmd.env_remove(ALLOW_ENV);
    let out = cmd.output().expect("failed to run hook");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

/// [`run_hook_args`] with extra environment and `stdin` fed to the hook.
pub fn run_hook_with(
    script: &Path,
    cwd: &Path,
    args: &[&str],
    vars: &[(&str, &std::ffi::OsStr)],
    stdin: &[u8],
) -> (i32, String) {
    use std::io::Write as _;
    let mut cmd = Command::new("sh");
    cmd.arg(script)
        .args(args)
        .current_dir(cwd)
        .env_remove("GITSCALE_HOOK")
        .env_remove(ALLOW_ENV)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let mut child = cmd.spawn().expect("failed to run hook");
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().expect("failed to run hook");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

/// A stand-in for git-lfs in a directory of its own: each call appends its
/// arguments, then whatever it was given on stdin, to the returned log.
/// Returns that directory, to put in front of `PATH`, and the log.
pub fn fake_lfs(dir: &Path) -> (PathBuf, PathBuf) {
    let bin = dir.join("fake-lfs-bin");
    let log = dir.join("fake-lfs.log");
    write_script(
        &bin.join("git-lfs"),
        &format!(
            "echo \"git-lfs $*\" >> '{log}'\ncat >> '{log}'\n",
            log = log.display()
        ),
    );
    (bin, log)
}

/// `PATH` with `dir` in front.
pub fn path_with(dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).unwrap()
}

/// The hook git-lfs writes for `hook`.
pub fn lfs_stock_hook(hook: &str) -> String {
    format!(
        "command -v git-lfs >/dev/null 2>&1 || {{ printf >&2 \"\\n%s\\n\\n\" \"This \
         repository is configured for Git LFS but 'git-lfs' was not found on your path.\"; \
         exit 2; }}\ngit lfs {} \"$@\"\n",
        hook
    )
}

/// Where `hook run` leaves word of a failed placement.
pub fn breadcrumb(repo: &Path) -> PathBuf {
    repo.join(".git/gitscale-pull-failed")
}
