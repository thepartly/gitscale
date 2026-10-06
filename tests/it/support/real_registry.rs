//! Helpers for the real registry tests.

use super::TestEnv;
use std::path::Path;
use std::process::Command;
/// The registry to test against, as `host:port`, or `None` to skip.
pub fn registry() -> Option<String> {
    let addr = std::env::var("GITSCALE_TEST_REGISTRY")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| v.trim_start_matches("http://").to_string());
    if addr.is_none() {
        eprintln!("GITSCALE_TEST_REGISTRY is not set; skipping");
    }
    addr
}

/// Point `env` at the registry, over plain HTTP whatever its host is called.
pub fn use_registry(env: &TestEnv, addr: &str) {
    env.use_registry(&format!("http://{}", addr));
}

/// A repository name no earlier run used: a real registry keeps what it is
/// given between runs.
pub fn unique(name: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{}{}", name, nanos % 1_000_000_000)
}

/// A root whose `meta/app` is `bare` at the release `revision`, taken as an
/// artefact.
pub fn entry_config(env: &TestEnv, bare: &Path, revision: &str) -> String {
    env.prefer(bare, gitscale::prefer::Form::Artefact);
    format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"{}\" }}\n",
        env.registries(),
        bare.display(),
        revision
    )
}

/// Tag `main` of `bare` as the release `tag`, as a pipeline releasing it
/// would have.
pub fn release(bare: &Path, tag: &str) {
    super::run_git_pub(bare, &["tag", tag, "main"]);
}

/// Two groups of the build in `dist/`: `vendor`, then everything else. The
/// config travels in the manifest, so an image has two layers.
pub const LAYERED: &str =
    "[[artefact.layer]]\nname = \"vendor\"\ninclude = [\"dist/vendor/**\"]\n\n\
                       [[artefact.layer]]\nname = \"app\"\ninclude = [\"dist/**\"]\n";

pub fn skopeo() -> bool {
    let found = Command::new("skopeo")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("skopeo is not on PATH; skipping");
    }
    found
}

pub fn run(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "{} {:?}: {}",
        program,
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}
