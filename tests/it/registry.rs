//! Talking to registries: credentials and refusals against the fake registry,
//! and conformance against a real one.
//!
//! The conformance tests check what the fake registry might get wrong about
//! the Distribution spec — upload locations, media types,
//! `Docker-Content-Digest` — and whether other OCI tools read what gitscale
//! publishes, and the other way round. They are skipped unless
//! `GITSCALE_TEST_REGISTRY` names a registry, as `host:port`. It is spoken to
//! over plain HTTP, as `registry:2` serves:
//!
//! ```text
//! docker compose up -d registry
//! GITSCALE_TEST_REGISTRY=localhost:5000 cargo test --test it registry::   # on the host
//! GITSCALE_TEST_REGISTRY=registry:5000 cargo test --test it registry::    # in the devbox
//! ```
//!
//! The interop tests also need `skopeo` on PATH, and skip without it.

use crate::support;
use crate::support::artefacts::entry_config;
use crate::support::artefacts::*;
use crate::support::real_registry::*;
use crate::support::registry_access::*;
use crate::support::TestEnv;

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_a_ci_job_logs_in_with_its_job_token() {
    let env = TestEnv::new("art_job_token");
    let bare = env.artefact_repo("app", &[("app.bin", "from ci")]);
    env.registry().require_auth(support::registry::Auth {
        user: "gitlab-ci-token".into(),
        password: "job-token-value".into(),
        realm_host: "127.0.0.1".into(),
    });
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));

    let vars = gitlab_job(&env);
    let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let out = env.run_with_env(&vars, &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "from ci");
    assert!(
        env.registry().count("GET", "/token [auth]") > 0,
        "{:?}",
        env.registry().log()
    );
}

#[test]
fn normal_002_a_docker_login_is_used_outside_ci() {
    let env = TestEnv::new("art_docker_login");
    let bare = env.artefact_repo("app", &[("app.bin", "logged in")]);
    env.registry().require_auth(support::registry::Auth {
        user: "dev".into(),
        password: "personal-token".into(),
        realm_host: "127.0.0.1".into(),
    });
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = env.repos_remote.join("docker-config");
    std::fs::create_dir_all(&docker).unwrap();
    let auth = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        "dev:personal-token",
    );
    std::fs::write(
        docker.join("config.json"),
        format!(
            r#"{{"auths": {{"{}": {{"auth": "{}"}}}}}}"#,
            env.registry().addr,
            auth
        ),
    )
    .unwrap();

    let without = env.run_with_env(&[], &["sync"]);
    assert!(!without.success, "no login, no access");
    let out = env.run_with_env(&[("DOCKER_CONFIG", docker.to_str().unwrap())], &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "logged in");
}

