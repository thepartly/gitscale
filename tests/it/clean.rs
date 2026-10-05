//! `git scale clean`: removing untracked files from the workspace and its
//! checkouts, with git's flags, without touching what gitscale manages; and
//! `git scale gc`.

use crate::support;
use crate::support::artefacts::*;
use crate::support::resolution::*;
use crate::support::status_clean::*;
use crate::support::workspace::*;
use crate::support::TestEnv;

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

#[test]
fn normal_001_force_removes_untracked_files_and_keeps_checkouts() {
    let env = clean_env("clean_removes_untracked_and_keeps_checkouts");
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
    std::fs::create_dir_all(env.playground.join("build")).unwrap();
    std::fs::write(env.playground.join("build/out"), "x").unwrap();
    std::fs::write(env.playground.join("core/scratch.txt"), "x").unwrap();

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);

    assert!(!env.playground.join("junk.txt").exists());
    assert!(!env.playground.join("build").exists());
    assert!(!env.playground.join("core/scratch.txt").exists());
    // The checkout is untracked as far as the workspace repo is concerned, so
    // an unguarded clean at the root would have taken it.
    assert!(env.playground.join("core/README.md").exists());
    assert!(env.playground.join(".gitscale.toml").exists());
}

#[test]
fn normal_002_a_dry_run_lists_and_removes_nothing() {
    let env = clean_env("clean_dry_run_removes_nothing");
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();

    let out = env.run(&["clean"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join("junk.txt").exists(),
        "a dry run deleted files"
    );
    assert!(out.stdout.contains("junk.txt"), "stdout: {}", out.stdout);
    assert!(out.stdout.contains("dry run"), "stdout: {}", out.stdout);
    // Each repository is accounted for, and the total is what -f would do.
    assert!(
        out.stdout
            .lines()
            .any(|l| l == "  core — nothing to remove"),
        "stdout: {}",
        out.stdout
    );
    assert!(
        out.stdout.lines().any(|l| l == "    junk.txt"),
        "stdout: {}",
        out.stdout
    );
    assert!(
        out.stdout.trim_end().ends_with("1 path in 1 repo."),
        "stdout: {}",
        out.stdout
    );
}

#[test]
fn normal_003_a_subrepo_takes_excludes_from_its_own_config() {
    let env = TestEnv::new("subrepo_clean_excludes_come_from_its_own_config");
    // A `[clean]`-only config: core declares what it keeps without being a
    // workspace itself.
    let core = env.create_bare_repo(
        "core",
        "main",
        &[
            ("README.md", "core"),
            (".gitscale.toml", "[clean]\nexclude = [\"envs/\"]\n"),
        ],
    );
    env.write_config(&format!(
        "[repos]\n\"core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        core.to_str().unwrap()
    ));
    env.init_playground_git();
    assert!(env.run(&["sync"]).success);

    std::fs::create_dir_all(env.playground.join("core/envs")).unwrap();
    std::fs::write(env.playground.join("core/envs/dev"), "x").unwrap();
    std::fs::write(env.playground.join("core/build.out"), "x").unwrap();

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join("core/envs/dev").exists(),
        "core's own exclude was ignored"
    );
    assert!(!env.playground.join("core/build.out").exists());
}

#[test]
fn normal_004_keeps_managed_symlinks() {
    let env = TestEnv::new("clean_keeps_managed_symlinks");
    let shared = env.create_bare_repo("sharedlibs", "main", &[("lib.txt", "shared")]);
    let child_config = format!(
        "[repos]\n\"libs/shared\" = {{ url = \"{}\", revision = \"main\" }}\n",
        shared.to_str().unwrap()
    );
    let core = env.create_bare_repo(
        "core",
        "main",
        &[("README.md", "core"), (".gitscale.toml", &child_config)],
    );
    env.write_config(&format!(
        "[repos]\n\"core\" = {{ url = \"{}\", revision = \"main\" }}\n\
         \"sharedlibs\" = {{ url = \"{}\", revision = \"main\" }}\n",
        core.to_str().unwrap(),
        shared.to_str().unwrap()
    ));
    env.init_playground_git();
    assert!(env.run(&["sync"]).success);

    let link = env.playground.join("core/libs/shared");
    assert!(
        link.is_symlink(),
        "expected resolve to have created the link"
    );

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        link.is_symlink(),
        "clean removed a gitscale-managed symlink"
    );
    assert!(env.playground.join("core/libs/shared/lib.txt").exists());
}

