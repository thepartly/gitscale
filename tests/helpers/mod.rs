use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use gitscale::run_cli;
pub use gitscale::CliOutput;

/// Root dir for all test runtime artefacts, relative to workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tests_base() -> PathBuf {
    workspace_root().join("tests")
}

/// A self-contained test environment with its own playground, remote repos, and artefact storage.
pub struct TestEnv {
    pub playground: PathBuf,
    pub repos_remote: PathBuf,
    pub artefacts_remote: PathBuf,
    name: String,
}

impl TestEnv {
    pub fn new(name: &str) -> Self {
        let base = tests_base();
        let playground = base.join("playground").join(name);
        let repos_remote = base.join("repos-remote").join(name);
        let artefacts_remote = base.join("artefacts-remote").join(name);

        // Clean slate
        let _ = fs::remove_dir_all(&playground);
        let _ = fs::remove_dir_all(&repos_remote);
        let _ = fs::remove_dir_all(&artefacts_remote);

        fs::create_dir_all(&playground).unwrap();
        fs::create_dir_all(&repos_remote).unwrap();
        fs::create_dir_all(&artefacts_remote).unwrap();

        Self {
            playground,
            repos_remote,
            artefacts_remote,
            name: name.to_string(),
        }
    }

    /// Create a bare git repo with an initial commit containing the given files.
    /// Returns the path to the bare repo (usable as a git remote URL).
    pub fn create_bare_repo(&self, repo_name: &str, branch: &str, files: &[(&str, &str)]) -> PathBuf {
        let bare_path = self.repos_remote.join(format!("{}.git", repo_name));
        fs::create_dir_all(&bare_path).unwrap();

        // Init bare repo
        run_git(&bare_path, &["init", "--bare"]);

        // Create a temp working clone to make commits
        let tmp_clone = self.repos_remote.join(format!("{}-tmp", repo_name));
        let _ = fs::remove_dir_all(&tmp_clone);
        run_git(
            &self.repos_remote,
            &["clone", bare_path.to_str().unwrap(), tmp_clone.to_str().unwrap()],
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
        run_git(&tmp_clone, &["push", "origin", &format!("{}:{}", branch, branch)]);

        // Clean up temp clone
        let _ = fs::remove_dir_all(&tmp_clone);

        bare_path
    }

    /// Create an artefact .tar.gz in local storage for the given repo URL and revision.
    pub fn create_artefact(&self, repo_url: &str, revision: &str, files: &[(&str, &str)]) {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use tar::Builder;

        // Build the object path the same way storage.rs does
        let obj_url = gitscale::storage::object_url(
            self.artefacts_remote.to_str().unwrap(),
            repo_url,
            revision,
        )
        .unwrap();

        let obj_path = PathBuf::from(&obj_url);
        if let Some(parent) = obj_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        let file = fs::File::create(&obj_path).unwrap();
        let enc = GzEncoder::new(file, Compression::default());
        let mut builder = Builder::new(enc);

        for (name, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, content.as_bytes())
                .unwrap();
        }

        builder.finish().unwrap();
    }

    /// Write a .gitscale.toml config in the playground directory.
    pub fn write_config(&self, config_content: &str) {
        fs::write(self.playground.join(".gitscale.toml"), config_content).unwrap();
    }

    /// Storage URL pointing to local artefact dir.
    pub fn storage_url(&self) -> String {
        self.artefacts_remote.to_str().unwrap().to_string()
    }

    /// Run gitscale CLI with extra args, using this env's playground as root.
    pub fn run(&self, args: &[&str]) -> CliOutput {
        let mut full_args = vec!["gitscale"];
        // Insert subcommand first, then -C root, then remaining args
        if let Some((subcmd, rest)) = args.split_first() {
            full_args.push(subcmd);
            full_args.push("-C");
            full_args.push(self.playground.to_str().unwrap());
            full_args.extend_from_slice(rest);
        }
        run_cli(&full_args)
    }

    /// Init the playground as a git repo (needed for status to show self ".")
    pub fn init_playground_git(&self) {
        run_git(&self.playground, &["init"]);
        run_git(&self.playground, &["config", "user.email", "test@test.com"]);
        run_git(&self.playground, &["config", "user.name", "Test"]);
        // Initial commit so HEAD exists
        fs::write(self.playground.join(".gitkeep"), "").unwrap();
        run_git(&self.playground, &["add", "."]);
        run_git(&self.playground, &["commit", "-m", "init"]);
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
        let _ = fs::remove_dir_all(base.join("artefacts-remote").join(&self.name));
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
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