#[test]
fn normal_003_publish_and_sync_against_a_real_registry() {
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
    support::real_registry::release(&bare, "v1.0.0");
    let (out, _) = env.publish_with(
        &bare,
        "main",
        support::real_registry::LAYERED,
        &files("v1"),
        &["v1.0.0"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    // Publishing the same files again is recognised as the same image.
    let (again, _) = env.publish_with(
        &bare,
        "main",
        support::real_registry::LAYERED,
        &files("v1"),
        &["v1.0.0"],
    );
    assert!(again.success, "{}", again.stderr);
    assert!(
        again.stdout.contains("Already published"),
        "{}",
        again.stdout
    );

    env.write_config(&support::real_registry::entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        std::fs::read_to_string(env.playground.join("meta/app/dist/app.js")).unwrap(),
        "v1"
    );
    // The producer's config, from the manifest, at the top of the checkout.
    assert!(
        std::fs::read_to_string(env.playground.join("meta/app/.gitscale.toml"))
            .unwrap()
            .ends_with(support::real_registry::LAYERED)
    );

    env.push_commit(&bare, "main", "README.md", "v2");
    support::real_registry::release(&bare, "v1.1.0");
    let (out, _) = env.publish_with(
        &bare,
        "main",
        support::real_registry::LAYERED,
        &files("v2"),
        &["v1.1.0"],
    );
    assert!(out.success, "{}", out.stderr);
    // The vendor layer is the one from the first publish.
    assert!(
        out.stdout.contains("vendor already in the registry"),
        "{}",
        out.stdout
    );
    env.write_config(&support::real_registry::entry_config(&env, &bare, "v1.1.0"));
    assert!(env.run(&["fetch"]).success);
    assert!(env.run(&["sync"]).success);
    assert_eq!(
        std::fs::read_to_string(env.playground.join("meta/app/dist/app.js")).unwrap(),
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
            .ends_with("(2 releases)"),
        "{}",
        listed.stdout
    );
    assert!(
        listed
            .stdout
            .lines()
            .any(|l| l.trim_start().starts_with("v1.1.0 ") && l.ends_with("(installed)")),
        "{}",
        listed.stdout
    );
    let shown = env.run(&["artefact", "show"]);
    assert!(shown.success, "{}", shown.stderr);
    assert!(shown.stdout.contains("status     ok"), "{}", shown.stdout);
}

#[test]
fn normal_004_other_tools_read_what_gitscale_publishes() {
    let Some(addr) = registry() else {
        return;
    };
    if !skopeo() {
        return;
    }
    let env = TestEnv::new("conformance_interop_out");
    use_registry(&env, &addr);
    let bare = env.create_bare_repo(&unique("out"), "main", &[("README.md", "x")]);
    support::real_registry::release(&bare, "v1.0.0");
    let (out, _) = env.publish_with(
        &bare,
        "main",
        support::real_registry::LAYERED,
        &[("vendor/lib.js", "vendor"), ("app.js", "app")],
        &["v1.0.0"],
    );
    assert!(out.success, "{}", out.stderr);

    let reference = format!("docker://{}/{}:v1.0.0", addr, env.image(&bare));
    let inspected = run("skopeo", &["inspect", "--tls-verify=false", &reference]);
    let inspected: serde_json::Value = serde_json::from_str(&inspected).unwrap();
    // The two groups, and nothing else: the config is in the manifest.
    assert_eq!(
        inspected["Layers"].as_array().unwrap().len(),
        2,
        "{}",
        inspected
    );
    let raw = run(
        "skopeo",
        &["inspect", "--raw", "--tls-verify=false", &reference],
    );
    let raw: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(
        raw["annotations"]["dev.gitscale.config"]
            .as_str()
            .unwrap()
            .ends_with(support::real_registry::LAYERED),
        "{}",
        raw
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
fn normal_005_gitscale_reads_what_other_tools_publish() {
    let Some(addr) = registry() else {
        return;
    };
    if !skopeo() {
        return;
    }
    let env = TestEnv::new("conformance_interop_in");
    use_registry(&env, &addr);
    let source = env.create_bare_repo(&unique("source"), "main", &[("README.md", "s")]);
    support::real_registry::release(&source, "v1.0.0");
    let (out, _) = env.publish_with(
        &source,
        "main",
        "[artefact]\ninclude = [\"dist/**\"]\n",
        &[("hello.txt", "copied by skopeo")],
        &["v1.0.0"],
    );
    assert!(out.success, "{}", out.stderr);

    // Another repository's artefact, written by skopeo rather than gitscale:
    // the image above, copied to the name and release gitscale will look
    // for, as a Docker image — whose manifest has no annotations, so no
    // config either.
    let target = env.create_bare_repo(&unique("target"), "main", &[("README.md", "t")]);
    support::real_registry::release(&target, "v1.0.0");
    run(
        "skopeo",
        &[
            "copy",
            "--format",
            "v2s2",
            "--src-tls-verify=false",
            "--dest-tls-verify=false",
            &format!("docker://{}/{}:v1.0.0", addr, env.image(&source)),
            &format!("docker://{}/{}:v1.0.0", addr, env.image(&target)),
        ],
    );

    env.write_config(&support::real_registry::entry_config(
        &env, &target, "v1.0.0",
    ));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        std::fs::read_to_string(env.playground.join("meta/app/dist/hello.txt")).unwrap(),
        "copied by skopeo"
    );
    // No config to write: the image declares no dependencies.
    assert!(!env.playground.join("meta/app/.gitscale.toml").exists());
}

/// A registry that challenges with `Basic` rather than a token service gets
/// the stored login directly, as `docker` does.
#[test]
fn normal_009_a_basic_challenge_is_answered_with_the_stored_login() {
    let env = TestEnv::new("registry_basic_login");
    let bare = env.artefact_repo("app", &[("app.bin", "basic")]);
    env.registry().require_auth(auth("dev", "personal-token"));
    env.registry().use_basic_auth();
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_login(&env, &env.registry().addr, "dev", "personal-token");

    let out = env.run_with_env(&[("DOCKER_CONFIG", s(&docker))], &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "basic");
    assert!(env
        .registry()
        .seen()
        .iter()
        .any(|r| r.authorization.as_deref()
            == Some(&format!("Basic {}", encoded("dev", "personal-token")))));
}

/// What `docker login` stores for some registries is an OAuth2 refresh
/// token (`identitytoken`), which gitscale exchanges at the token service
/// for an access token rather than sending as a password.
#[test]
fn normal_010_a_stored_identity_token_is_exchanged_for_a_bearer_token() {
    let env = TestEnv::new("registry_identity_token");
    let bare = env.artefact_repo("app", &[("app.bin", "refreshed")]);
    env.registry().require_auth(auth("unused", "unused"));
    env.registry().accept_refresh_token("refresh-me");
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_config(
        &env,
        "docker-config",
        &format!(
            r#"{{"auths": {{"{}": {{"identitytoken": "refresh-me"}}}}}}"#,
            env.registry().addr
        ),
    );

    let out = env.run_with_env(&[("DOCKER_CONFIG", s(&docker))], &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "refreshed");
    assert!(
        env.registry().count("POST", "/token") > 0,
        "{:?}",
        env.registry().log()
    );
}

/// A login kept by a credential helper (`credsStore`, as Docker Desktop and
/// `pass` set up) is asked for by running `docker-credential-<name> get`
/// with the registry on its input.
#[test]
fn normal_011_a_credential_helper_is_asked_for_the_registry() {
    let env = TestEnv::new("registry_credential_helper");
    let bare = env.artefact_repo("app", &[("app.bin", "from a helper")]);
    env.registry().require_auth(auth("dev", "helper-secret"));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_config(&env, "docker-config", r#"{"credsStore": "gitscaletest"}"#);
    let bin = env.repos_remote.join("helper-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let helper = bin.join("docker-credential-gitscaletest");
    std::fs::write(
        &helper,
        format!(
            "#!/bin/sh\nread registry\nif [ \"$registry\" = \"{}\" ]; then\n  echo '{{\"Username\":\"dev\",\"Secret\":\"helper-secret\"}}'\nelse\n  exit 1\nfi\n",
            env.registry().addr
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let out = env.run_with_env(&[("DOCKER_CONFIG", s(&docker)), ("PATH", &path)], &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "from a helper");
}

/// With no Docker login, Podman's auth file (`REGISTRY_AUTH_FILE`) is read:
/// `podman login` and `oras login` write it.
#[test]
fn normal_012_a_podman_auth_file_is_read_when_docker_has_no_login() {
    let env = TestEnv::new("registry_podman_auth");
    let bare = env.artefact_repo("app", &[("app.bin", "podman")]);
    env.registry().require_auth(auth("dev", "podman-token"));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let file = env.repos_remote.join("auth.json");
    std::fs::write(
        &file,
        format!(
            r#"{{"auths": {{"{}": {{"auth": "{}"}}}}}}"#,
            env.registry().addr,
            encoded("dev", "podman-token")
        ),
    )
    .unwrap();

    let out = env.run_with_env(&[("REGISTRY_AUTH_FILE", s(&file))], &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "podman");
}

/// A token service may hand the token out as `access_token` (the OAuth2
/// name) rather than `token`.
#[test]
fn normal_013_a_token_named_access_token_is_used() {
    let env = TestEnv::new("registry_access_token_field");
    let bare = env.artefact_repo("app", &[("app.bin", "oauth")]);
    env.registry().require_auth(auth("dev", "pw"));
    env.registry().token_field("access_token");
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_login(&env, &env.registry().addr, "dev", "pw");

    let out = env.run_with_env(&[("DOCKER_CONFIG", s(&docker))], &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "oauth");
}

/// `publish` to a registry that needs a login asks the token service for a
/// token that may push (`pull,push` on the image's repository), with the
/// stored login, and publishes.
#[test]
fn normal_014_publish_logs_in_for_push_access() {
    let env = TestEnv::new("registry_publish_login");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    env.registry().require_auth(auth("dev", "pw"));
    let docker = docker_login(&env, &env.registry().addr, "dev", "pw");
    let producer = producer_with_build(&env, &bare);

    let out = run_bin_in(
        &env,
        &producer,
        &[("DOCKER_CONFIG", s(&docker))],
        &["artefact", "publish"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(hash_tags(env.registry().tags("app/gitscale")).len(), 1);
    let push_scope = url::form_urlencoded::byte_serialize(b"repository:app/gitscale:pull,push")
        .collect::<String>();
    assert!(
        env.registry()
            .seen()
            .iter()
            .any(|r| r.path == "/token" && r.query.contains(&format!("scope={}", push_scope))),
        "{:?}",
        env.registry().seen()
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn edge_006_the_job_token_never_goes_to_a_token_service_elsewhere() {
    let env = TestEnv::new("art_foreign_realm");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    // The registry sends clients to a token service on another host name.
    env.registry().require_auth(support::registry::Auth {
        user: "gitlab-ci-token".into(),
        password: "job-token-value".into(),
        realm_host: "localhost".into(),
    });
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));

    let vars = gitlab_job(&env);
    let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let out = env.run_with_env(&vars, &["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("Job token permissions"),
        "{}",
        out.stderr
    );
    assert!(env.registry().count("GET", "/token") > 0);
    assert_eq!(
        env.registry().count("GET", "/token [auth]"),
        0,
        "the token went to a host the CI server does not own: {:?}",
        env.registry().log()
    );
}

/// A registry spoken to over plain HTTP, off this machine, is used
/// anonymously, as the docs promise: no credential crosses the network in
/// the clear. That covers the token a token service hands out for a stored
/// login, not only the login itself.
#[test]
#[ignore = "bug: a Bearer token is sent to a plain-HTTP registry off this machine"]
fn edge_015_a_plain_http_registry_off_this_machine_gets_no_bearer_token() {
    let env = TestEnv::new("registry_plain_http_bearer");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    // The token service is on this machine; the registry is not.
    env.registry().require_auth(auth("dev", "pw"));
    let remote = elsewhere();
    remote.mirror_from(env.registry());
    remote.require_auth(auth("dev", "pw"));
    remote.set_realm(&format!("http://{}/token", env.registry().addr));
    env.use_registry(&format!("http://{}", remote.addr));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_login(&env, &remote.addr, "dev", "pw");

    let mut vars = vec![("DOCKER_CONFIG", s(&docker))];
    vars.extend(NO_PROXY);
    let _ = env.run_with_env(&vars, &["sync"]);
    assert!(!remote.seen().is_empty(), "the registry was never asked");
    let sent: Vec<_> = remote
        .seen()
        .into_iter()
        .filter(|r| r.authorization.is_some())
        .collect();
    assert!(
        sent.is_empty(),
        "credentials went over plain HTTP: {:?}",
        sent
    );
}

/// The same registry answering with a `Basic` challenge gets no stored
/// login: that would be the password itself, in the clear.
#[test]
fn edge_016_a_plain_http_registry_off_this_machine_gets_no_basic_login() {
    let env = TestEnv::new("registry_plain_http_basic");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    let remote = elsewhere();
    remote.mirror_from(env.registry());
    remote.require_auth(auth("dev", "pw"));
    remote.use_basic_auth();
    env.use_registry(&format!("http://{}", remote.addr));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_login(&env, &remote.addr, "dev", "pw");

    let mut vars = vec![("DOCKER_CONFIG", s(&docker))];
    vars.extend(NO_PROXY);
    let out = env.run_with_env(&vars, &["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(!remote.seen().is_empty(), "the registry was never asked");
    assert!(
        remote.seen().iter().all(|r| r.authorization.is_none()),
        "{:?}",
        remote.seen()
    );
}

/// The CI job token goes only to the registry the CI server owns
/// (`CI_REGISTRY`): another registry gets nothing, even one whose token
/// service is on its own host.
#[test]
fn edge_017_the_job_token_never_goes_to_a_registry_the_ci_server_does_not_own() {
    let env = TestEnv::new("registry_job_token_other_registry");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.registry()
        .require_auth(auth("gitlab-ci-token", "job-token-value"));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let port = env.registry().addr.rsplit(':').next().unwrap().to_string();
    let vars = gitlab_vars(&format!("127.0.0.1:{}", port), "registry.ci.example:5050");

    let out = env.run_with_env(&borrowed(&vars), &["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert_eq!(
        env.registry().count("GET", "/token [auth]"),
        0,
        "the job token went to a registry the CI server does not own: {:?}",
        env.registry().log()
    );
    assert!(!out.stderr.contains("job-token-value"), "{}", out.stderr);
}

/// An upload location on another host gets none of the registry's
/// credentials: the token the registry's token service issued is for the
/// registry.
#[test]
#[ignore = "bug: the cached Authorization header goes to whatever URL the upload location names"]
fn edge_018a_an_upload_location_on_another_host_gets_no_token() {
    let env = TestEnv::new("registry_upload_elsewhere_token");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    env.registry().require_auth(auth("dev", "pw"));
    let storage = elsewhere();
    storage.accept_any_upload();
    env.registry()
        .upload_to(&format!("http://{}", storage.addr));
    let docker = docker_login(&env, &env.registry().addr, "dev", "pw");
    let producer = producer_with_build(&env, &bare);

    let mut vars = vec![("DOCKER_CONFIG", s(&docker))];
    vars.extend(NO_PROXY);
    let out = run_bin_in(&env, &producer, &vars, &["artefact", "publish"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(
        storage.count("PUT", "/blobs/uploads/") > 0,
        "{:?}",
        storage.log()
    );
    let sent: Vec<_> = storage
        .seen()
        .into_iter()
        .filter(|r| r.authorization.is_some())
        .collect();
    assert!(sent.is_empty(), "{:?}", sent);
}

/// The same upload location, asking for a `Basic` login, never gets the CI
/// job token: it goes only to the registry the CI server owns.
#[test]
#[ignore = "bug: the job token is sent to an upload location on another host"]
fn edge_018b_an_upload_location_on_another_host_never_gets_the_job_token() {
    let env = TestEnv::new("registry_upload_elsewhere_job_token");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    env.registry()
        .require_auth(auth("gitlab-ci-token", "job-token-value"));
    env.registry().use_basic_auth();
    let storage = elsewhere();
    storage.accept_any_upload();
    storage.require_auth(auth("someone", "else"));
    storage.use_basic_auth();
    env.registry()
        .upload_to(&format!("http://{}", storage.addr));
    let producer = producer_with_build(&env, &bare);
    let port = env.registry().addr.rsplit(':').next().unwrap().to_string();
    let mut vars = gitlab_vars(&format!("127.0.0.1:{}", port), &env.registry().addr);
    vars.push(("GITLAB_CI", "true".to_string()));
    let mut vars = borrowed(&vars);
    vars.extend(NO_PROXY);

    let _ = run_bin_in(&env, &producer, &vars, &["artefact", "publish"]);
    assert!(
        storage.count("PUT", "/blobs/uploads/") > 0,
        "{:?}",
        storage.log()
    );
    let job = format!("Basic {}", encoded("gitlab-ci-token", "job-token-value"));
    assert!(
        storage
            .seen()
            .iter()
            .all(|r| r.authorization.as_deref() != Some(job.as_str())),
        "the job token went to {}: {:?}",
        storage.addr,
        storage.seen()
    );
}

/// A tag list whose next page is on another host gets none of the
/// registry's credentials there.
#[test]
#[ignore = "bug: the cached Authorization header goes to whatever URL the tag list's next page names"]
fn edge_019_a_tag_list_page_on_another_host_gets_no_token() {
    let env = TestEnv::new("registry_tags_elsewhere");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    env.publish(&bare, "main", &[("app.bin", "v1")]);
    env.push_commit(&bare, "main", "README.md", "v2");
    env.publish(&bare, "main", &[("app.bin", "v2")]);
    env.push_commit(&bare, "main", "README.md", "v3");
    env.publish(&bare, "main", &[("app.bin", "v3")]);
    env.registry().require_auth(auth("dev", "pw"));
    let other = elsewhere();
    other.mirror_from(env.registry());
    env.registry()
        .tags_next_page_on(&format!("http://{}", other.addr));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_login(&env, &env.registry().addr, "dev", "pw");

    let mut vars = vec![("DOCKER_CONFIG", s(&docker))];
    vars.extend(NO_PROXY);
    let out = env.run_with_env(&vars, &["artefact", "list"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(other.count("GET", "/tags/list") > 0, "{:?}", other.log());
    assert!(
        other.seen().iter().all(|r| r.authorization.is_none()),
        "{:?}",
        other.seen()
    );
}

/// A blob download the registry redirects to another host — storage or a
/// CDN, as most registries do — carries no `Authorization` there.
#[test]
fn edge_020_a_blob_redirect_to_another_host_carries_no_authorization() {
    let env = TestEnv::new("registry_blob_redirect");
    let bare = env.artefact_repo("app", &[("app.bin", "redirected")]);
    env.registry().require_auth(auth("dev", "pw"));
    let cdn = elsewhere();
    cdn.mirror_from(env.registry());
    env.registry()
        .redirect_blobs_to(&format!("http://{}", cdn.addr));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_login(&env, &env.registry().addr, "dev", "pw");

    let mut vars = vec![("DOCKER_CONFIG", s(&docker))];
    vars.extend(NO_PROXY);
    let out = env.run_with_env(&vars, &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "redirected");
    assert!(cdn.count("GET", "/blobs/") > 0, "{:?}", cdn.log());
    assert!(
        cdn.seen().iter().all(|r| r.authorization.is_none()),
        "{:?}",
        cdn.seen()
    );
}

/// Not every registry sends `Docker-Content-Digest`: without it, the digest
/// is read from the manifest itself, and the sync works the same.
#[test]
fn edge_021_a_manifest_without_a_content_digest_header_is_digested_locally() {
    let env = TestEnv::new("registry_no_digest_header");
    let bare = env.artefact_repo("app", &[("app.bin", "digested")]);
    env.registry().omit_manifest_digest();
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(read(&env, "meta/app/dist/app.bin"), "digested");
    assert!(
        env.registry().count("GET", "/manifests/v1.0.0") > 0,
        "{:?}",
        env.registry().log()
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_007_a_refusal_says_how_to_get_access() {
    let env = TestEnv::new("art_forbidden");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.registry().refuse_with(403);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success);
    assert!(out.stderr.contains("refused"), "{}", out.stderr);
    assert!(out.stderr.contains("docker login"), "{}", out.stderr);
}

#[test]
fn error_008_a_missing_tag_is_reported_by_a_real_registry() {
    let Some(addr) = registry() else {
        return;
    };
    let env = TestEnv::new("conformance_missing");
    use_registry(&env, &addr);
    let bare = env.create_bare_repo(&unique("unpublished"), "main", &[("README.md", "x")]);
    support::real_registry::release(&bare, "v1.0.0");
    env.write_config(&support::real_registry::entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success);
    assert!(out.stderr.contains("no artefact for"), "{}", out.stderr);
}

/// A registry that answers with a server error or a rate limit is reported
/// with the status and what it said, and nothing is installed.
#[test]
fn error_022_registry_errors_are_reported_with_their_status() {
    for status in [429u16, 500, 502] {
        let env = TestEnv::new(&format!("registry_status_{}", status));
        let bare = env.artefact_repo("app", &[("app.bin", "x")]);
        env.registry()
            .fail("HEAD", "/manifests/", status, "try again later");
        env.registry()
            .fail("GET", "/manifests/", status, "try again later");
        env.write_config(&entry_config(&env, &bare, "v1.0.0"));
        let out = env.run(&["sync"]);
        assert!(!out.success, "{}: {}", status, out.stdout);
        assert!(
            out.stderr
                .contains(&format!("answered {} for app/gitscale", status)),
            "{}: {}",
            status,
            out.stderr
        );
        assert!(!env.playground.join("meta/app/dist").exists());
    }
}

/// A stored login the token service refuses ends in a refusal that says how
/// to log in, and never shows the password.
#[test]
fn error_023_a_refused_login_says_how_to_log_in_and_hides_the_password() {
    let env = TestEnv::new("registry_refused_login");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.registry().require_auth(auth("dev", "right-password"));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_login(&env, &env.registry().addr, "dev", "wrong-password");

    let out = env.run_with_env(&[("DOCKER_CONFIG", s(&docker))], &["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("refused to let gitscale read"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("docker login"), "{}", out.stderr);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(!text.contains("wrong-password"), "{}", text);
    assert!(
        !text.contains(&encoded("dev", "wrong-password")),
        "{}",
        text
    );
}

/// What a registry says back in an error is shown to help — but never a
/// credential it echoes: a job log would keep the job token, base64-encoded
/// where CI masking does not recognise it.
#[test]
#[ignore = "bug: error bodies are printed verbatim, credentials the registry echoes included"]
fn error_024_registry_errors_never_print_credentials() {
    let env = TestEnv::new("registry_error_echo");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.registry()
        .require_auth(auth("gitlab-ci-token", "job-token-value"));
    env.registry().use_basic_auth();
    // A blob download: a response with a body (a HEAD has none).
    env.registry()
        .fail_authorized_echoing_headers("GET", "/blobs/", 500);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let port = env.registry().addr.rsplit(':').next().unwrap().to_string();
    let vars = gitlab_vars(&format!("127.0.0.1:{}", port), &env.registry().addr);

    let out = env.run_with_env(&borrowed(&vars), &["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("answered 500"), "{}", out.stderr);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(!text.contains("job-token-value"), "{}", text);
    assert!(
        !text.contains(&encoded("gitlab-ci-token", "job-token-value")),
        "the job token is in the output, base64-encoded: {}",
        text
    );
}

/// A GitLab job token refused for a push says the one thing that can fix
/// it: the job token publishes only to its own project's registry.
#[test]
fn error_025_a_refused_publish_in_ci_says_what_the_job_token_may_do() {
    let env = TestEnv::new("registry_publish_refused_ci");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    env.registry()
        .require_auth(auth("gitlab-ci-token", "job-token-value"));
    env.registry()
        .fail_authorized("POST", "/blobs/uploads/", 403, "");
    let producer = producer_with_build(&env, &bare);
    let port = env.registry().addr.rsplit(':').next().unwrap().to_string();
    let mut vars = gitlab_vars(&format!("127.0.0.1:{}", port), &env.registry().addr);
    vars.push(("GITLAB_CI", "true".to_string()));

    let out = run_bin_in(&env, &producer, &borrowed(&vars), &["artefact", "publish"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("refused to let gitscale publish to app/gitscale (403)"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("can publish only to its own project's registry"),
        "{}",
        out.stderr
    );
    assert!(env.registry().tags("app/gitscale").is_empty());
}

/// A registry whose manifest does not match the digest it claims for it is
/// not believed: the sync fails, and nothing is installed.
#[test]
fn error_026_a_manifest_not_matching_its_digest_is_refused() {
    let env = TestEnv::new("registry_manifest_digest_lie");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    let claimed = format!("sha256:{}", "c".repeat(64));
    env.registry()
        .lie_about_manifest(&env.image(&bare), "v1.0.0", &claimed);
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains(&format!(
            "served a manifest that does not match its digest {}",
            claimed
        )),
        "{}",
        out.stderr
    );
    assert!(!env.playground.join("meta/app/dist").exists());
}

/// A layer download cut off part way fails the sync, leaves no partial file
/// in the image store for a reader to find, and leaves the installed
/// version alone.
#[test]
fn error_027_a_cut_off_download_leaves_no_partial_file_and_the_old_install() {
    let env = TestEnv::new("registry_truncated_blob");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    assert!(env.run(&["sync"]).success);

    env.push_commit(&bare, "main", "README.md", "v2");
    layered(
        &env,
        &bare,
        "v1.1.0",
        "app v2 with a longer body to cut in half",
    );
    let app_layer = layer_digests(&env, &bare, "v1.1.0")[1].clone();
    env.registry().truncate_blob(&app_layer);
    env.write_config(&entry_config(&env, &bare, "v1.1.0"));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert_eq!(read(&env, "meta/app/dist/app.js"), "app v1");
    assert_eq!(installed_tag(&env), "v1.0.0");
    let leftovers: Vec<String> = all_names(&image_store(&env))
        .into_iter()
        .filter(|n| n.contains("partial"))
        .collect();
    assert!(leftovers.is_empty(), "{:?}", leftovers);
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

/// Tokens are kept for the length of a command: a sync asks the token
/// service once for its one scope, however many manifests and blobs it
/// reads; a publish once for reading and once for pushing.
#[test]
fn perf_028_one_token_exchange_per_scope_per_command() {
    let env = TestEnv::new("registry_token_reuse");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    layered(&env, &bare, "v1.0.0", "app v1");
    env.registry().require_auth(auth("dev", "pw"));
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    let docker = docker_login(&env, &env.registry().addr, "dev", "pw");

    env.registry().clear_log();
    let out = env.run_with_env(&[("DOCKER_CONFIG", s(&docker))], &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(blob_downloads(&env) >= 2, "{:?}", env.registry().log());
    assert_eq!(
        env.registry().count("GET", "/token"),
        1,
        "{:?}",
        env.registry().log()
    );

    env.push_commit(&bare, "main", "README.md", "v2");
    let producer = producer_with_build(&env, &bare);
    env.registry().clear_log();
    let out = run_bin_in(
        &env,
        &producer,
        &[("DOCKER_CONFIG", s(&docker))],
        &["artefact", "publish"],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        env.registry().count("GET", "/token"),
        2,
        "{:?}",
        env.registry().log()
    );
}

/// A registry that names a next page of its tag list forever is not
/// followed forever: `artefact list` stops after a thousand pages and says
/// why.
#[test]
fn perf_029_a_tag_list_that_never_ends_is_cut_off() {
    let env = TestEnv::new("registry_endless_tags");
    let bare = env.artefact_repo("app", &[("app.bin", "x")]);
    env.registry().endless_tags();
    env.write_config(&entry_config(&env, &bare, "v1.0.0"));
    env.registry().clear_log();
    let out = env.run(&["artefact", "list"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr.contains("kept paginating its tag list"),
        "{}",
        out.stderr
    );
    assert_eq!(env.registry().count("GET", "/tags/list"), 1000);
}