#[test]
fn normal_005_a_command_line_exclude_applies_to_every_repo() {
    let env = clean_env("cli_exclude_applies_to_every_repo");
    std::fs::write(env.playground.join("keep.me"), "x").unwrap();
    std::fs::write(env.playground.join("core/keep.me"), "x").unwrap();
    std::fs::write(env.playground.join("core/drop.me"), "x").unwrap();

    let out = env.run(&["clean", "-fdx", "-e", "keep.me"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(env.playground.join("keep.me").exists());
    assert!(env.playground.join("core/keep.me").exists());
    assert!(!env.playground.join("core/drop.me").exists());
}

#[test]
fn normal_006_names_select_repos_and_dot_the_root() {
    let env = clean_env("clean_names_select_repos_and_the_root");
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
    std::fs::write(env.playground.join("core/scratch.txt"), "x").unwrap();

    let out = env.run(&["clean", "-fdx", "core"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join("junk.txt").exists(),
        "naming a repo should leave the root alone"
    );
    assert!(!env.playground.join("core/scratch.txt").exists());

    let out = env.run(&["clean", "-fdx", "."]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(!env.playground.join("junk.txt").exists());

    // Both at once.
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
    std::fs::write(env.playground.join("core/scratch.txt"), "x").unwrap();
    let out = env.run(&["clean", "-fdx", ".", "core"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(!env.playground.join("junk.txt").exists());
    assert!(!env.playground.join("core/scratch.txt").exists());
}

#[test]
fn normal_007_skips_artefact_checkouts() {
    let env = TestEnv::new("clean_skips_artefact_repos");
    let bare = env.artefact_repo("svc", &[("a.txt", "x")]);
    env.write_config(&format!(
        "{}[repos]\n\
         \"meta/svc\" = {{ url = \"{}\", revision = \"main\", artefact = \"replace\" }}\n",
        env.registries(),
        bare.display()
    ));
    env.init_playground_git();
    assert!(env.run(&["sync"]).success);

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join("meta/svc/dist/a.txt").exists(),
        "an artefact checkout has no working tree to clean"
    );
    assert!(
        out.stdout.contains("meta/svc (artefact)"),
        "the skip names its reason: {}",
        out.stdout
    );
}

/// `git scale gc` drops the images nothing has used within the period, and the
/// layers no remaining image needs.
#[test]
fn normal_008_gc_drops_cold_images_and_their_blobs() {
    let env = TestEnv::new("art_clean_gc");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let old = layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["sync"]).success);
    env.push_commit(&bare, "main", "README.md", "v2");
    let new = layered(&env, &bare, "app v2");
    assert!(env.run(&["sync"]).success);

    let entry = image_store(&env);
    let blobs = || {
        std::fs::read_dir(entry.join("blobs/sha256"))
            .unwrap()
            .count()
    };
    let before = blobs();
    age(&entry, &old);

    let out = env.run(&["gc", "--keep-recent", "30d"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("1 image dropped"), "{}", out.stdout);
    let index = std::fs::read_to_string(entry.join("index.json")).unwrap();
    assert!(!index.contains(&old) && index.contains(&new), "{}", index);
    // The old manifest and its app layer go; the shared vendor layer stays.
    assert_eq!(blobs(), before - 2);
}

/// The dry run says why it leaves a repository alone, so a skipped one is
/// never mistaken for a clean one: an artefact, a checkout not cloned yet,
/// and an entry that is a symlink each name their reason.
#[test]
fn normal_019_a_dry_run_names_why_it_skips_what_it_skips() {
    let env = TestEnv::new("clean_dry_run_skips");
    let svc = env.artefact_repo("svc", &[("a.txt", "x")]);
    let later = env.create_bare_repo("later", "main", &[("l.txt", "l")]);
    let linked = env.create_bare_repo("linked", "main", &[("k.txt", "k")]);
    env.write_config(&format!(
        "{}[repos]\n\
         \"meta/svc\" = {{ url = \"{}\", revision = \"main\", artefact = \"replace\" }}\n\
         \"libs/later\" = {{ url = \"{}\", revision = \"main\" }}\n\
         \"libs/linked\" = {{ url = \"{}\", revision = \"main\" }}\n",
        env.registries(),
        svc.display(),
        later.display(),
        linked.display()
    ));
    env.init_playground_git();
    assert!(env.run(&["sync", "meta/svc", "libs/linked"]).success);
    symlink_entry(&env, &linked, "libs/linked");

    let out = env.run(&["clean"]);
    assert!(out.success, "{}", out.stderr);
    for line in [
        "  meta/svc — skip (artefact)",
        "  libs/later — skip (not cloned)",
        "  libs/linked — skip (symlink)",
    ] {
        assert!(
            out.stdout.lines().any(|l| l == line),
            "missing {:?}:\n{}",
            line,
            out.stdout
        );
    }
}

/// A leading `/` anchors a pattern at the root of each repository cleaned —
/// the workspace's and every checkout's — and nowhere deeper.
#[test]
fn normal_020_an_anchored_exclude_keeps_only_the_match_at_each_repo_root() {
    let env = clean_env("clean_anchored_exclude");
    for base in ["", "core/"] {
        std::fs::create_dir_all(env.playground.join(format!("{}sub", base))).unwrap();
        std::fs::write(env.playground.join(format!("{}keep.me", base)), "x").unwrap();
        std::fs::write(env.playground.join(format!("{}sub/keep.me", base)), "x").unwrap();
    }

    let out = env.run(&["clean", "-fdx", "-e", "/keep.me"]);
    assert!(out.success, "{}", out.stderr);
    for base in ["", "core/"] {
        assert!(
            env.playground.join(format!("{}keep.me", base)).exists(),
            "{}keep.me was anchored at its repo's root",
            base
        );
        assert!(
            !env.playground.join(format!("{}sub/keep.me", base)).exists(),
            "{}sub/keep.me is not at the root",
            base
        );
    }
}

/// `gc` without `--keep-recent` keeps what `[clean] keep_recent` says, not
/// the built-in three months, and the flag overrides the config. The summary
/// counts the stores it collected.
#[test]
fn normal_021_gc_takes_its_period_from_the_config_unless_the_flag_overrides_it() {
    let env = TestEnv::new("clean_gc_config_period");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let lib = env.create_bare_repo("mylib", "main", &[("a.txt", "a")]);
    let old = layered(&env, &bare, "app v1");
    let config = |keep: &str| {
        format!(
            "[clean]\nkeep_recent = \"{}\"\n\n{}\"libs/mylib\" = {{ url = \"{}\", revision = \"main\" }}\n",
            keep,
            entry_config(&env, &bare, "main"),
            lib.display()
        )
    };
    env.write_config(&config("100years"));
    assert!(env.run(&["sync"]).success);
    env.push_commit(&bare, "main", "README.md", "v2");
    layered(&env, &bare, "app v2");
    assert!(env.run(&["sync"]).success);
    let entry = image_store(&env);
    age(&entry, &old);
    let held = || std::fs::read_to_string(entry.join("index.json")).unwrap();

    // Used 26 years ago is within a hundred years.
    let out = env.run(&["gc"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("0 images dropped"), "{}", out.stdout);
    assert!(held().contains(&old), "the config's period was not used");
    let stores = regex::Regex::new(r": ([1-9][0-9]*) stores? collected").unwrap();
    assert!(stores.is_match(&out.stdout), "{}", out.stdout);

    let out = env.run(&["gc", "--keep-recent", "30d"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("1 image dropped"), "{}", out.stdout);
    assert!(!held().contains(&old), "{}", held());
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn edge_009_an_uncommitted_workspace_config_survives_its_own_clean() {
    let env = TestEnv::new("an_uncommitted_workspace_config_survives_its_own_clean");
    env.init_playground_git();
    // Written after the initial commit, so it is untracked.
    env.write_config("[repos]\n");

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(env.playground.join(".gitscale.toml").exists());
}

#[test]
fn edge_010_root_clean_excludes_do_not_reach_subrepos() {
    let env = clean_env("root_clean_excludes_do_not_reach_subrepos");
    let existing = std::fs::read_to_string(env.playground.join(".gitscale.toml")).unwrap();
    std::fs::write(
        env.playground.join(".gitscale.toml"),
        format!("[clean]\nexclude = [\".env\"]\n\n{}", existing),
    )
    .unwrap();
    support::run_git_pub(&env.playground, &["add", "."]);
    support::run_git_pub(&env.playground, &["commit", "-m", "clean rules"]);

    std::fs::write(env.playground.join(".env"), "root").unwrap();
    std::fs::write(env.playground.join("core/.env"), "child").unwrap();

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join(".env").exists(),
        "the root's own exclude was ignored"
    );
    assert!(
        !env.playground.join("core/.env").exists(),
        "a root exclude leaked into a sub-repo"
    );
}

/// `recursive = false` puts the repo's own config out of reach, keep-list
/// included, so it is cleaned by the command line's patterns alone — and what
/// lives inside it that gitscale did not put there is left to git, which
/// reports a nested clone rather than deleting it.
#[test]
fn edge_011_recursive_false_is_cleaned_without_its_own_keep_list() {
    let env = TestEnv::new("recursive_false_cleaned");
    let core = env.create_bare_repo(
        "core",
        "main",
        &[
            ("README.md", "core"),
            (".gitscale.toml", "[clean]\nexclude = [\"envs/\"]\n"),
        ],
    );
    env.write_config(&format!(
        "[repos]\n\"core\" = {{ url = \"{}\", revision = \"main\", recursive = false }}\n",
        core.to_str().unwrap()
    ));
    env.init_playground_git();
    assert!(env.run(&["sync"]).success);

    std::fs::create_dir_all(env.playground.join("core/envs")).unwrap();
    std::fs::write(env.playground.join("core/envs/dev"), "x").unwrap();
    std::fs::write(env.playground.join("core/build.out"), "x").unwrap();
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
    // A dependency of core's own, cloned by hand: a grandchild of the workspace.
    let grandchild = env.playground.join("core/vendor/dep");
    std::fs::create_dir_all(&grandchild).unwrap();
    support::run_git_pub(&grandchild, &["init", "-q"]);

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(!env.playground.join("core/build.out").exists());
    assert!(
        !env.playground.join("core/envs/dev").exists(),
        "a keep-list gitscale does not read cannot keep anything"
    );
    assert!(
        grandchild.join(".git").exists(),
        "a nested clone is reported, never deleted"
    );
    assert!(!env.playground.join("junk.txt").exists());
}

#[test]
fn edge_012_works_in_a_readonly_checkout() {
    let env = TestEnv::new("clean_works_in_a_readonly_repo");
    let core = env.create_bare_repo("core", "main", &[("README.md", "core")]);
    env.write_config(&format!(
        "[repos]\n\"core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        core.to_str().unwrap()
    ));
    env.init_playground_git();
    assert!(env.run(&["sync"]).success);

    // readonly mode clears the write bit on files; unlinking one needs write
    // permission on its directory rather than on the file.
    let scratch = env.playground.join("core/scratch.txt");
    std::fs::write(&scratch, "x").unwrap();
    let mut perms = std::fs::metadata(&scratch).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o444);
    std::fs::set_permissions(&scratch, perms).unwrap();

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(!scratch.exists());
}

#[test]
fn edge_013_keeps_a_checkout_nested_inside_another() {
    let env = TestEnv::new("clean_keeps_a_checkout_nested_inside_another_repo");
    let core = env.create_bare_repo("core", "main", &[("README.md", "core")]);
    // An artefact checkout has no `.git`, so git's own refusal to delete a
    // nested repository would not save it.
    let vendor = env.artefact_repo("vendor", &[("v.txt", "x")]);
    env.write_config(&format!(
        "{}[repos]\n\
         \"core\" = {{ url = \"{}\", revision = \"main\" }}\n\
         \"core/vendor\" = {{ url = \"{}\", revision = \"main\", artefact = \"replace\" }}\n",
        env.registries(),
        core.to_str().unwrap(),
        vendor.display()
    ));
    env.init_playground_git();
    assert!(env.run(&["sync"]).success);
    assert!(env.playground.join("core/vendor/dist/v.txt").exists());

    std::fs::write(env.playground.join("core/scratch.tmp"), "x").unwrap();

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join("core/vendor/dist/v.txt").exists(),
        "cleaning core deleted a checkout declared inside it"
    );
    assert!(!env.playground.join("core/scratch.tmp").exists());
}

#[test]
fn edge_014_removes_a_directory_holding_no_repository_so_sync_works() {
    let env = workspace_with_stray_directory("clean_stray_entry_dir", |dir| {
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("target/out.o"), "x").unwrap();
    });
    let stray = env.playground.join("libs/core");

    let dry = env.run(&["clean"]);
    assert!(dry.success, "stderr: {}", dry.stderr);
    assert!(dry.stdout.contains("holds no repository"), "{}", dry.stdout);
    assert!(stray.join("target/out.o").exists(), "a dry run deleted it");

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(!stray.exists(), "nothing in it belongs to a checkout");

    // The point of it: a clean leaves nothing that stops the next sync.
    let pull = env.run(&["sync"]);
    assert!(pull.success, "stderr: {}", pull.stderr);
    assert!(stray.join("README.md").is_file());
    assert_eq!(
        git_out(&env.playground, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature"
    );
}

#[test]
fn edge_015_leaves_a_directory_whose_git_is_broken() {
    // It has a `.git`, so it was a checkout — a damaged one can still hold the
    // only copy of somebody's work.
    let env = workspace_with_stray_directory("clean_broken_entry_git", |dir| {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("work.txt"), "unpushed").unwrap();
    });

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    let said = format!("{}{}", out.stdout, out.stderr);
    assert!(said.contains("not a git repository"), "{}", said);
    assert!(env.playground.join("libs/core/work.txt").exists());
}

#[test]
fn edge_016_leaves_a_stray_directory_holding_another_checkout() {
    let env = TestEnv::new("clean_stray_holds_checkout");
    let core = env.create_bare_repo("core", "main", &[("README.md", "core")]);
    let vendor = env.create_bare_repo("vendor", "main", &[("v.txt", "v")]);
    env.write_config(&format!(
        "[repos]\n\"libs/core\" = {{ url = \"{}\", revision = \"main\" }}\n\
         \"libs/core/vendor\" = {{ url = \"{}\", revision = \"main\" }}\n",
        core.to_str().unwrap(),
        vendor.to_str().unwrap()
    ));
    env.init_playground_git();
    assert!(env.run(&["sync", "libs/core/vendor"]).success);
    assert!(env.playground.join("libs/core/vendor/v.txt").is_file());

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "stderr: {}", out.stderr);
    assert!(
        env.playground.join("libs/core/vendor/v.txt").is_file(),
        "removing libs/core would have taken the vendor checkout with it"
    );
    let said = format!("{}{}", out.stdout, out.stderr);
    assert!(said.contains("another declared checkout"), "{}", said);
}

/// The root's clean keeps the implicit checkouts, which are untracked
/// directories to it like any declared one, and a dependency's clean keeps
/// the links planted in it.
#[test]
fn edge_017_keeps_implicit_checkouts_and_their_links() {
    let env = TestEnv::new("res_clean_implicit");
    env.init_playground_git();
    implicit_d(&env);
    assert!(env.run(&["sync"]).success);
    std::fs::write(env.playground.join("stray.txt"), "x").unwrap();

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    assert!(!env.playground.join("stray.txt").exists(), "the clean ran");
    assert!(env.playground.join("imports/d/VERSION").is_file());
    assert!(env.playground.join("imports/b/libs/d").is_symlink());
}

/// An overlay's files are kept by name, and a name is not a pattern: build
/// output such as a Next.js `pages/[slug].js` must survive a clean like any
/// other overlay file.
#[test]
#[ignore = "bug: overlay file names reach git clean -e unescaped, so [slug].js is read as a glob and deleted"]
fn edge_022_overlay_files_named_with_glob_characters_survive() {
    let env = TestEnv::new("clean_overlay_glob_names");
    let app = env.create_bare_repo(
        "app",
        "main",
        &[("README.md", "app"), (".gitignore", "/dist/\n")],
    );
    env.publish(
        &app,
        "main",
        &[
            ("pages/[slug].js", "route"),
            ("pages/s.js", "plain"),
            ("star*.txt", "star"),
        ],
    );
    env.write_config(&format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"main\", artefact = \"overlay\" }}\n",
        env.registries(),
        app.display()
    ));
    let pull = env.run(&["sync"]);
    assert!(pull.success, "{}{}", pull.stdout, pull.stderr);
    let dist = env.playground.join("meta/app/dist");
    assert!(dist.join("pages/[slug].js").is_file());

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "{}", out.stderr);
    for file in ["pages/[slug].js", "pages/s.js", "star*.txt"] {
        assert!(
            dist.join(file).is_file(),
            "the overlay's {} was deleted",
            file
        );
    }
}

