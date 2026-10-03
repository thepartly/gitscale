//! `gitscale check`: the merge gate.

use crate::support::resolution::allow;
use crate::support::status_clean::*;
use crate::support::worktrees::*;
use crate::support::{run_git_pub, TestEnv};

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// A merge request into a branch the root does not pin is not gated.
#[test]
fn normal_001_gates_only_merges_into_pinned_branches() {
    let f = fixture("wt_ci_gate", "");
    f.core_commit("feat/x", "lib.txt", "remote work");
    let job = f.env.repos_remote.join("job");
    run_git_pub(
        &f.env.repos_remote,
        &[
            "clone",
            "-q",
            f.root.to_str().unwrap(),
            job.to_str().unwrap(),
        ],
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"))
        .args(["check", "-C", job.to_str().unwrap()])
        .env("CI", "true")
        .env("GITLAB_CI", "true")
        .env("CI_COMMIT_SHA", git(&job, &["rev-parse", "HEAD"]))
        .env("CI_MERGE_REQUEST_SOURCE_BRANCH_NAME", "feat/x")
        .env("CI_MERGE_REQUEST_TARGET_BRANCH_NAME", "integration")
        .env("CI_DEFAULT_BRANCH", "main")
        .env("GITSCALE_CACHE_DIR", &f.env.cache)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("not a pinned branch"));
}

/// The gate's main case: a merge request into the branch the root pins, with
/// a checkout taken from the topic, fails with exit 1 and says what would ship,
/// what was tested and both ways out.
#[test]
fn normal_002_fails_a_merge_request_into_a_pinned_branch() {
    let env = TestEnv::new("check_mr_into_pinned");
    let core = repo_with_topic(&env, "core", &[("lib.txt", "v1")]);
    let job = job_checkout(
        &env,
        &format!(
            "[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            core.display()
        ),
    );
    let tip = git(&core, &["rev-parse", "feat/x"]);

    let out = check_job(&env, &job, &gitlab_merge_request(&job, "feat/x", "main"));
    assert_eq!(out.code, Some(1), "{}", out.said());
    for line in [
        "imports/core was taken from branch feat/x, not from v1.0.0 pinned in .gitscale.toml."
            .to_string(),
        format!(
            "This pipeline tested imports/core at feat/x ({}), so merging now would ship a pin \
             that was not tested.",
            &tip[..7]
        ),
        "- imports/core's change not merged yet: merge it first, then run gitscale upgrade here \
         and push."
            .to_string(),
        format!(
            "- already merged and pinned: delete branch feat/x in {}, then rerun this pipeline.",
            core.display()
        ),
    ] {
        assert!(
            out.stderr.contains(&line),
            "missing {:?} in:\n{}",
            line,
            out.stderr
        );
    }
    assert!(!out.stdout.contains("ok:"), "{}", out.stdout);
}

/// `[develop] pinned` decides which merge targets are gated, globs included:
/// a merge into `release/1.0` is checked, and with that list set, `main` —
/// which it does not name — is not.
#[test]
fn normal_003_develop_pinned_globs_decide_which_targets_are_gated() {
    let env = TestEnv::new("check_pinned_globs");
    let core = repo_with_topic(&env, "core", &[("lib.txt", "v1")]);
    let job = job_checkout(
        &env,
        &format!(
            "[develop]\npinned = [\"release/*\"]\n\n\
             [repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            core.display()
        ),
    );

    let gated = check_job(
        &env,
        &job,
        &gitlab_merge_request(&job, "feat/x", "release/1.0"),
    );
    assert_eq!(gated.code, Some(1), "{}", gated.said());
    assert!(
        gated
            .stderr
            .contains("imports/core was taken from branch feat/x"),
        "{}",
        gated.stderr
    );

    let free = check_job(&env, &job, &gitlab_merge_request(&job, "feat/x", "main"));
    assert_eq!(free.code, Some(0), "{}", free.said());
    assert!(
        free.stdout
            .contains("ok: main is not a pinned branch, so merging into it is not gated"),
        "{}",
        free.stdout
    );
}

/// On GitHub the target is `GITHUB_BASE_REF` and the default branch comes from
/// the event payload: a pull request into the repository's default branch is
/// gated even when that branch is not called `main`, and one into another
/// branch is not.
#[test]
fn normal_004_on_github_the_base_ref_and_the_event_default_decide() {
    let env = TestEnv::new("check_github");
    let core = repo_with_topic(&env, "core", &[("lib.txt", "v1")]);
    let job = job_checkout(
        &env,
        &format!(
            "[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            core.display()
        ),
    );
    let event = env.repos_remote.join("event.json");
    let sha = git(&job, &["rev-parse", "HEAD"]);
    let pull_request = |default: &str| {
        std::fs::write(
            &event,
            format!(
                "{{\"repository\": {{\"default_branch\": \"{}\"}}}}",
                default
            ),
        )
        .unwrap();
        gitscale_at(
            &job,
            &env.cache,
            &[
                ("CI", "true"),
                ("GITHUB_ACTIONS", "true"),
                ("GITHUB_SHA", &sha),
                ("GITHUB_HEAD_REF", "feat/x"),
                ("GITHUB_BASE_REF", "trunk"),
                ("GITHUB_EVENT_PATH", event.to_str().unwrap()),
            ],
            &["check"],
        )
    };

    let gated = pull_request("trunk");
    assert_eq!(gated.code, Some(1), "{}", gated.said());
    assert!(
        gated
            .stderr
            .contains("imports/core was taken from branch feat/x"),
        "{}",
        gated.stderr
    );

    let free = pull_request("main");
    assert_eq!(free.code, Some(0), "{}", free.said());
    assert!(
        free.stdout.contains("ok: trunk is not a pinned branch"),
        "{}",
        free.stdout
    );
}

