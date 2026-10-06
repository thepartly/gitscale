use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use gitscale::run_cli_with;
pub use gitscale::CliOutput;

pub mod artefacts;
pub mod ci_auth;
pub mod ci_cache;
pub mod ci_checkout;
pub mod git_http;
pub mod hooks;
pub mod real_registry;
pub mod registry;
pub mod registry_access;
pub mod resolution;
pub mod skill;
pub mod status_clean;
pub mod workspace;
pub mod worktrees;
pub use registry::FakeRegistry;

/// Root dir for all test runtime artefacts, relative to workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tests_base() -> PathBuf {
    workspace_root().join("tests")
}

/// A self-contained test environment with its own playground, remote repos,
/// cache and — started on first use — OCI registry.
pub struct TestEnv {
    pub playground: PathBuf,
    pub repos_remote: PathBuf,
    pub cache: PathBuf,
    name: String,
    registry: OnceLock<FakeRegistry>,
    /// A real registry to use instead of the fake one.
    registry_override: OnceLock<String>,
}

impl TestEnv {
    pub fn new(name: &str) -> Self {
        let base = tests_base();
        // Scope the on-disk directories to this process. Test names are unique
        // within a binary, but two concurrent `cargo test` invocations would
        // otherwise share `tests/playground/<name>` and wipe each other's
        // clones mid-run via the `remove_dir_all` below — surfacing as flaky
        // snapshot failures in the multi-repo tests.
        let name = format!("{}-{}", std::process::id(), name);
        let name = name.as_str();
        let playground = base.join("playground").join(name);
        let repos_remote = base.join("repos-remote").join(name);
        let cache = base.join("cache").join(name);

        // Clean slate
        let _ = fs::remove_dir_all(&playground);
        let _ = fs::remove_dir_all(&repos_remote);
        let _ = fs::remove_dir_all(&cache);

        fs::create_dir_all(&playground).unwrap();
        fs::create_dir_all(&repos_remote).unwrap();
        // A workspace is the top of a git repository: its stores and records
        // live in that repository's git directory.
        run_git(&playground, &["init", "--quiet", "-b", "main"]);
        run_git(&playground, &["config", "user.email", "test@test.com"]);
        run_git(&playground, &["config", "user.name", "Test"]);

        Self {
            playground,
            repos_remote,
            cache,
            name: name.to_string(),
            registry: OnceLock::new(),
            registry_override: OnceLock::new(),
        }
    }

    /// Create a bare git repo with an initial commit containing the given files.
    /// Returns the path to the bare repo (usable as a git remote URL).
    pub fn create_bare_repo(
        &self,
        repo_name: &str,
        branch: &str,
        files: &[(&str, &str)],
    ) -> PathBuf {
        let bare_path = self.repos_remote.join(format!("{}.git", repo_name));
        fs::create_dir_all(&bare_path).unwrap();

        // Init bare repo
        run_git(&bare_path, &["init", "--bare"]);

        // Create a temp working clone to make commits
        let tmp_clone = self.repos_remote.join(format!("{}-tmp", repo_name));
        let _ = fs::remove_dir_all(&tmp_clone);
        run_git(
            &self.repos_remote,
            &[
                "clone",
                bare_path.to_str().unwrap(),
                tmp_clone.to_str().unwrap(),
            ],
        );

        // Configure git identity for commits
        run_git(&tmp_clone, &["config", "user.email", "test@test.com"]);
        run_git(&tmp_clone, &["config", "user.name", "Test"]);

        // Create branch if not default
        run_git(&tmp_clone, &["checkout", "-b", branch]);

        // Write files and commit
        for (name, content) in files {
            let file_path = tmp_clone.join(name);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&file_path, content).unwrap();
        }
        run_git(&tmp_clone, &["add", "."]);
        run_git(&tmp_clone, &["commit", "-m", "initial"]);
        run_git(
            &tmp_clone,
            &["push", "origin", &format!("{}:{}", branch, branch)],
        );
        // Make `branch` the remote's default, as on a real host: `init --bare`
        // points HEAD at git's built-in default, which may name no branch here.
        run_git(
            &bare_path,
            &["symbolic-ref", "HEAD", &format!("refs/heads/{}", branch)],
        );

        // Clean up temp clone
        let _ = fs::remove_dir_all(&tmp_clone);