/// A declared checkout is kept by its directory's name, whatever characters
/// that name holds: an artefact checkout, which has no `.git` for git to
/// recognise, named `meta/app[1]` must survive the workspace's clean.
#[test]
#[ignore = "bug: checkout directories reach git clean -e unescaped, so a name with [ ] is a glob and the checkout is deleted"]
fn edge_023_a_checkout_named_with_glob_characters_survives_the_root_clean() {
    let env = TestEnv::new("clean_checkout_glob_name");
    let app = env.artefact_repo("app", &[("app.bin", "x")]);
    env.write_config(&format!(
        "{}[repos]\n\"meta/app[1]\" = {{ url = \"{}\", revision = \"main\", artefact = \"replace\" }}\n",
        env.registries(),
        app.display()
    ));
    env.init_playground_git();
    let pull = env.run(&["sync"]);
    assert!(pull.success, "{}{}", pull.stdout, pull.stderr);
    let installed = env.playground.join("meta/app[1]/dist/app.bin");
    assert!(installed.is_file());

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        installed.is_file(),
        "the root clean deleted a declared checkout"
    );
}

/// An entry's directory holding no repository is removed whole, as the docs
/// say — and that includes a repository somebody cloned by hand inside it,
/// commits and all. This pins the current behaviour: it contradicts the rule
/// that a nested repository gitscale does not manage is reported rather than
/// deleted, and is listed as a decision for the owner.
#[test]
fn edge_024_a_stray_directory_is_removed_with_a_clone_made_inside_it() {
    let env = workspace_with_stray_directory("clean_stray_with_clone", |dir| {
        let mine = dir.join("mine");
        std::fs::create_dir_all(&mine).unwrap();
        support::run_git_pub(&mine, &["init", "-q"]);
        support::run_git_pub(&mine, &["config", "user.email", "t@t.com"]);
        support::run_git_pub(&mine, &["config", "user.name", "T"]);
        std::fs::write(mine.join("work.txt"), "unpushed").unwrap();
        support::run_git_pub(&mine, &["add", "."]);
        support::run_git_pub(&mine, &["commit", "-q", "-m", "only copy"]);
    });

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "{}", out.stderr);
    assert!(!env.playground.join("libs/core").exists());
}