/// The pin that would ship is named with the file it is written in: for a
/// dependency's dependency, that repository's own `.gitscale.toml`.
#[test]
fn normal_005_names_the_config_of_the_dependency_that_pins_it() {
    let env = TestEnv::new("check_dependency_pin");
    let d = repo_with_topic(&env, "d", &[("d.txt", "d")]);
    let b_config = format!(
        "[repos]\n\"libs/d\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let b = env.create_bare_repo("b", "main", &[(".gitscale.toml", &b_config)]);
    run_git_pub(&b, &["tag", "v1.0.0", "main"]);
    let job = job_checkout(
        &env,
        &format!(
            "{}[repos]\n\"imports/b\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            allow(&env),
            b.display()
        ),
    );

    let out = check_job(&env, &job, &gitlab_branch(&job, "feat/x"));
    assert_eq!(out.code, Some(1), "{}", out.said());
    assert!(
        out.stderr.contains(
            "was taken from branch feat/x, not from v1.0.0 pinned in imports/b/.gitscale.toml."
        ),
        "{}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("imports/b was taken"),
        "b has no topic branch: {}",
        out.stderr
    );
}

/// Every checkout taken from the topic is named, not only the first: fixing
/// one at a time against a gate that hides the rest is a pipeline per slot.
#[test]
fn normal_006_lists_every_checkout_taken_from_the_topic() {
    let env = TestEnv::new("check_every_slot");
    let core = repo_with_topic(&env, "core", &[("lib.txt", "v1")]);
    let util = repo_with_topic(&env, "util", &[("util.txt", "v1")]);
    let job = job_checkout(
        &env,
        &format!(
            "[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n\
             \"imports/util\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            core.display(),
            util.display()
        ),
    );

    let out = check_job(&env, &job, &gitlab_branch(&job, "feat/x"));
    assert_eq!(out.code, Some(1), "{}", out.said());
    for dir in ["imports/core", "imports/util"] {
        assert!(
            out.stderr
                .contains(&format!("{} was taken from branch feat/x", dir)),
            "{} missing:\n{}",
            dir,
            out.stderr
        );
    }
}

/// Outside CI the gate speaks of the workspace and of running itself again,
/// not of a pipeline that does not exist.
#[test]
fn normal_007_outside_ci_it_speaks_of_the_workspace() {
    let env = TestEnv::new("check_local");
    let core = repo_with_topic(&env, "core", &[("lib.txt", "v1")]);
    env.write_config(&format!(
        "[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        core.display()
    ));
    env.init_playground_git();
    run_git_pub(&env.playground, &["switch", "-q", "-c", "feat/x"]);
    let pull = env.run(&["pull"]);
    assert!(pull.success, "{}{}", pull.stdout, pull.stderr);

    let out = gitscale(&env, &[], &["check"]);
    assert_eq!(out.code, Some(1), "{}", out.said());
    assert!(
        out.stderr
            .contains("This workspace tested imports/core at feat/x"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("then rerun gitscale check."),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains("pipeline"), "{}", out.stderr);
}

/// No topic anywhere: the gate passes, and says so.
#[test]
fn normal_008_passes_when_every_checkout_is_at_a_pin() {
    let env = TestEnv::new("check_all_pinned");
    let core = repo_with_topic(&env, "core", &[("lib.txt", "v1")]);
    let job = job_checkout(
        &env,
        &format!(
            "[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            core.display()
        ),
    );

    let out = check_job(&env, &job, &gitlab_merge_request(&job, "fix/y", "main"));
    assert_eq!(out.code, Some(0), "{}", out.said());
    assert!(
        out.stdout
            .contains("ok: every checkout resolves from a pinned revision"),
        "{}",
        out.stdout
    );
}

/// With no default branch from the pipeline — a GitHub event without a
/// payload — the root's own `origin/HEAD` names it: a pull request into a
/// default branch called `trunk` is gated.
#[test]
fn normal_012_without_a_pipeline_default_origin_head_names_the_default_branch() {
    let env = TestEnv::new("check_origin_head_default");
    let core = repo_with_topic(&env, "core", &[("lib.txt", "v1")]);
    let config = format!(
        "[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        core.display()
    );
    let root = env.create_bare_repo("root", "trunk", &[(".gitscale.toml", &config)]);
    let job = env.repos_remote.join("job");
    run_git_pub(
        &env.repos_remote,
        &["clone", "-q", root.to_str().unwrap(), job.to_str().unwrap()],
    );
    run_git_pub(&job, &["checkout", "-q", "--detach"]);
    let sha = git(&job, &["rev-parse", "HEAD"]);

    let out = gitscale_at(
        &job,
        &env.cache,
        &[
            ("CI", "true"),
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", &sha),
            ("GITHUB_HEAD_REF", "feat/x"),
            ("GITHUB_BASE_REF", "trunk"),
        ],
        &["check"],
    );
    assert_eq!(out.code, Some(1), "{}", out.said());
    assert!(
        out.stderr
            .contains("imports/core was taken from branch feat/x"),
        "{}",
        out.stderr
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A checkout held at its pin because the repository above it pins the
/// topic's branch is not from the topic, so it is not what blocks: only the
/// repository that follows the branch is named.
#[test]
fn edge_009_a_checkout_a_repository_holds_at_its_pin_does_not_block() {
    let env = TestEnv::new("check_pinned_by");
    let d = env.create_bare_repo("d", "main", &[("d.txt", "d v1")]);
    run_git_pub(&d, &["tag", "v1.0.0", "main"]);
    run_git_pub(&d, &["branch", "staging", "main"]);
    env.push_commit(&d, "staging", "d.txt", "d staging");
    let b_config = format!(
        "[develop]\npinned = [\"staging\"]\n\n[repos]\n\"libs/d\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
        d.display()
    );
    let b = env.create_bare_repo("b", "main", &[(".gitscale.toml", &b_config)]);
    run_git_pub(&b, &["tag", "v1.0.0", "main"]);
    run_git_pub(&b, &["branch", "staging", "main"]);
    let job = job_checkout(
        &env,
        &format!(
            "{}[repos]\n\"imports/b\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            allow(&env),
            b.display()
        ),
    );

    let out = check_job(&env, &job, &gitlab_branch(&job, "staging"));
    assert_eq!(out.code, Some(1), "{}", out.said());
    assert!(
        out.stderr
            .contains("imports/b was taken from branch staging"),
        "{}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("imports/d was taken"),
        "d is held at b's pin: {}",
        out.stderr
    );
}

/// What the gate does when it cannot fetch but has fetched before. This pins
/// the current behaviour, which is to resolve from the refs fetched last time
/// with a warning on stderr: the gate then answers from stale refs. Whether a
/// merge gate should fail instead when it cannot reach a remote is a decision
/// for the owner.
#[test]
fn edge_010_a_failed_fetch_answers_from_what_was_fetched_before() {
    let env = TestEnv::new("check_stale_refs");
    let core = repo_with_topic(&env, "core", &[("lib.txt", "v1")]);
    let job = job_checkout(
        &env,
        &format!(
            "[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            core.display()
        ),
    );
    let vars = gitlab_branch(&job, "main");
    let first = check_job(&env, &job, &vars);
    assert_eq!(first.code, Some(0), "{}", first.said());

    let away = core.with_extension("moved");
    std::fs::rename(&core, &away).unwrap();
    let out = check_job(&env, &job, &vars);
    assert!(
        out.stderr.contains("using what was fetched before"),
        "{}",
        out.said()
    );
    assert_eq!(out.code, Some(0), "{}", out.said());
}

/// A `replace` artefact taken from the topic — the image of its branch tip —
/// blocks like a source checkout: a merge would ship the pinned image, not
/// the one tested.
#[test]
fn edge_013_a_replace_artefact_from_the_topic_blocks() {
    let env = TestEnv::new("check_topic_artefact");
    let app = env.artefact_repo("app", &[("app.bin", "main build")]);
    run_git_pub(&app, &["branch", "feat/x", "main"]);
    env.push_commit(&app, "feat/x", "README.md", "feature");
    env.publish(&app, "feat/x", &[("app.bin", "feature build")]);
    let job = job_checkout(
        &env,
        &format!(
            "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"main\", artefact = \"replace\" }}\n",
            env.registries(),
            app.display()
        ),
    );

    let out = check_job(&env, &job, &gitlab_branch(&job, "feat/x"));
    assert_eq!(out.code, Some(1), "{}", out.said());
    assert!(
        out.stderr.contains("meta/app was taken from branch feat/x"),
        "{}",
        out.stderr
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

/// A repository the gate cannot fetch, and never has, is an error with exit
/// 1: a gate that cannot resolve must not pass.
#[test]
fn error_011_a_repository_it_cannot_fetch_fails_the_check() {
    let env = TestEnv::new("check_unreachable");
    let missing = env.repos_remote.join("nowhere.git");
    let job = job_checkout(
        &env,
        &format!(
            "[repos]\n\"imports/core\" = {{ url = \"{}\", revision = \"v1.0.0\" }}\n",
            missing.display()
        ),
    );

    let out = check_job(&env, &job, &gitlab_branch(&job, "main"));
    assert_eq!(out.code, Some(1), "{}", out.said());
    assert!(out.stderr.contains("nowhere.git"), "{}", out.stderr);
    assert!(!out.stdout.contains("ok:"), "{}", out.stdout);
}
