//! `[hooks] post_sync` commands, and the allowlist that decides whose may run.

use crate::support::workspace::*;
use crate::support::{run_git_pub, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_runs_for_an_allowed_repository() {
    let env = hook_env(
        "hooks_post_sync",
        "https://github.com/thepartly/gitscale.git",
    );
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[hooks]
post_sync = "touch .hook-ran"

[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run_as_hook("github.com/thepartly/*", &["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);

    // Hook should have created this file
    assert!(env.playground.join(".hook-ran").exists());
}

/// A `git scale sync` the user typed is not a drive-by: they chose the
/// directory and the moment, so the allowlist — which belongs to the installed
/// hook — does not apply.
#[test]
fn normal_002_runs_when_the_user_invoked_gitscale() {
    let env = hook_env(
        "hooks_post_sync_runs_when_invoked_directly",
        "https://gitlab.example.com/attacker/payload.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch .hook-ran\"\n");

    let out = env.run_binary_plain(&["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(env.playground.join(".hook-ran").exists());
}

/// `sync` places the checkouts as one of its steps, and the docs promise
/// `post_sync` runs once, not twice: a command that installs or migrates
/// something must not run again on top of itself.
#[test]
fn normal_009_sync_runs_it_exactly_once() {
    let env = TestEnv::new("post_sync_once_per_sync");
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    env.write_config(&format!(
        "[hooks]\npost_sync = \"echo ran >> .post-sync-count\"\n\n[repos]\n\"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
        bare.display()
    ));
    let count = || {
        std::fs::read_to_string(env.playground.join(".post-sync-count"))
            .unwrap_or_default()
            .lines()
            .count()
    };

    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(count(), 1, "sync ran post_sync {} times", count());
    let out = env.run(&["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert_eq!(count(), 2, "pull ran post_sync {} times", count() - 1);
}

/// Patterns are written against `host/owner/repo`, which every spelling of
/// one repository shares: SSH or HTTPS, any case, a default port, a trailing
/// slash, a `.git` suffix, credentials in the URL.
#[test]
fn normal_010_every_spelling_of_an_allowed_repository_is_allowed() {
    let env = hook_env(
        "post_sync_spellings",
        "https://github.com/thepartly/gitscale.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch .hook-ran\"\n");
    for origin in [
        "git@github.com:thepartly/gitscale.git",
        "ssh://git@github.com:22/thepartly/gitscale.git",
        "https://GitHub.com/ThePartly/GitScale",
        "https://github.com:443/thepartly/gitscale",
        "https://github.com/thepartly/gitscale/",
        "https://github.com/thepartly/gitscale.git/",
        "https://someone:secret@github.com/thepartly/gitscale.git",
    ] {
        run_git_pub(&env.playground, &["remote", "set-url", "origin", origin]);
        let _ = std::fs::remove_file(env.playground.join(".hook-ran"));
        let out = env.run_as_hook("github.com/thepartly/gitscale", &["sync"]);
        assert!(out.success, "{}: {}", origin, out.stderr);
        assert!(env.playground.join(".hook-ran").exists(), "{}", origin);
    }
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A repository must not be able to vouch for itself: the allowlist reaches
/// gitscale from the hook shim, never from the `.gitscale.toml` under test.
#[test]
fn edge_003_a_config_cannot_allowlist_itself() {
    let env = hook_env(
        "hooks_post_sync_config_cannot_allowlist_itself",
        "https://gitlab.example.com/attacker/payload.git",
    );

    env.write_config(
        r#"[hooks]
post_sync = "touch pwned"

[trust]
allow = "*"
"#,
    );

    let out = env.run_as_hook("github.com/thepartly/*", &["sync"]);
    assert!(!env.playground.join("pwned").exists());
    assert!(
        out.stderr.contains("refusing to run the post_sync hook"),
        "stderr: {}",
        out.stderr
    );
}

/// An owner pattern ends at the slash, so a lookalike owner is a different
/// owner. This is the whole value of the allowlist.
#[test]
fn edge_004_a_lookalike_owner_is_refused() {
    let env = hook_env(
        "hooks_post_sync_lookalike_owner_is_refused",
        "https://github.com/thepartly-evil/gitscale.git",
    );

    env.write_config("[hooks]\npost_sync = \"touch pwned\"\n");

    let out = env.run_as_hook("github.com/thepartly/*", &["sync"]);
    assert!(!out.success);
    assert!(!env.playground.join("pwned").exists());
    // Refused by the allowlist, not failed for some other reason.
    assert!(
        out.stderr.contains("refusing to run the post_sync hook"),
        "stderr: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains(
            "github.com/thepartly-evil/gitscale is not on this machine's gitscale hook allowlist"
        ),
        "stderr: {}",
        out.stderr
    );
}

/// A workspace with no remote has no host or owner to match on, so only a path
/// pattern can name it.
#[test]
fn edge_005_a_workspace_without_a_remote_matches_on_its_path() {
    let env = TestEnv::new("hooks_post_sync_local_workspace_matches_on_path");
    env.init_playground_git();
    env.write_config("[hooks]\npost_sync = \"touch .hook-ran\"\n");

    let out = env.run_as_hook("github.com/*", &["sync"]);
    assert!(!out.success);
    assert!(!env.playground.join(".hook-ran").exists());
    assert!(
        out.stderr.contains("refusing to run the post_sync hook"),
        "stderr: {}",
        out.stderr
    );

    let pattern = format!("{}/*", env.playground.parent().unwrap().display());
    let out = env.run_as_hook(&pattern, &["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(env.playground.join(".hook-ran").exists());
}

/// A hook installed with an empty allowlist runs nothing at all, rather than
/// reading as "unrestricted".
#[test]
fn edge_006_an_empty_allowlist_refuses_everything() {
    let env = hook_env(
        "hooks_post_sync_empty_allowlist_refuses_everything",
        "https://github.com/thepartly/gitscale.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch pwned\"\n");

    let out = env.run_as_hook("", &["sync"]);
    assert!(!out.success);
    assert!(!env.playground.join("pwned").exists());
    assert!(
        out.stderr.contains("refusing to run the post_sync hook"),
        "stderr: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("The hook currently allows nothing."),
        "stderr: {}",
        out.stderr
    );
}

/// With `-v` an allowed command says which pattern let it through and what it
/// runs — what someone checking an allowlist wants to see.
#[test]
fn edge_011_verbose_output_names_the_pattern_and_the_command() {
    let env = hook_env(
        "post_sync_verbose",
        "https://github.com/thepartly/gitscale.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch .hook-ran\"\n");
    let out = env.run_as_hook("github.com/other/*,github.com/thepartly/*", &["sync", "-v"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains("trust  post_sync allowed by 'github.com/thepartly/*'"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("hook  post_sync: touch .hook-ran"),
        "{}",
        out.stdout
    );
}

/// A blank `post_sync` is no command at all: it is not run, and so there is
/// nothing for even an empty allowlist to refuse.
#[test]
fn edge_012_a_blank_command_runs_nothing_and_is_never_refused() {
    let env = hook_env(
        "post_sync_blank",
        "https://gitlab.example.com/attacker/payload.git",
    );
    env.write_config("[hooks]\npost_sync = \"   \"\n");
    let out = env.run_as_hook("", &["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(!out.stderr.contains("refusing"), "{}", out.stderr);
}

/// A path pattern is documented "for a workspace with no remote", but the
/// workspace's path is matched whatever its remote: a path pattern admits
/// every repository cloned under that directory, whoever it came from. Pins
/// current behaviour; listed for the owner to decide.
#[test]
fn edge_013_a_path_pattern_also_admits_a_workspace_that_has_a_remote() {
    let env = hook_env(
        "post_sync_path_pattern_with_remote",
        "https://gitlab.example.com/attacker/payload.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch .hook-ran\"\n");
    let pattern = format!("{}/*", env.playground.parent().unwrap().display());
    let out = env.run_as_hook(&pattern, &["sync"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join(".hook-ran").exists(),
        "pinned: the path pattern admits a workspace with an unlisted remote"
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_007_a_failing_command_fails_the_sync() {
    let env = hook_env(
        "hooks_post_sync_failure",
        "https://github.com/thepartly/gitscale.git",
    );
    let bare = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);

    env.write_config(&format!(
        r#"[hooks]
post_sync = "exit 1"

[repos]
"libs/mylib" = {{ url = "{}", revision = "main" }}
"#,
        bare.display()
    ));

    let out = env.run_as_hook("*", &["sync"]);
    assert!(!out.success);
    assert!(out.stderr.contains("post_sync hook failed"));
}

/// The reported attack: a branch carries its own `.gitscale.toml`, a developer
/// clones it to review, and the shared hook runs the payload as them. The
/// allowlist the hook was installed with is what has to stop it.
#[test]
fn error_008_is_refused_for_an_unlisted_repository() {
    let env = hook_env(
        "hooks_post_sync_refused_for_unlisted_repo",
        "https://gitlab.example.com/attacker/payload.git",
    );

    env.write_config("[hooks]\npost_sync = \"touch pwned\"\n");

    let out = env.run_as_hook("github.com/thepartly/*", &["sync"]);
    assert!(!out.success);
    assert!(
        !env.playground.join("pwned").exists(),
        "the payload ran despite not being allowlisted"
    );
    assert!(
        out.stderr.contains("refusing to run the post_sync hook"),
        "stderr: {}",
        out.stderr
    );
    // The refusal has to name the repository, or nobody can act on it.
    assert!(
        out.stderr.contains("gitlab.example.com/attacker/payload"),
        "stderr: {}",
        out.stderr
    );
}

/// `post_sync` runs after a sync that worked. When the sync fails it does not
/// run — a build step must not run over checkouts that are not there.
#[test]
fn error_014_is_skipped_after_a_failed_sync() {
    let env = TestEnv::new("post_sync_after_failed_pull");
    env.write_config(&format!(
        "[hooks]\npost_sync = \"touch .hook-ran\"\n\n[repos]\n\"libs/x\" = {{ url = \"{}\", revision = \"main\" }}\n",
        env.repos_remote.join("missing.git").display()
    ));
    let out = env.run(&["sync"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(!env.playground.join(".hook-ran").exists());
}

/// Remotes spelled to look like an allowed repository while naming another:
/// the allowed host in the credentials or the path, a `..` that climbs out of
/// the owner (plain or percent-encoded), a lookalike Unicode host, a
/// lookalike owner. None of them passes `github.com/thepartly/*`.
#[test]
fn error_015_disguised_remotes_are_refused() {
    let env = hook_env(
        "post_sync_disguised_remotes",
        "https://github.com/thepartly/gitscale.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch pwned\"\n");
    for origin in [
        "https://github.com@evil.example/thepartly/x.git",
        "https://evil.example/github.com/thepartly/x.git",
        "git@evil.example:github.com/thepartly/x.git",
        "https://github.com/thepartly/../evil/x.git",
        "git@github.com:thepartly/../evil/x.git",
        "https://github.com/thepartly/%2e%2e/evil/x.git",
        "https://g\u{0456}thub.com/thepartly/x.git",
        "git@g\u{0456}thub.com:thepartly/x.git",
        "https://github.com/thepartly-evil/x.git",
    ] {
        run_git_pub(&env.playground, &["remote", "set-url", "origin", origin]);
        let out = env.run_as_hook("github.com/thepartly/*", &["sync"]);
        assert!(!out.success, "{} was allowed", origin);
        assert!(
            out.stderr.contains("refusing to run the post_sync hook"),
            "{}: {}",
            origin,
            out.stderr
        );
        assert!(!env.playground.join("pwned").exists(), "{} ran it", origin);
    }
}

/// A percent-encoded slash keeps `..` segments out of sight of the dot-segment
/// check: `acme/x%2F..%2F..%2Fevil%2Fy` reads, to a glob, as a repository under
/// `acme`, while a server that decodes `%2F` serves evil's. Such a remote
/// must answer to no owner pattern, as a plain `..` does.
#[test]
#[ignore = "bug: a %2F-encoded `..` path passes for a repository under the owner it climbs out of"]
fn error_016_an_encoded_slash_does_not_pass_for_a_path_under_the_owner() {
    let env = hook_env(
        "post_sync_encoded_slash",
        "https://github.com/thepartly/x%2F..%2F..%2Fevil%2Fy.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch pwned\"\n");
    let out = env.run_as_hook("github.com/thepartly/*", &["sync"]);
    assert!(!env.playground.join("pwned").exists());
    assert!(
        out.stderr.contains("refusing to run the post_sync hook"),
        "{}",
        out.stderr
    );
}

/// The docs give `*/acme/*` as "that owner on any host". Because `*` crosses
/// `/`, it also matches any path with an `acme` segment further down — an
/// attacker's own group with a subgroup called `acme`, on any host.
#[test]
#[ignore = "bug: `*/acme/*` matches gitlab.com/evil/acme/x, not only owner acme"]
fn error_017_a_star_owner_pattern_does_not_admit_a_subgroup_named_like_the_owner() {
    let env = hook_env(
        "post_sync_star_owner",
        "https://gitlab.com/evil/acme/payload.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch pwned\"\n");
    let out = env.run_as_hook("*/acme/*", &["sync"]);
    assert!(
        !env.playground.join("pwned").exists(),
        "an attacker's subgroup passed for owner acme"
    );
    assert!(
        out.stderr.contains("refusing to run the post_sync hook"),
        "{}",
        out.stderr
    );
}

/// An `origin` can carry credentials — older GitLab runners check out with the
/// job token in the URL. The refusal names the repository by host and path,
/// and never prints them.
#[test]
fn error_018_the_refusal_never_prints_credentials_from_the_origin() {
    let env = hook_env(
        "post_sync_refusal_userinfo",
        "https://gitlab-ci-token:S3CRET-TOKEN@gitlab.example.com/acme/app.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch pwned\"\n");
    let out = env.run_as_hook("github.com/*", &["sync"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(
            "gitlab.example.com/acme/app is not on this machine's gitscale hook allowlist"
        ),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("--allow 'github.com/*,gitlab.example.com/acme/app'"),
        "{}",
        out.stderr
    );
    assert!(
        !format!("{}{}", out.stdout, out.stderr).contains("S3CRET-TOKEN"),
        "{}",
        out.stderr
    );
}

/// The refused command is echoed for the developer to read, so it is made
/// safe for a terminal first: escape sequences that could repaint the screen
/// show as text, and a long command is cut short.
#[test]
fn error_019_the_refusal_escapes_and_shortens_the_command() {
    let env = hook_env(
        "post_sync_refusal_sanitized",
        "https://gitlab.example.com/attacker/payload.git",
    );
    let long = "x".repeat(300);
    env.write_config(&format!(
        "[hooks]\npost_sync = \"echo \\u001b[2Jhidden {}\"\n",
        long
    ));
    let out = env.run_as_hook("github.com/*", &["sync"]);
    assert!(!out.success);
    assert!(!out.stderr.contains('\u{1b}'), "raw escape in the refusal");
    assert!(out.stderr.contains("echo \\x1b[2Jhidden"), "{}", out.stderr);
    assert!(!out.stderr.contains(&long), "the command was not shortened");
    assert!(out.stderr.contains('…'), "{}", out.stderr);
}

/// Some non-ASCII letters lowercase to ASCII ones: U+212A KELVIN SIGN becomes
/// `k`. A host spelled with it is a different host from the one spelled with
/// `k`, and must not pass for it.
#[test]
#[ignore = "bug: matching lowercases with Unicode rules, so a U+212A host passes for one spelled with k"]
fn error_020_a_host_that_only_folds_to_an_allowed_one_is_refused() {
    let env = hook_env(
        "post_sync_kelvin_host",
        "git@git.\u{212A}nown.example:acme/app.git",
    );
    env.write_config("[hooks]\npost_sync = \"touch pwned\"\n");
    let out = env.run_as_hook("git.known.example/*", &["sync"]);
    assert!(
        !env.playground.join("pwned").exists(),
        "a Kelvin-sign host passed for git.known.example"
    );
    assert!(
        out.stderr.contains("refusing to run the post_sync hook"),
        "{}",
        out.stderr
    );
}