/// An entry that is a symlink to somebody's own checkout is skipped, and
/// nothing is removed from what it points at.
#[test]
fn edge_025_a_symlinked_entry_is_skipped_and_its_target_left_alone() {
    let env = clean_env("clean_symlinked_entry");
    let core = env.repos_remote.join("core.git");
    let own = symlink_entry(&env, &core, "core");
    std::fs::write(own.join("mine.txt"), "x").unwrap();

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("core (symlink)"), "{}", out.stdout);
    assert!(
        own.join("mine.txt").exists(),
        "clean reached through the link"
    );
    assert!(env.playground.join("core").is_symlink());
}

/// An untracked symlink to something outside the workspace is removed as the
/// link it is: what it points at stays.
#[test]
fn edge_026_an_untracked_symlink_is_removed_without_following_it() {
    let env = clean_env("clean_outside_symlink");
    let outside = env.repos_remote.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("precious.txt"), "x").unwrap();
    std::os::unix::fs::symlink(&outside, env.playground.join("core/out")).unwrap();
    std::os::unix::fs::symlink(&outside, env.playground.join("out")).unwrap();

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "{}", out.stderr);
    assert!(env.playground.join("core/out").symlink_metadata().is_err());
    assert!(env.playground.join("out").symlink_metadata().is_err());
    assert!(outside.join("precious.txt").exists());
}

