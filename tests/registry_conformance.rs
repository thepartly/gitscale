//! Against a real registry, on request: what the fake registry might get
//! wrong about the Distribution spec — upload locations, media types,
//! `Docker-Content-Digest` — and whether other OCI tools read what gitscale
//! publishes, and the other way round.
//!
//! Skipped unless `GITSCALE_TEST_REGISTRY` names one, as `host:port`. It is
//! spoken to over plain HTTP, as `registry:2` serves:
//!
//! ```text
//! docker compose up -d registry
//! GITSCALE_TEST_REGISTRY=localhost:5000 cargo test --test registry_conformance   # on the host
//! GITSCALE_TEST_REGISTRY=registry:5000 cargo test --test registry_conformance    # in the devbox
//! ```
//!
//! The interop tests also need `skopeo` on PATH, and skip without it.

#[allow(dead_code)]
mod helpers;

use helpers::{git_stdout, TestEnv};
use std::path::Path;
use std::process::Command;

/// The registry to test against, as `host:port`, or `None` to skip.
fn registry() -> Option<String> {
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
fn use_registry(env: &TestEnv, addr: &str) {
    env.use_registry(&format!("http://{}", addr));
}

/// A repository name no earlier run used: a real registry keeps what it is
/// given between runs.
fn unique(name: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{}{}", name, nanos % 1_000_000_000)
}

fn entry_config(env: &TestEnv, bare: &Path) -> String {
    format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"main\", artefact = \"replace\" }}\n",
        env.registries(),
        bare.display()
    )
}

const LAYERED: &str = "[artefact]\nroot = \"dist\"\n\n\
                       [[artefact.layer]]\nname = \"vendor\"\ninclude = [\"vendor/**\"]\n\n\
                       [[artefact.layer]]\nname = \"app\"\ninclude = [\"**\"]\n";

#[test]
fn publish_clone_and_pull_against_a_real_registry() {
    let Some(addr) = registry() else {
        return;
    };
    let env = TestEnv::new("conformance_roundtrip");
    use_registry(&env, &addr);
    let bare = env.create_bare_repo(&unique("app"), "main", &[("README.md", "app")]);

    let files = |app: &'static str| {
        [
            ("vendor/lib.js", "vendor"),
            ("app.js", app),
            ("bin/tool", "#!/bin/sh\n"),
        ]
    };
    let (out, _) = env.publish_with(&bare, "main", LAYERED, &files("v1"), &[]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    // Publishing the same files again is recognised as the same image.
    let (again, _) = env.publish_with(&bare, "main", LAYERED, &files("v1"), &[]);
    assert!(again.success, "{}", again.stderr);
    assert!(
        again.stdout.contains("Already published"),
        "{}",
        again.stdout
    );

    env.write_config(&entry_config(&env, &bare));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        std::fs::read_to_string(env.playground.join("meta/app/app.js")).unwrap(),
        "v1"
    );

    env.push_commit(&bare, "main", "README.md", "v2");
    let (out, _) = env.publish_with(&bare, "main", LAYERED, &files("v2"), &[]);
    assert!(out.success, "{}", out.stderr);
    // The vendor layer is the one from the first publish.
    assert!(
        out.stdout.contains("vendor already in the registry"),
        "{}",
        out.stdout
    );
    assert!(env.run(&["fetch"]).success);
    assert!(env.run(&["pull"]).success);
    assert_eq!(
        std::fs::read_to_string(env.playground.join("meta/app/app.js")).unwrap(),
        "v2"
    );

    // The registry's tag list and manifests, as `list` and `show` read them.
    let listed = env.run(&["artefact", "list"]);
    assert!(listed.success, "{}", listed.stderr);
    assert!(
        listed
            .stdout
            .lines()
            .next()
            .unwrap()
            .ends_with("(2 images)"),
        "{}",
        listed.stdout
    );
    assert!(
        listed.stdout.contains("main  (installed)"),
        "{}",
        listed.stdout
    );
    let shown = env.run(&["artefact", "show"]);
    assert!(shown.success, "{}", shown.stderr);
    assert!(shown.stdout.contains("status     ok"), "{}", shown.stdout);
}

#[test]
fn a_missing_tag_is_reported_by_a_real_registry() {
    let Some(addr) = registry() else {
        return;
    };
    let env = TestEnv::new("conformance_missing");
    use_registry(&env, &addr);
    let bare = env.create_bare_repo(&unique("unpublished"), "main", &[("README.md", "x")]);
    env.write_config(&entry_config(&env, &bare));
    let out = env.run(&["pull"]);
    assert!(!out.success);
    assert!(out.stderr.contains("no artefact for"), "{}", out.stderr);
}

fn skopeo() -> bool {
    let found = Command::new("skopeo")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("skopeo is not on PATH; skipping");
    }
    found
}

fn run(program: &str, args: &[&str]) -> String {
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

#[test]
fn other_tools_read_what_gitscale_publishes() {
    let Some(addr) = registry() else {
        return;
    };
    if !skopeo() {
        return;
    }
    let env = TestEnv::new("conformance_interop_out");
    use_registry(&env, &addr);
    let bare = env.create_bare_repo(&unique("out"), "main", &[("README.md", "x")]);
    let (out, commit) = env.publish_with(
        &bare,
        "main",
        LAYERED,
        &[("vendor/lib.js", "vendor"), ("app.js", "app")],
        &[],
    );
    assert!(out.success, "{}", out.stderr);

    let reference = format!("docker://{}/{}:{}", addr, env.image(&bare), commit);
    let inspected = run("skopeo", &["inspect", "--tls-verify=false", &reference]);
    let inspected: serde_json::Value = serde_json::from_str(&inspected).unwrap();
    assert_eq!(
        inspected["Layers"].as_array().unwrap().len(),
        2,
        "{}",
        inspected
    );
    // And the whole image copies out as a standard layout.
    let layout = env.repos_remote.join("copied-layout");
    run(
        "skopeo",
        &[
            "copy",
            "--src-tls-verify=false",
            &reference,
            &format!("oci:{}:copy", layout.display()),
        ],
    );
    assert!(layout.join("index.json").is_file());
}

#[test]
fn gitscale_reads_what_other_tools_publish() {
    let Some(addr) = registry() else {
        return;
    };
    if !skopeo() {
        return;
    }
    let env = TestEnv::new("conformance_interop_in");
    use_registry(&env, &addr);
    let source = env.create_bare_repo(&unique("source"), "main", &[("README.md", "s")]);
    let (out, commit) = env.publish_with(
        &source,
        "main",
        "[artefact]\nroot = \"dist\"\ninclude = [\"**\"]\n",
        &[("hello.txt", "copied by skopeo")],
        &[],
    );
    assert!(out.success, "{}", out.stderr);

    // Another repository's artefact, written by skopeo rather than gitscale:
    // the image above, copied to the name and tag gitscale will look for.
    let target = env.create_bare_repo(&unique("target"), "main", &[("README.md", "t")]);
    let target_commit = git_stdout(&target, &["rev-parse", "main"]);
    run(
        "skopeo",
        &[
            "copy",
            "--src-tls-verify=false",
            "--dest-tls-verify=false",
            &format!("docker://{}/{}:{}", addr, env.image(&source), commit),
            &format!("docker://{}/{}:{}", addr, env.image(&target), target_commit),
        ],
    );

    env.write_config(&entry_config(&env, &target));
    let out = env.run(&["pull"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        std::fs::read_to_string(env.playground.join("meta/app/hello.txt")).unwrap(),
        "copied by skopeo"
    );
}