        bare_path
    }

    /// This environment's registry, started on first use.
    pub fn registry(&self) -> &FakeRegistry {
        self.registry.get_or_init(FakeRegistry::start)
    }

    /// Publish to and pull from the registry at `addr` (`host:port`) rather
    /// than the fake one — for the conformance tests.
    pub fn use_registry(&self, addr: &str) {
        let _ = self.registry_override.set(addr.to_string());
    }

    pub fn registry_addr(&self) -> String {
        match self.registry_override.get() {
            Some(addr) => addr.clone(),
            None => self.registry().addr.clone(),
        }
    }

    /// The `[registries]` table that maps every repository under
    /// `repos_remote` to this environment's registry: `<name>.git` publishes
    /// as `<addr>/<name>/gitscale`.
    pub fn registries(&self) -> String {
        format!(
            "[registries]\n\"{}/\" = \"{}\"\n\n",
            self.repos_remote.display(),
            self.registry_addr()
        )
    }

    /// The image repository `bare` publishes to, as the registry names it.
    pub fn image(&self, bare: &Path) -> String {
        let name = bare.file_stem().unwrap().to_string_lossy().to_lowercase();
        format!("{}/gitscale", name)
    }

    /// A repository released as `v1.0.0`, the head of `main`, with an
    /// artefact published for it: `files` as the build output, one layer.
    /// Returns the bare repo, which is what an entry's `url` names.
    pub fn artefact_repo(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let bare = self.create_bare_repo(name, "main", &[("README.md", name)]);
        self.release(&bare, "v1.0.0", files);
        bare
    }

    /// Release the head of `main` in `bare` as `tag`, publishing `files` as
    /// its artefact. Returns the commit.
    pub fn release(&self, bare: &Path, tag: &str, files: &[(&str, &str)]) -> String {
        run_git(bare, &["tag", tag, "main"]);
        let artefact = "[artefact]\ninclude = [\"dist/**\"]\n";
        let (out, commit) = self.publish_with(bare, "main", artefact, files, &[tag]);
        assert!(out.success, "publish failed:\n{}{}", out.stdout, out.stderr);
        commit
    }

    /// Publish `files` as the artefact of the commit `revision` names in
    /// `bare`, with gitscale's own `artefact publish`: the build output under
    /// `dist/`, which the producer ignores, so a consumer finds each file at
    /// `dist/<name>`. The image is tagged with its source hash, and released
    /// as the version tag the commit has, if any. Returns the commit.
    pub fn publish(&self, bare: &Path, revision: &str, files: &[(&str, &str)]) -> String {
        let artefact = "[artefact]\ninclude = [\"dist/**\"]\n";
        let tagged = git_stdout(bare, &["tag", "--points-at", revision]);
        let release: Vec<&str> = tagged
            .lines()
            .filter(|t| gitscale::version::parse(t).is_some())
            .take(1)
            .collect();
        let (out, commit) = self.publish_with(bare, revision, artefact, files, &release);
        assert!(out.success, "publish failed:\n{}{}", out.stdout, out.stderr);
        commit
    }

    /// Publish from a fresh checkout of `revision` with the given `[artefact]`
    /// table, build output and extra arguments. Returns the command's output
    /// and the commit published for.
    pub fn publish_with(
        &self,
        bare: &Path,
        revision: &str,
        artefact_toml: &str,
        files: &[(&str, &str)],
        args: &[&str],
    ) -> (CliOutput, String) {
        let producer = self.producer(bare, revision);
        for (name, content) in files {
            let path = producer.join("dist").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        // Build output is ignored, as the artefact policy asks of anything an
        // image ships that the commit does not track.
        let ignore = producer.join(".git/info/exclude");
        fs::create_dir_all(ignore.parent().unwrap()).unwrap();
        let mut rules = fs::read_to_string(&ignore).unwrap_or_default();
        rules.push_str("/dist/\n");
        fs::write(&ignore, rules).unwrap();
        fs::write(
            producer.join(".gitscale.toml"),
            format!("{}{}", self.registries(), artefact_toml),
        )
        .unwrap();
        let mut full = vec!["artefact", "publish"];
        full.extend_from_slice(args);
        let out = self.run_in(&producer, &full);
        let commit = git_stdout(&producer, &["rev-parse", "HEAD"]);
        (out, commit)
    }

    /// A fresh checkout of `bare` at `revision`, where a producer's build
    /// would run.
    pub fn producer(&self, bare: &Path, revision: &str) -> PathBuf {
        let name = bare.file_stem().unwrap().to_string_lossy().into_owned();
        let producer = self.repos_remote.join(format!("{}-producer", name));
        let _ = fs::remove_dir_all(&producer);
        run_git(
            &self.repos_remote,
            &[
                "clone",
                "--quiet",
                bare.to_str().unwrap(),
                producer.to_str().unwrap(),
            ],
        );
        run_git(&producer, &["checkout", "--quiet", revision]);
        producer
    }

    /// Add a commit to `branch` of `bare`. Returns the new commit.
    pub fn push_commit(&self, bare: &Path, branch: &str, file: &str, content: &str) -> String {
        let work = self.repos_remote.join("push-tmp");
        let _ = fs::remove_dir_all(&work);
        run_git(
            &self.repos_remote,
            &[
                "clone",
                "--quiet",
                "--branch",
                branch,
                bare.to_str().unwrap(),
                work.to_str().unwrap(),
            ],
        );
        run_git(&work, &["config", "user.email", "test@test.com"]);
        run_git(&work, &["config", "user.name", "Test"]);
        fs::write(work.join(file), content).unwrap();
        run_git(&work, &["add", "."]);
        run_git(&work, &["commit", "--quiet", "-m", "change"]);
        run_git(&work, &["push", "--quiet", "origin", branch]);
        let commit = git_stdout(&work, &["rev-parse", "HEAD"]);
        let _ = fs::remove_dir_all(&work);
        commit
    }

    /// Run gitscale with `-C dir`, for a command aimed somewhere other than
    /// the playground. `-C` goes first, where git commands take it too.
    /// Rendering is forced plain and sequential: the harness may run under a
    /// TTY, and interactive mode would emit parallel progress bars to stderr
    /// instead of the deterministic stdout the snapshots capture.
    pub fn run_in(&self, dir: &Path, args: &[&str]) -> CliOutput {
        let mut full_args = vec!["gitscale", "-C", dir.to_str().unwrap()];
        full_args.extend_from_slice(args);
        run_cli_with(&full_args, false)
    }

    /// Write a .gitscale.toml config in the playground directory.
    pub fn write_config(&self, config_content: &str) {
        fs::write(self.playground.join(".gitscale.toml"), config_content).unwrap();
    }

    /// Take `repo` as `form` in the playground's workspace, as `git scale
    /// prefer` records it.
    pub fn prefer(&self, repo: &Path, form: gitscale::prefer::Form) {
        prefer(&self.playground, repo, form);
    }

    /// The root's own store for `url`: where every checkout of it is a
    /// worktree of.
    pub fn store(&self, url: &str) -> PathBuf {
        let common = git_stdout(
            &self.playground,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        );
        PathBuf::from(common)
            .join("gitscale/repos")
            .join(gitscale::store::entry_name(url))
    }

    /// The CI cache entries that exist right now, by directory name.
    pub fn cache_entries(&self, kind: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.cache.join(kind))
            .map(|listing| {
                listing
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The single CI cache entry for `url`, which must exist.
    pub fn cache_entry(&self, kind: &str, url: &str) -> PathBuf {
        let name = if kind == "images" {
            gitscale::store::image_entry_name(url)
        } else {
            gitscale::store::entry_name(url)
        };
        let path = self.cache.join(kind).join(name);
        assert!(
            path.is_dir(),
            "no {} entry for {} at {}",
            kind,
            url,
            path.display()
        );
        path
    }

    /// Run the real gitscale binary with extra environment variables — the
    /// only way to exercise anything that reads the process environment (`CI`,
    /// say), since the in-process runner shares one environment across every
    /// test thread. The CI cache is this test's own unless `vars` names one.
    pub fn run_with_env(&self, vars: &[(&str, &str)], args: &[&str]) -> CliOutput {
        self.run_binary_with(None, vars, args)
    }

    /// Run gitscale CLI with extra args, using this env's playground as root.
    pub fn run(&self, args: &[&str]) -> CliOutput {
        self.run_in(&self.playground, args)
    }

    /// Run the real gitscale binary as a subprocess with the allowlist the
    /// installed hook shim would have passed it.
    ///
    /// The in-process `run` cannot be used for this: the test harness runs
    /// tests on threads of one process, and setting an environment variable
    /// there would be visible to every other test at once.
    pub fn run_as_hook(&self, allow: &str, args: &[&str]) -> CliOutput {
        self.run_binary(Some(allow), args)
    }

    /// The same, without an allowlist — a `gitscale` command the user typed.
    pub fn run_binary_plain(&self, args: &[&str]) -> CliOutput {
        self.run_binary(None, args)
    }

    fn run_binary(&self, allow: Option<&str>, args: &[&str]) -> CliOutput {
        self.run_binary_with(allow, &[], args)
    }

    fn run_binary_with(
        &self,
        allow: Option<&str>,
        vars: &[(&str, &str)],
        args: &[&str],
    ) -> CliOutput {
        let mut full_args: Vec<String> = vec![
            "-C".to_string(),
            self.playground.to_str().unwrap().to_string(),
        ];
        full_args.extend(args.iter().map(|a| a.to_string()));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_gitscale"));
        cmd.args(&full_args)
            .env_remove("GITSCALE_HOOK_ALLOW")
            .env("GITSCALE_CACHE_DIR", &self.cache);
        if let Some(allow) = allow {
            cmd.env("GITSCALE_HOOK_ALLOW", allow);
        }
        for (name, value) in vars {
            cmd.env(name, value);
        }
        let output = cmd.output().expect("failed to run the gitscale binary");
        CliOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            success: output.status.success(),
        }
    }

    /// Run `gitscale hook run <hook>` on the playground the way the installed
    /// shim does — sentinel and allowlist set — with extra environment on top.
    /// `hook` is a nested subcommand, so `run_binary`'s `-C` placement does not
    /// fit it.
    pub fn run_hook_run(&self, hook: &str, allow: &str, vars: &[(&str, &str)]) -> CliOutput {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_gitscale"));
        cmd.args(["hook", "run", hook, "-C", self.playground.to_str().unwrap()])
            .env("GITSCALE_HOOK", hook)
            .env("GITSCALE_HOOK_ALLOW", allow)
            .env("GITSCALE_CACHE_DIR", &self.cache);
        for (name, value) in vars {
            cmd.env(name, value);
        }
        let output = cmd.output().expect("failed to run the gitscale binary");
        CliOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            success: output.status.success(),
        }
    }

    /// Point the playground's own repo at `url`, so the hook allowlist has a
    /// deterministic host/owner/repo to judge it by. Its default branch is
    /// recorded as a clone would have it, so nothing ever asks `url` — which
    /// is a name for the allowlist, not a remote to reach.
    pub fn set_playground_origin(&self, url: &str) {
        run_git(&self.playground, &["remote", "add", "origin", url]);
        run_git(
            &self.playground,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
    }

    /// Give the playground's repo a first commit, so HEAD exists.
    pub fn init_playground_git(&self) {
        fs::write(self.playground.join(".gitkeep"), "").unwrap();
        run_git(&self.playground, &["add", ".gitkeep"]);
        run_git(&self.playground, &["commit", "--quiet", "-m", "init"]);
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        // Keep dirs if GITSCALE_TEST_KEEP is set (useful for debugging)
        if std::env::var("GITSCALE_TEST_KEEP").is_ok() {
            return;
        }
        let base = tests_base();
        let _ = fs::remove_dir_all(base.join("playground").join(&self.name));
        let _ = fs::remove_dir_all(base.join("repos-remote").join(&self.name));
        let _ = fs::remove_dir_all(base.join("cache").join(&self.name));
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
    run_git_pub(cwd, args);
}

/// A git command's trimmed stdout, which must succeed.
pub fn git_stdout(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {:?} in {} failed: {}",
        args,
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Replace every 7- or 40-digit hex commit with `[sha]`, for snapshots of
/// output that names commits made at test time.
pub fn redact_shas(text: &str) -> String {
    let re = regex::Regex::new(r"\b[0-9a-f]{40}\b|\b[0-9a-f]{7}\b").unwrap();
    re.replace_all(text, "[sha]").to_string()
}

pub fn run_git_pub(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run git");
    if !output.status.success() {
        panic!(
            "git {:?} in {} failed:\n{}{}",
            args,
            cwd.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

/// Strip ANSI escape codes from a string (for snapshot comparison).
pub fn strip_ansi(s: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(s, "").to_string()
}

/// Make `path` writable again: a checkout at its pin is read-only, and a test
/// standing in for someone editing it anyway has to say so.
pub fn make_writable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(perms.mode() | 0o200);
    fs::set_permissions(path, perms).unwrap();
}

/// Write `content` to `path` in a checkout, read-only or not.
pub fn edit(path: &Path, content: &str) {
    if path.exists() {
        make_writable(path);
    }
    fs::write(path, content).unwrap();
}

/// Take `repo` as `form` in the workspace at `ws`, as `git scale prefer`
/// records it.
pub fn prefer(ws: &Path, repo: &Path, form: gitscale::prefer::Form) {
    let mut prefs = gitscale::prefer::Prefs::load(ws).unwrap();
    prefs.set(repo.to_str().unwrap(), form);
    prefs.save(ws).unwrap();
}