/// What the dry run lists is what `-f` then removes — names with spaces,
/// non-ASCII letters and a leading dash included — and the counts agree.
#[test]
fn edge_027_a_dry_run_lists_exactly_what_force_removes() {
    let env = clean_env("clean_dry_run_fidelity");
    let core = env.playground.join("core");
    let names = ["sp ace.txt", "ünï.txt", "-dash.txt", "build/out.o"];
    std::fs::create_dir_all(core.join("build")).unwrap();
    for name in names {
        std::fs::write(core.join(name), "x").unwrap();
    }
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();

    let dry = env.run(&["clean", "-dx"]);
    assert!(dry.success, "{}", dry.stderr);
    assert!(
        dry.stdout.contains("\n5 paths in 2 repos.\n"),
        "{}",
        dry.stdout
    );
    for shown in [
        "    sp ace.txt",
        "    -dash.txt",
        "    build/",
        "    junk.txt",
    ] {
        assert!(
            dry.stdout.lines().any(|l| l == shown),
            "{:?} not listed:\n{}",
            shown,
            dry.stdout
        );
    }
    for name in names {
        assert!(core.join(name).exists(), "the dry run removed {}", name);
    }

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.contains("core (4 paths)"), "{}", out.stdout);
    assert!(out.stdout.contains(". (1 path)"), "{}", out.stdout);
    for name in names {
        assert!(!core.join(name).exists(), "{} survived -f", name);
    }
    assert!(core.join("README.md").exists());
}

/// A checkout's own `.gitscale.toml` is kept even before it is committed,
/// like the workspace's.
#[test]
fn edge_028_a_checkouts_uncommitted_config_survives() {
    let env = clean_env("clean_child_config");
    let config = env.playground.join("core/.gitscale.toml");
    std::fs::write(&config, "[clean]\nexclude = [\"envs/\"]\n").unwrap();
    std::fs::write(env.playground.join("core/junk.txt"), "x").unwrap();

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "{}", out.stderr);
    assert!(config.is_file(), "a checkout's own config was deleted");
    assert!(!env.playground.join("core/junk.txt").exists());
}

/// Exclusions describe untracked files inside a repository; a directory
/// holding none is removed whole whatever they say.
#[test]
fn edge_029_excludes_do_not_reach_into_a_directory_holding_no_repository() {
    let env = workspace_with_stray_directory("clean_stray_ignores_excludes", |dir| {
        std::fs::write(dir.join("keep.me"), "x").unwrap();
    });

    let out = env.run(&["clean", "-fdx", "-e", "keep.me"]);
    assert!(out.success, "{}", out.stderr);
    assert!(!env.playground.join("libs/core").exists());
}

/// A `.git` that is a symlink to something gone still says a checkout was
/// here, as a damaged `.git` directory does: what is beside it may be the
/// only copy of somebody's work, and is left.
#[test]
#[ignore = "bug: a dangling .git symlink is not seen as a checkout, so its directory is removed as stray"]
fn edge_030_a_dangling_git_link_is_not_taken_for_a_stray_directory() {
    let env = workspace_with_stray_directory("clean_dangling_git_link", |dir| {
        std::os::unix::fs::symlink("/nonexistent/gitdir", dir.join(".git")).unwrap();
        std::fs::write(dir.join("work.txt"), "unpushed").unwrap();
    });

    let out = env.run(&["clean", "-fdx"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        env.playground.join("libs/core/work.txt").exists(),
        "{}",
        out.stdout
    );
}

/// `gc` in a root that has fetched nothing yet compacts nothing, and says
/// so rather than failing.
#[test]
fn edge_031_gc_with_no_stores_reports_nothing_collected() {
    let env = TestEnv::new("clean_gc_empty");
    env.write_config("[repos]\n");
    env.init_playground_git();
    let out = env.run(&["gc"]);
    assert!(out.success, "{}", out.stderr);
    assert!(
        out.stdout.contains("0 stores collected, 0 images dropped"),
        "{}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Errors and refusals
// ---------------------------------------------------------------------------

#[test]
fn error_018_an_option_like_pattern_is_refused() {
    let env = TestEnv::new("option_like_clean_pattern_is_refused");
    env.write_config("[clean]\nexclude = [\"--upload-pack=payload\"]\n");
    env.init_playground_git();

    let out = env.run(&["clean"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("command-line option"),
        "stderr: {}",
        out.stderr
    );

    // With -f too, before anything is removed.
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
    let out = env.run(&["clean", "-fdx"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("command-line option"),
        "stderr: {}",
        out.stderr
    );
    assert!(env.playground.join("junk.txt").exists());
}

/// A command-line pattern that is empty, or that git would read as an option,
/// is refused before anything is removed — with `-f` too.
#[test]
fn error_032_an_empty_or_option_like_command_line_pattern_is_refused() {
    let env = clean_env("clean_cli_pattern_refused");
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
    for (arg, message) in [
        ("", "-e needs a pattern"),
        (
            "--upload-pack=payload",
            "starts with '-', which git would read as a command-line option",
        ),
    ] {
        let out = env.run(&["clean", "-fdx", "-e", arg]);
        assert!(!out.success, "{}: {}", arg, out.stdout);
        assert!(out.stderr.contains(message), "{}: {}", arg, out.stderr);
        assert!(env.playground.join("junk.txt").exists(), "{}", arg);
    }
}

/// A checkout's `.gitscale.toml` that cannot be read stops the clean before
/// anything goes, in that repository or any other: an unreadable keep-list is
/// not an empty one. Broken TOML is caught when resolution reads the file; a
/// keep-list git would read as an option, when clean reads its rules.
#[test]
fn error_033_an_unreadable_checkout_config_stops_the_clean_before_anything_goes() {
    let env = clean_env("clean_bad_child_config");
    for (config, message) in [
        ("[clean\nexclude = ", "invalid TOML"),
        (
            "[clean]\nexclude = [\"--upload-pack=payload\"]\n",
            "core: cannot read its clean rules",
        ),
    ] {
        std::fs::write(env.playground.join("core/.gitscale.toml"), config).unwrap();
        std::fs::write(env.playground.join("core/junk.txt"), "x").unwrap();
        std::fs::write(env.playground.join("junk.txt"), "x").unwrap();

        let out = env.run(&["clean", "-fdx"]);
        assert!(!out.success, "{}", out.stdout);
        assert!(out.stderr.contains(message), "{}", out.stderr);
        assert!(env.playground.join("core/junk.txt").exists(), "{}", message);
        assert!(env.playground.join("junk.txt").exists(), "{}", message);
    }
}

/// A graph that cannot be resolved leaves the set of links and implicit
/// checkouts unknown, so clean refuses rather than guess.
#[test]
fn error_034_a_graph_that_cannot_be_resolved_is_refused() {
    let env = TestEnv::new("clean_resolution_fails");
    env.init_playground_git();
    conflicted_workspace(&env);

    let out = env.run(&["clean", "-fdx"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(out.stderr.contains("override conflict"), "{}", out.stderr);
    assert!(env.playground.join("junk.txt").exists());
    assert!(env.playground.join("imports/b/libs/d").is_symlink());
}

/// A checkout resolution cannot settle offline still has its links and its
/// own checkout kept: clean must not delete what it cannot account for.
#[test]
#[ignore = "bug: offline, a checkout resolution leaves unresolved has its planted links deleted by clean -fdx"]
fn error_035_links_of_a_checkout_resolution_cannot_settle_are_kept() {
    let env = TestEnv::new("clean_unresolved_slot");
    env.init_playground_git();
    let (b, d) = diamond(&env);
    env.write_config(&repos(&[
        ("imports/b", &b, ", revision = \"v1.0.0\""),
        ("imports/d", &d, ", revision = \"v1.2.0\""),
    ]));
    let ci = [("CI", "true")];
    let pull = env.run_with_env(&ci, &["sync", "--no-cache"]);
    assert!(pull.success, "{}{}", pull.stdout, pull.stderr);
    std::fs::remove_dir_all(env.playground.join(".git/gitscale/resolve")).unwrap();
    let status = env.run_with_env(&ci, &["ls"]);
    assert!(
        table_row(&status.stdout, "imports/d").contains("unresolved"),
        "{}",
        status.stdout
    );

    assert!(env.playground.join("imports/b/libs/d").is_symlink());

    let out = env.run_with_env(&ci, &["clean", "-fdx"]);
    assert!(
        env.playground.join("imports/b/libs/d").is_symlink(),
        "{}{}",
        out.stdout,
        out.stderr
    );
    assert!(env.playground.join("imports/d/VERSION").is_file());
}

/// A name that is not a declared checkout fails the clean, and nothing is
/// removed anywhere.
#[test]
fn error_036_an_unknown_name_fails_and_removes_nothing() {
    let env = clean_env("clean_unknown_name");
    std::fs::write(env.playground.join("core/junk.txt"), "x").unwrap();
    let out = env.run(&["clean", "-fdx", "core", "nosuch"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("nosuch is not a checkout of this workspace"),
        "{}",
        out.stderr
    );
    assert!(env.playground.join("core/junk.txt").exists());
}

/// A repository git cannot clean is a failure, exit 1, named on stderr; the
/// others are still cleaned.
#[test]
fn error_037_a_repo_that_cannot_be_cleaned_fails_and_the_rest_are_cleaned() {
    use std::os::unix::fs::PermissionsExt;
    let env = clean_env("clean_partial_failure");
    let locked = env.playground.join("core/locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::write(locked.join("stuck.txt"), "x").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();

    let out = gitscale(&env, &[], &["clean", "-fdx"]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.code, Some(1), "{}", out.said());
    assert!(out.stderr.contains("core:"), "{}", out.stderr);
    assert!(
        out.stderr.contains("1 repo(s) failed to clean"),
        "{}",
        out.stderr
    );
    assert!(
        !env.playground.join("junk.txt").exists(),
        "the root was not cleaned"
    );
}

/// `gc` compacts the root's own stores, and CI keeps none: it is refused
/// there, pointing at the command that compacts the CI cache.
#[test]
fn error_038_gc_is_refused_in_ci() {
    let env = TestEnv::new("clean_gc_ci");
    env.write_config("[repos]\n");
    env.init_playground_git();
    let out = env.run_with_env(&[("CI", "true")], &["gc"]);
    assert!(!out.success, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("CI keeps none; use gitscale cache compact"),
        "{}",
        out.stderr
    );
}

/// A period `gc` cannot read — a bare `m`, which is minutes to humantime
/// and months to a person, or no period at all — is refused before any image
/// is dropped, on the command line and in `[clean] keep_recent` alike.
#[test]
fn error_039_gc_refuses_a_period_it_cannot_read_before_dropping_anything() {
    let env = TestEnv::new("clean_gc_bad_period");
    let bare = env.create_bare_repo("app", "main", &[("README.md", "app")]);
    let old = layered(&env, &bare, "app v1");
    env.write_config(&entry_config(&env, &bare, "main"));
    assert!(env.run(&["sync"]).success);
    env.push_commit(&bare, "main", "README.md", "v2");
    layered(&env, &bare, "app v2");
    assert!(env.run(&["sync"]).success);
    let entry = image_store(&env);
    age(&entry, &old);
    let held = || std::fs::read_to_string(entry.join("index.json")).unwrap();

    for (period, message) in [("6m", "ambiguous"), ("soon", "invalid period")] {
        let out = env.run(&["gc", "--keep-recent", period]);
        assert!(!out.success, "{}", period);
        assert!(out.stderr.contains(message), "{}: {}", period, out.stderr);
        assert!(held().contains(&old), "{} dropped an image", period);
    }

    env.write_config(&format!(
        "[clean]\nkeep_recent = \"6m\"\n\n{}",
        entry_config(&env, &bare, "main")
    ));
    let out = env.run(&["gc"]);
    assert!(!out.success);
    assert!(out.stderr.contains("clean.keep_recent"), "{}", out.stderr);
    assert!(held().contains(&old));
}

/// `gc` cleans no working tree, so it takes no `-f`, no `-e` and no names.
#[test]
fn error_040_gc_takes_no_force_exclude_or_names() {
    let env = clean_env("clean_gc_conflicts");
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
    for args in [&["gc", "-f"][..], &["gc", "-e", "x"], &["gc", "core"]] {
        let out = gitscale(&env, &[], args);
        assert!(
            out.code.is_some_and(|c| c != 0),
            "{:?}: {}",
            args,
            out.said()
        );
        assert!(
            out.stderr.contains("unexpected argument"),
            "{:?}: {}",
            args,
            out.stderr
        );
        assert!(env.playground.join("junk.txt").exists(), "{:?}", args);
    }
}

/// `--keep-recent` is `gc`'s. Given to `clean`, the command is refused rather
/// than run as a plain clean that ignores the period — with `-f`, a clean that
/// deletes files the user meant to keep a compaction for.
#[test]
fn error_041_keep_recent_is_refused_by_clean() {
    let env = clean_env("clean_keep_recent_alone");
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
    let out = gitscale(&env, &[], &["clean", "-fdx", "--keep-recent", "30d"]);
    assert!(out.code.is_some_and(|c| c != 0), "{}", out.said());
    assert!(out.stderr.contains("--keep-recent"), "{}", out.stderr);
    assert!(env.playground.join("junk.txt").exists());
}

// ---------------------------------------------------------------------------
// git's flags
// ---------------------------------------------------------------------------

/// Untracked files, directories and ignored files in the root and a
/// checkout, for the flags to choose from.
fn flag_env(name: &str) -> TestEnv {
    let env = clean_env(name);
    for dir in [env.playground.clone(), env.playground.join("core")] {
        std::fs::write(dir.join("junk.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.join("build")).unwrap();
        std::fs::write(dir.join("build/out.o"), "x").unwrap();
        std::fs::write(dir.join("trace.log"), "x").unwrap();
        let exclude = std::path::PathBuf::from(support::git_stdout(
            &dir,
            &["rev-parse", "--git-path", "info/exclude"],
        ));
        let exclude = if exclude.is_absolute() {
            exclude
        } else {
            dir.join(exclude)
        };
        std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
        std::fs::write(&exclude, "*.log\n").unwrap();
    }
    env
}

/// `-f` alone removes untracked files; `-d` adds directories; `-x` adds
/// ignored files — in the root and in each checkout, as git's do.
#[test]
fn normal_042_d_and_x_mean_what_they_mean_to_git() {
    let env = flag_env("clean_flags_dx");
    let gone = |path: &str| !env.playground.join(path).exists();

    assert!(env.run(&["clean", "-f"]).success);
    assert!(gone("junk.txt") && gone("core/junk.txt"));
    assert!(!gone("build/out.o") && !gone("core/build/out.o"));
    assert!(!gone("trace.log") && !gone("core/trace.log"));

    assert!(env.run(&["clean", "-fd"]).success);
    assert!(gone("build") && gone("core/build"));
    assert!(!gone("trace.log") && !gone("core/trace.log"));

    assert!(env.run(&["clean", "-fx"]).success);
    assert!(gone("trace.log") && gone("core/trace.log"));
    assert!(env.playground.join("core/README.md").is_file());
    assert!(env.playground.join(".gitscale.toml").is_file());
}

/// `-X` removes only ignored files — and, as `-e` patterns are more ignored
/// files to it, never by way of what clean keeps: a checkout under an
/// ignored directory stays.
#[test]
fn normal_043_capital_x_removes_only_ignored_files_and_never_a_checkout() {
    let env = TestEnv::new("clean_flags_only_ignored");
    let bare = env.artefact_repo("svc", &[("a.txt", "x")]);
    let core = env.create_bare_repo("core", "main", &[("README.md", "core")]);
    env.write_config(&format!(
        "{}[repos]\n\
         \"meta/svc\" = {{ url = \"{}\", revision = \"main\", artefact = \"replace\" }}\n\
         \"meta/core\" = {{ url = \"{}\", revision = \"main\" }}\n",
        env.registries(),
        bare.display(),
        core.display()
    ));
    env.init_playground_git();
    std::fs::write(env.playground.join(".gitignore"), "meta/\n*.log\n").unwrap();
    assert!(env.run(&["sync"]).success);
    std::fs::write(env.playground.join("junk.txt"), "x").unwrap();
    std::fs::write(env.playground.join("trace.log"), "x").unwrap();

    let dry = env.run(&["clean", "-dX"]);
    assert!(dry.success, "{}", dry.stderr);
    assert!(
        dry.stdout.lines().any(|l| l == "    trace.log"),
        "{}",
        dry.stdout
    );
    assert!(
        !dry.stdout.lines().any(|l| l.starts_with("    meta")),
        "a checkout listed for removal: {}",
        dry.stdout
    );

    let out = env.run(&["clean", "-fdX"]);
    assert!(out.success, "{}", out.stderr);
    assert!(!env.playground.join("trace.log").exists());
    assert!(
        env.playground.join("junk.txt").exists(),
        "not ignored: kept"
    );
    assert!(env.playground.join("meta/svc/dist/a.txt").is_file());
    assert!(env.playground.join("meta/core/README.md").is_file());
}

/// With neither `-n` nor `-f` a clean only lists, and `-n` wins over `-f`.
#[test]
fn normal_044_n_lists_and_wins_over_f() {
    let env = flag_env("clean_flags_n");
    for args in [&["clean", "-dx"][..], &["clean", "-n", "-fdx"]] {
        let out = env.run(args);
        assert!(out.success, "{:?}: {}", args, out.stderr);
        assert!(out.stdout.contains("dry run"), "{:?}: {}", args, out.stdout);
        assert!(env.playground.join("junk.txt").exists(), "{:?}", args);
    }
}

/// `-q` reports only failures.
#[test]
fn normal_045_q_reports_only_failures() {
    let env = flag_env("clean_flags_q");
    let out = env.run(&["clean", "-fdxq"]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(out.stdout, "");
    assert!(!env.playground.join("junk.txt").exists());
}
