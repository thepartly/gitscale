# Test coverage review — 2 October 2026

The integration tests are now one binary, `tests/it/`, with one module per
feature and a naming scheme that carries the feature, the kind of case and a
stable id. 298 new tests cover what the review found weak or missing. 57 of
them fail today: each states the right behaviour, is marked
`#[ignore = "bug: …"]`, and is listed below.

Nothing is committed and no file under `src/` changed. This file is the
review; delete it once you have read it. The rules going forward are in
[testing.md](testing.md), and every test is listed in
[test-catalog.md](test-catalog.md).

## Numbers

| | Before | After |
|---|---:|---:|
| Integration tests | 265 in 10 binaries | 565 in 1 binary |
| — passing | 265 | 508 |
| — ignored as known bugs | 0 | 57 |
| Unit tests (`src/`, untouched) | 154 | 154 |

`cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean.
The suite passed four runs in a row with the same result, and each of the 57
ignored tests fails when run with `--ignored`.

| Feature | Before | After | Added | Known bugs |
|---|---:|---:|---:|---:|
| add | 5 | 8 | 3 | 1 |
| artefact | 43 | 66 | 23 | 6 |
| cache | 19 | 27 | 8 | 3 |
| catalog | 0 | 2 | 2 | 0 |
| check | 1 | 13 | 12 | 0 |
| ci_auth | 4 | 16 | 12 | 2 |
| ci_checkout | 10 | 12 | 2 | 1 |
| clean | 18 | 41 | 23 | 5 |
| cli | 0 | 10 | 10 | 3 |
| commit | 6 | 9 | 3 | 1 |
| config | 1 | 7 | 6 | 3 |
| develop | 13 | 37 | 24 | 4 |
| fetch | 4 | 6 | 2 | 0 |
| hook | 20 | 46 | 26 | 3 |
| links | 21 | 24 | 3 | 2 |
| post_sync | 8 | 20 | 12 | 3 |
| pull | 19 | 29 | 10 | 3 |
| push | 3 | 6 | 3 | 2 |
| registry | 8 | 30 | 22 | 5 |
| remove | 2 | 3 | 1 | 0 |
| resolution | 27 | 56 | 29 | 1 |
| skill | 5 | 13 | 8 | 2 |
| status | 9 | 33 | 24 | 2 |
| stores | 7 | 16 | 9 | 1 |
| sync | 8 | 13 | 5 | 2 |
| upgrade | 4 | 22 | 18 | 2 |
| **total** | **265** | **565** | **300** | **57** |

## What changed

- **One binary.** `tests/*.rs` and `tests/helpers/` became `tests/it/`:
  `main.rs`, one module per feature, and `support/` for helpers. The old
  `tests/helpers/mod.rs` and `registry.rs` are `support/mod.rs` and
  `support/registry.rs`. Each old file's private helpers moved, unchanged
  apart from `pub`, into a support module named for its domain
  (`support/workspace.rs` holds what was in `cli.rs`, and so on). One copy of
  `git_stdout` in `cli.rs` was dropped: it differed from the shared one only
  in its panic message.
- **Names.** `<feature>::<kind>_<id>[<variant>]_<sentence>`, where kind is
  `normal`, `edge`, `error` or `perf`, and the id is unique within the feature
  and never reused. `cargo test --test it pull::edge_` runs one kind of one
  feature. Vague names were rewritten as sentences: `pull_selective` became
  `pull::normal_003_names_select_which_entries_to_pull`. The full map is at the
  end of this file.
- **Catalog.** `tests/it/catalog.rs` checks the naming rules and keeps
  `docs/test-catalog.md` in step with the sources. It fails when the catalog is
  stale; regenerate it with
  `GITSCALE_UPDATE_CATALOG=1 cargo test --test it catalog::`.
- **Snapshots** moved from `tests/snapshots/cli__*.snap` to
  `tests/it/snapshots/it__<feature>__*.snap`. Only the `source:` header line
  changed in each.
- **Outside `tests/`:** new `docs/testing.md` and `docs/test-catalog.md`, plus
  the comments in `docker-compose.yml` and `.github/workflows/ci.yml` that
  named `tests/registry_conformance.rs`.

The changes are in the working tree only; nothing is staged.
`git add -A && git diff --cached -M --stat` shows the moves as renames.

## How I checked that no test was lost

1. Before any change I recorded `cargo test -- --list`: 265 integration tests
   and 154 unit tests, all passing.
2. After the move, a script compared every one of the 265 test functions with
   its original: attributes, doc comment and body. The only allowed
   differences were the new name, `helpers::` → `support::`, and a
   `support::<module>::` qualifier where two old files had helpers of the same
   name. All 265 matched, and so did all 86 moved helpers. `--list` showed
   exactly the 265 expected names, and all passed.
3. After the new tests were merged, a second script found every original test
   again by feature and id, and checked that its original lines are still
   there, in order. Additions were allowed. All 265 are present. The changes
   are:
   - `fetch::normal_002` was renamed, keeping its id, because its old name
     ("updates a checkout already there") was false. It now asserts that the
     checkout does not move.
   - `commit::normal_002` and `commit::error_006` gained assertions. The second
     now runs its `""` case inside a loop that also tries whitespace.
   - The three real-registry conformance tests, `registry::normal_003`,
     `normal_004` and `normal_005`, were fixed. See *Other findings*.

Several old tests were strengthened in place. Each agent's notes list the
added assertions; the largest are in `hook`, `links` (their checks passed
because the test directory's own name contains "skip" and "orphan"),
`post_sync`, `status` and `upgrade`.

## Failing tests: the bugs they show

Each test is ignored with a `bug:` reason. `cargo test --test it -- --ignored`
runs all of them. `cli::edge_006` is a race that fails about once in 40
attempts; the test loops 100 times, so it fails almost every run.

### Data loss

| Test | What goes wrong | Where |
|---|---|---|
| `sync::edge_010_never_deletes_the_workspace_after_a_dot_entry_is_removed` | An entry `"."` is accepted, and pull records the root as a checkout. Once the entry is removed, `sync` deletes the whole workspace, `.git` included. | config.rs:475, ledger.rs:78, sync.rs:121 |
| `sync::edge_009_never_removes_a_clone_the_user_made_at_a_removed_entry` | Pull records any directory with a `.git` as its own. When the entry goes, sync deletes the user's own clone, which pull had refused to touch. | ledger.rs:78, sync.rs:109-122 |
| `links::edge_020_sync_never_replaces_a_plain_directory_at_a_link_path` | A plain directory at a link path is deleted without `--force`: git run inside it answers for the parent repository. | git.rs:1015, sync.rs:167-179 |
| `links::edge_021_sync_leaves_a_symlink_the_repository_tracks` | A symlink the repository tracks is removed as an orphan. | resolve.rs:153-158, sync.rs:140 |
| `develop::error_032_stop_keeps_unpushed_commits_of_a_branch_the_checkout_is_not_on` | `develop --stop` force-deletes a topic branch with unpushed commits when the checkout is detached at its pin. | checkout.rs:488-510, develop.rs:213 |
| `develop::error_033_develop_onto_a_remote_topic_keeps_commits_made_at_the_pin` | Developing onto a topic branch the remote already has orphans commits made at the detached pin. | develop.rs:163-173 |
| `clean::edge_022_overlay_files_named_with_glob_characters_survive` | Overlay file names reach `git clean -e` unescaped, so `pages/[slug].js` is read as a glob and deleted. | clean.rs:205-210 |
| `clean::edge_023_a_checkout_named_with_glob_characters_survives_the_root_clean` | The same for checkout directories and link paths: a checkout named with `[ ]` is deleted by the root clean. | clean.rs:267-279, 316-340 |
| `clean::edge_030_a_dangling_git_link_is_not_taken_for_a_stray_directory` | A dangling `.git` symlink makes the directory "stray", and it is removed whole. | git.rs:375-377, clean.rs:175-177 |
| `clean::error_035_links_of_a_checkout_resolution_cannot_settle_are_kept` | Offline, the planted links of an unresolved checkout are deleted by `clean -f`. The refusal only catches a hard resolution error. | clean.rs:121, 316-340 |
| `hook::normal_021_global_install_keeps_each_repositorys_own_git_hooks_running` | A `--global` install never runs a repository's own `.git/hooks`: post-checkout, pre-commit and git-lfs pre-push all stop silently. docs/hooks.md says the opposite. | commands/hook.rs:61-120, 312-343 |

### Security

| Test | What goes wrong | Where |
|---|---|---|
| `pull::error_027_never_places_a_checkout_outside_the_workspace_through_a_symlink` | Directory checks are lexical, so a symlinked component in the config's path puts a checkout outside the workspace. | config.rs:475-498, checkout.rs:213 |
| `config::error_005_the_workspace_itself_or_its_git_directory_is_refused` | `.`, `./`, `.git` and `.git/hooks` are accepted as directories. | config.rs:475 |
| `artefact::error_061_a_symlink_through_a_symlink_cannot_point_out_of_the_checkout` | With `a/b -> ..`, an entry `a/b/l -> ../escaped` installs a link out of the checkout: targets are checked against the archive path. | artefact.rs:190, 620 |
| `artefact::error_062_an_overlay_never_writes_through_a_symlink_in_the_checkout` | The overlay copies through a symlinked directory and writes outside the checkout. | artefact.rs:883-898 |
| `registry::edge_015_a_plain_http_registry_off_this_machine_gets_no_bearer_token` | A Bearer token goes in clear text to a plain-HTTP registry off this machine. Only Basic is guarded. | registry.rs:356, 368 |
| `registry::edge_018a_an_upload_location_on_another_host_gets_no_token` | The cached token is sent to an upload Location on another host. | registry.rs:341-342, 600-608 |
| `registry::edge_018b_an_upload_location_on_another_host_never_gets_the_job_token` | Through a Basic challenge from that host, the CI job token is sent too. | same |
| `registry::edge_019_a_tag_list_page_on_another_host_gets_no_token` | The cached token is sent to a tag-list next page on another host. | registry.rs:496 |
| `registry::error_024_registry_errors_never_print_credentials` | Error bodies are printed verbatim; a registry that echoes headers puts the job token into the log. | registry.rs:685-700 |
| `post_sync::error_017_a_star_owner_pattern_does_not_admit_a_subgroup_named_like_the_owner` | `*/acme/*` matches `gitlab.com/evil/acme/x`; the docs say it means owner `acme` on any host. | trust.rs:205-237 |
| `post_sync::error_016_an_encoded_slash_does_not_pass_for_a_path_under_the_owner` | A `%2F`-encoded `..` passes for a path under the owner. | trust.rs:256-260, 298-304 |
| `post_sync::error_020_a_host_that_only_folds_to_an_allowed_one_is_refused` | A host spelled with U+212A (Kelvin sign) passes for one spelled with `k`. | trust.rs:206, 303 |

### Wrong result

| Test | What goes wrong | Where |
|---|---|---|
| `artefact::error_059_an_image_index_is_refused_rather_than_installed_empty` | An OCI index, or any manifest without layers, is "installed": the old files are wiped, nothing is installed, and every pull exits 0. | artefact.rs:380 |
| `artefact::edge_049_a_recreated_overlay_checkout_gets_its_build_again` | The overlay is skipped when its record names the commit, even after the checkout was deleted and re-cloned. | artefact.rs:848 |
| `artefact::edge_051_an_image_with_a_readonly_directory_installs_and_updates` | A layer with a 0555 directory cannot be unpacked: the mode is applied before the directory's contents. | artefact.rs:597, 634 |
| `artefact::edge_057_a_forced_republish_brings_the_dependencies_it_declares` | After a `--force` republish, resolution still reads the old image's config layer. | artefact.rs:1033-1038 |
| `cache::normal_020_update_warms_overlay_images` | `cache update` warms only `replace` entries. | commands/cache.rs:67 |
| `cache::edge_022_compact_keeps_the_images_resolution_just_read` | Config layers that resolution reads get no use marker, so `compact` evicts them at once. | artefact.rs:1053-1061 |
| `cache::normal_027_status_names_an_overlay_image_after_its_entry` | `cache status` shows an overlay's image under its raw cache name, in a second row. | commands/cache.rs:140 |
| `upgrade::edge_015_promotion_leaves_another_majors_entry_alone` | Promotion rewrites every root entry of the repository below the tag, across majors: v0.9.0 becomes v1.1.0. | commands/upgrade.rs:198 |
| `resolution::edge_053_a_chain_seventy_deep_settles` | A dependency chain 64 or more deep fails with "did not settle after 64 rounds". | resolution.rs:578, 685 |
| `develop::edge_022_develop_on_an_existing_topic_puts_the_checkout_on_its_branch` | After the root switches back to a topic, `develop` says "already on" while the checkout is still detached. | develop.rs:127-131 |
| `develop::edge_023_develop_in_a_new_root_worktree_of_a_topic_makes_the_checkout` | The same check means a new root worktree gets no checkout at all. | develop.rs:127-131 |
| `stores::error_015_a_store_whose_origin_was_removed_gets_it_back` | A store whose `origin` was removed never gets it back, so every fetch fails. | store.rs:316-319, git.rs:156-164 |
| `status::edge_032_a_checkout_git_cannot_read_is_not_ok` | A failed `git status` counts as clean, so an unreadable checkout shows `ok`. | git.rs:1052-1068 |
| `status::error_026_verbose_fetch_keeps_json_parseable` | `status -v --fetch --format json` prints "Fetching …" into the JSON. | commands/status.rs:64-66 |
| `push::edge_004_skips_a_topic_checkout_that_is_only_behind_its_remote` | A checkout merely behind its remote is pushed, and rejected. | git.rs:646-652 |
| `pull::edge_025_moving_an_outer_checkout_leaves_a_nested_checkouts_write_bits` | Moving an outer checkout makes a nested, developed checkout read-only. | git.rs:426-486 |
| `cli::edge_006_a_terminal_places_nested_entries_like_a_plain_run` | On a terminal, nested entries are placed in parallel and race: "already exists". | progress.rs:643, checkout.rs:213 |
| `clean::error_041_keep_recent_without_gc_is_refused` | `clean -f --keep-recent 30d` without `--gc` is accepted, the period is ignored, and a real clean runs. | lib.rs:102 |
| `hook::edge_039_install_beside_a_repositorys_own_copy_runs_gitscale_once` | Install displaces a repository's own committed gitscale hook and chains to it, so every checkout pulls twice. | commands/hook.rs:316 |
| `hook::error_042_a_refused_install_leaves_every_hook_untouched` | A refusal on post-merge comes after post-checkout was already rewritten, leaving a partial install. | commands/hook.rs:312-352 |
| `skill::edge_008_a_copy_edited_in_its_frontmatter_counts_as_edited_by_hand` | Edits to the skill's frontmatter are not detected, and are overwritten. | skill.rs:35-40, 93-104 |
| `skill::error_012_remove_with_a_foreign_copy_removes_nothing` | `skill remove` deletes one copy, then fails on the other. | skill.rs:160-184 |
| `ci_auth::edge_010_a_ci_server_under_a_path_keeps_the_path` | The path in `CI_SERVER_URL` is dropped, so a GitLab under a relative URL root gets the wrong URLs. | ci.rs:237-248 |
| `commit::edge_008_does_not_commit_the_links_gitscale_planted` | On a topic, `commit` runs `git add -A` and commits the planted dependency links. Status treats them as nobody's work. The docs could be read either way. | git.rs:726, commit.rs:55 |

### Messages and usability

| Test | What goes wrong | Where |
|---|---|---|
| `cli::normal_003_root_option_before_the_subcommand_is_accepted` | `gitscale -C <dir> status`, as docs/cli.md shows it, is a usage error. | lib.rs:52 onwards |
| `cli::error_009_a_nonexistent_root_path_is_named` | A bad `-C` path gives only "cannot resolve start path". | config.rs:292, lib.rs:384 |
| `config::error_007_a_refused_value_says_why` | Refused config values name only the key; the reason is lost. | config.rs:651, 692, 757 |
| `config::error_006_two_entries_naming_one_directory_are_refused` | `libs/x`, `libs/x/` and `./libs/x` load as three entries. | config.rs:678-721 |
| `add::error_008_refuses_a_directory_already_declared_under_another_spelling` | The same, through `add`. | add.rs:39 |
| `pull::edge_029_a_name_with_a_trailing_slash_selects_its_entry` | `pull libs/lib1/`, as shell completion writes it, is "Unknown repos". | config.rs:405 |
| `push::error_005b_a_rejected_push_says_it_was_rejected` | A rejected push reports only "failed to push some refs". | git.rs:117-124 |
| `ci_auth::error_016_a_403_in_a_repository_name_is_not_a_refused_token` | Any "403" in git's output, a repository name included, gets the job-token hint. | git.rs:133 |
| `ci_checkout::error_012_a_gitlab_hook_reports_why_the_clean_check_could_not_run` | Any error from the runner-clean check, a bad config included, is reported as `GIT_CLEAN_FLAGS` deleting the checkouts. | commands/hook.rs:715-723 |

## Decisions for you

These tests pin the current behaviour, because the docs do not say what is
intended. Each doc comment says that it pins.

- **Data loss decision:** `clean::edge_024` — removing a stray entry directory
  also deletes a clone the user made inside it. This matches clean.md but
  contradicts "nested repositories are reported, not deleted".
- **The merge gate on stale data:** `check::edge_010` — when a fetch fails,
  `check` falls back to what was fetched before and can pass, with only a
  warning. Should it fail closed?
- **Status exit codes:** `status::error_025` — status exits 0 when resolution
  fails as a whole.
- **Unlinked clones:** `status::edge_023` — an unlinked clone also makes its
  owner `dirty`.
- **Commits on a detached root:** `develop::edge_028` — `commit` on a detached
  root commits it on no branch.
- **Dropping a newer major:** `develop::edge_029` — the older major's checkout
  moves onto the topic's own branch, which was cut from the newer major.
- **A deleted followed branch:** `develop::edge_026` — once the remote deletes
  the followed branch, `--stop` refuses until the branch is deleted by hand.
- **A store missing its HEAD:** `stores::error_014` — every pull fails with
  "cannot move the new store into …" and never recovers.
- **Partial commits:** `commit::error_009` — when one checkout's commit fails,
  the root is still committed and the command exits 1.
- **Misspelt keys:** `config::edge_003` — a key such as `revison` is silently
  ignored. Should unknown keys in `[repos]` warn?
- **Uninstall after `--force`:** `hook::error_040b` — uninstall after a global
  `--force` install unsets `core.hooksPath` instead of restoring the old value.
- **Shadowed local installs:** `hook::edge_033` — a `--local` install is
  silently ineffective under a global `core.hooksPath`; only `hook status`
  shows it.
- **Path patterns:** `post_sync::edge_013` — a path pattern admits any
  repository under it, remote or not. The docs say path patterns are for
  workspaces without a remote.
- **Skill downgrades:** `skill::edge_013` — an explicit `skill install`
  downgrades a copy a newer gitscale wrote.
- **Whiteouts:** `artefact::edge_052` — whiteout files in foreign images are
  unpacked literally, and the deleted file stays.
- **Semver and calver together:** `resolution::normal_032` — a repository
  that moves to calendar versions at its next major gets a checkout per major.
- **Not tested, a rule is needed:**
  - The pre-release rule in promote.rs:43 allows `v1.0.0-rc.1` → `v1.3.0-beta`.
    npm and Cargo, which the docs cite, allow only pre-releases of the same
    `x.y.z`.
  - `MAX_ROUNDS = 64`: raise it, scale it with the graph, or document it.
  - `status --why` refuses a link path that `upgrade` accepts.
  - `stale` can never show for gitscale's own CI checkouts, which are always
    detached.
  - The JSON from `status` has no ref-mismatch, unlinked or modified field.

## Other findings

- **The real-registry conformance tests are broken at HEAD (05df7b1) and fixed
  in this working tree.** On a clean export of HEAD, all four fail when
  `GITSCALE_TEST_REGISTRY` is set:
  - their fixture used `root = "dist"`, which the config now refuses;
  - they read files where they no longer land;
  - they expected 2 image layers, not 3.

  `.github/workflows/ci.yml` sets that variable, so CI's test step should have
  been failing. They are fixed now, and all four pass against the real
  `registry:2` (`GITSCALE_TEST_REGISTRY=registry:5000`; `localhost:5000` is
  refused from the sandbox and the devbox).
- **A test could rewrite your installed skill.** `hook`'s `cli()` helper
  called `gitscale::run_cli`, which treats a terminal as interactive, so on a
  terminal the hook tests refreshed the agent skill in your real `$HOME`. It
  now calls `run_cli_with(.., false)`. Interactive behaviour is tested only
  through a pty, `script`, with `HOME` pointed at a temporary directory.
- **`--system` hooks cannot be tested safely.** `hook install --system` writes
  `/etc/gitscale/hooks`, which holds real shims on this machine, and
  `git config --system` ignores `GIT_CONFIG_NOSYSTEM`. Only read-only status is
  tested, with `GIT_CONFIG_SYSTEM` pointed at a temporary file. A
  `GITSCALE_SYSTEM_HOOKS_DIR`-style override would make the rest testable.
- **Docs that disagree with the code:**
  - docs/cli.md shows `-C` before the subcommand.
  - docs/hooks.md says a global shim runs each repository's own hook.
  - clean.md has a stale note about a root that is not a git repository.
  - cli.md §remove says "delete it yourself", but sync removes the checkout.
  - dependencies.md says planted links make a child `dirty`; status shows
    `untracked-links`.
  - status.md says an artefact's `remote` is null until a fetch, but pull
    records it too.
- **Suspected but not bugs:** these were checked and hold, and the tests now
  pin them:
  - a store is fetched once per command, however many rounds resolution takes;
  - the job token never reaches another host or the disk, and an inherited
    `credential.helper=store` never receives it;
  - an interrupted artefact update keeps the old install;
  - status without `--fetch` makes no network calls;
  - the hook recursion guard holds end to end;
  - tracked files win over an overlay;
  - bad manifests are refused.

## Not covered

- **Unit-level checks** — status icon and colour precedence, version-parser
  corner cases, the cost of resolution on dense graphs. They need unit tests in
  `src/` or timing, and `src/` was off-limits.
- **`hook install` / `uninstall --system`** — no seam (see above).
- **Races that cannot be made deterministic** — `compact` against a running
  install, worktree adds sharing a name.
- **An artefact config layer that cannot be fetched**, and offline
  `follow_commit` swallowing errors — no reliable way to build the state.
- **Interactive mode's unbounded thread count** — no documented bound to test
  against.

## Appendix: old name → new name

| Before | After |
|---|---|
| `tests/artefact.rs` `a_branch_name_matches_exactly` | `artefact::edge_021_a_branch_name_matches_exactly` |
| `tests/artefact.rs` `a_ci_job_logs_in_with_its_job_token` | `registry::normal_001_a_ci_job_logs_in_with_its_job_token` |
| `tests/artefact.rs` `a_damaged_blob_is_downloaded_again` | `artefact::edge_027_a_damaged_blob_is_downloaded_again` |
| `tests/artefact.rs` `a_deleted_checkout_is_cloned_again` | `artefact::edge_025_a_deleted_checkout_is_cloned_again` |
| `tests/artefact.rs` `a_different_build_of_a_published_commit_needs_force` | `artefact::error_028_a_different_build_of_a_published_commit_needs_force` |
| `tests/artefact.rs` `a_docker_login_is_used_outside_ci` | `registry::normal_002_a_docker_login_is_used_outside_ci` |
| `tests/artefact.rs` `a_dry_run_lists_files_without_a_registry` | `artefact::edge_019_a_dry_run_lists_files_without_a_registry` |
| `tests/artefact.rs` `a_dry_run_lists_the_layers_and_sends_nothing` | `artefact::normal_002_a_dry_run_lists_the_layers_and_sends_nothing` |
| `tests/artefact.rs` `a_full_sha_is_used_as_given` | `artefact::normal_004_a_full_sha_is_used_as_given` |
| `tests/artefact.rs` `a_group_that_matches_nothing_fails_the_publish` | `artefact::error_029_a_group_that_matches_nothing_fails_the_publish` |
| `tests/artefact.rs` `a_pull_downloads_only_the_layer_that_changed` | `artefact::perf_039_a_pull_downloads_only_the_layer_that_changed` |
| `tests/artefact.rs` `a_refusal_says_how_to_get_access` | `registry::error_007_a_refusal_says_how_to_get_access` |
| `tests/artefact.rs` `a_republished_commit_shows_as_changed_and_pull_takes_it` | `artefact::normal_010_a_republished_commit_shows_as_changed_and_pull_takes_it` |
| `tests/artefact.rs` `a_second_root_worktree_downloads_nothing` | `artefact::perf_041_a_second_root_worktree_downloads_nothing` |
| `tests/artefact.rs` `an_abbreviated_sha_is_refused_with_directions` | `artefact::error_032_an_abbreviated_sha_is_refused_with_directions` |
| `tests/artefact.rs` `an_all_hex_tag_name_is_a_tag` | `artefact::edge_020_an_all_hex_tag_name_is_a_tag` |
| `tests/artefact.rs` `an_annotated_tag_resolves_to_its_commit` | `artefact::normal_003_an_annotated_tag_resolves_to_its_commit` |
| `tests/artefact.rs` `an_identical_republish_uploads_nothing` | `artefact::perf_038_an_identical_republish_uploads_nothing` |
| `tests/artefact.rs` `an_old_style_checkout_is_replaced_on_pull` | `artefact::edge_024_an_old_style_checkout_is_replaced_on_pull` |
| `tests/artefact.rs` `cache_compact_drops_cold_commits_and_their_blobs` | `cache::normal_010_compact_drops_cold_images_and_their_blobs` |
| `tests/artefact.rs` `cache_status_has_an_images_column` | `cache::normal_013_status_has_an_images_column` |
| `tests/artefact.rs` `cache_update_warms_without_a_checkout` | `cache::normal_006_update_warms_images_without_a_checkout` |
| `tests/artefact.rs` `changing_the_configured_revision_is_a_ref_mismatch` | `artefact::normal_012_changing_the_configured_revision_is_a_ref_mismatch` |
| `tests/artefact.rs` `clean_gc_drops_cold_images_and_their_blobs` | `clean::normal_008_gc_drops_cold_images_and_their_blobs` |
| `tests/artefact.rs` `in_ci_without_the_cache_every_layer_is_downloaded` | `artefact::perf_042_in_ci_without_the_cache_every_layer_is_downloaded` |
| `tests/artefact.rs` `list_of_a_repository_nothing_published_for` | `artefact::edge_026_list_of_a_repository_nothing_published_for` |
| `tests/artefact.rs` `list_shows_every_published_commit_with_its_refs` | `artefact::normal_015_list_shows_every_published_commit_with_its_refs` |
| `tests/artefact.rs` `no_revision_follows_the_default_branch` | `artefact::normal_005_no_revision_follows_the_default_branch` |
| `tests/artefact.rs` `parallel_cold_ci_jobs_download_each_blob_once` | `artefact::perf_043_parallel_cold_ci_jobs_download_each_blob_once` |
| `tests/artefact.rs` `publish_refuses_files_that_break_the_artefact_policy` | `artefact::error_031_publish_refuses_files_that_break_the_artefact_policy` |
| `tests/artefact.rs` `publish_tags_the_commit_and_annotates_the_image` | `artefact::normal_001_publish_tags_the_commit_and_annotates_the_image` |
| `tests/artefact.rs` `publishing_needs_an_artefact_table` | `artefact::error_030_publishing_needs_an_artefact_table` |
| `tests/artefact.rs` `pull_prunes_cold_images_once_a_day` | `artefact::normal_016_pull_prunes_cold_images_once_a_day` |
| `tests/artefact.rs` `records_live_in_the_git_directory_not_the_checkout` | `artefact::normal_013_records_live_in_the_git_directory_not_the_checkout` |
| `tests/artefact.rs` `show_and_list_refuse_an_entry_that_is_not_an_artefact` | `artefact::error_037_show_and_list_refuse_an_entry_that_is_not_an_artefact` |
| `tests/artefact.rs` `show_reports_an_entry_it_cannot_look_up_and_fails` | `artefact::error_036_show_reports_an_entry_it_cannot_look_up_and_fails` |
| `tests/artefact.rs` `show_says_what_the_registry_and_the_checkout_hold` | `artefact::normal_014_show_says_what_the_registry_and_the_checkout_hold` |
| `tests/artefact.rs` `status_json_carries_the_installed_commit_and_digest` | `artefact::normal_011_status_json_carries_the_installed_commit_and_digest` |
| `tests/artefact.rs` `status_says_behind_and_missing_after_a_fetch` | `artefact::normal_009_status_says_behind_and_missing_after_a_fetch` |
| `tests/artefact.rs` `the_job_token_never_goes_to_a_token_service_elsewhere` | `registry::edge_006_the_job_token_never_goes_to_a_token_service_elsewhere` |
| `tests/cache.rs` `a_developer_pull_touches_no_cache` | `cache::normal_002_a_developer_pull_touches_no_cache` |
| `tests/cache.rs` `a_job_is_unaffected_by_the_entry_being_deleted` | `cache::edge_016_a_job_is_unaffected_by_the_entry_being_deleted` |
| `tests/cache.rs` `a_second_job_re_pins_when_the_head_has_moved` | `cache::normal_004_a_second_job_re_pins_when_the_head_has_moved` |
| `tests/cache.rs` `a_sha_pinned_entry_needs_no_ref_advertisement` | `cache::edge_017_a_sha_pinned_entry_needs_no_ref_advertisement` |
| `tests/cache.rs` `an_unknown_period_is_refused_before_anything_is_deleted` | `cache::error_019_an_unknown_period_is_refused_before_anything_is_deleted` |
| `tests/cache.rs` `cache_compact_drops_stale_pins_from_an_entry_it_keeps` | `cache::normal_009_compact_drops_stale_pins_from_an_entry_it_keeps` |
| `tests/cache.rs` `cache_compact_evicts_what_nothing_has_used` | `cache::normal_008_compact_evicts_what_nothing_has_used` |
| `tests/cache.rs` `cache_status_lists_every_revision_a_snapshot_holds` | `cache::normal_012_status_lists_every_revision_a_snapshot_holds` |
| `tests/cache.rs` `cache_status_names_entries_after_the_repos_that_declare_them` | `cache::normal_011_status_names_entries_after_the_repos_that_declare_them` |
| `tests/cache.rs` `cache_update_warms_a_repo_nobody_has_pulled` | `cache::normal_005_update_warms_a_repo_nobody_has_pulled` |
| `tests/cache.rs` `ci_pins_an_annotated_tag_at_its_commit` | `cache::edge_014_ci_pins_an_annotated_tag_at_its_commit` |
| `tests/cache.rs` `ci_pins_the_branch_named_and_not_one_ending_in_the_name` | `cache::edge_015_ci_pins_the_branch_named_and_not_one_ending_in_the_name` |
| `tests/cache.rs` `ci_takes_a_pinned_commit_out_of_a_snapshot_entry` | `cache::normal_001_ci_takes_a_pinned_commit_out_of_a_snapshot_entry` |
| `tests/cache.rs` `compact_works_from_outside_any_workspace` | `cache::edge_018_compact_works_from_outside_any_workspace` |
| `tests/cache.rs` `no_cache_in_ci_fetches_the_commit_from_the_remote` | `cache::normal_003_no_cache_in_ci_fetches_the_commit_from_the_remote` |
| `tests/ci_auth.rs` `another_host_is_never_offered_the_token` | `ci_auth::edge_003_another_host_is_never_offered_the_token` |
| `tests/ci_auth.rs` `entry_on_another_host_keeps_its_ssh_url` | `ci_auth::edge_004_an_entry_on_another_host_keeps_its_ssh_url` |
| `tests/ci_auth.rs` `git_takes_the_token_from_the_environment` | `ci_auth::normal_001_git_takes_the_token_from_the_environment` |
| `tests/ci_auth.rs` `ssh_entry_on_the_ci_server_is_cloned_over_https` | `ci_auth::normal_002_an_ssh_entry_on_the_ci_server_is_cloned_over_https` |
| `tests/ci_checkout.rs` `a_ci_pull_cleans_each_checkout_as_clean_f_does` | `ci_checkout::normal_003_a_ci_pull_cleans_each_checkout_as_clean_f_does` |
| `tests/ci_checkout.rs` `a_ci_pull_leaves_each_checkout_as_a_fresh_clone_would` | `ci_checkout::normal_002_a_ci_pull_leaves_each_checkout_as_a_fresh_clone_would` |
| `tests/ci_checkout.rs` `a_gitlab_hook_pull_fails_when_the_runner_would_delete_the_checkouts` | `ci_checkout::error_009_a_gitlab_hook_pull_fails_when_the_runner_would_delete_the_checkouts` |
| `tests/ci_checkout.rs` `a_kept_checkout_follows_every_pin_change_without_the_cache` | `ci_checkout::normal_001_a_kept_checkout_follows_every_pin_change_without_the_cache` |
| `tests/ci_checkout.rs` `a_pin_the_remote_does_not_have_fails_instead_of_staying_put` | `ci_checkout::error_008_a_pin_the_remote_does_not_have_fails_instead_of_staying_put` |
| `tests/ci_checkout.rs` `an_exclude_covering_only_some_checkouts_names_the_rest` | `ci_checkout::error_010_an_exclude_covering_only_some_checkouts_names_the_rest` |
| `tests/ci_checkout.rs` `an_exclude_that_keeps_the_checkouts_passes` | `ci_checkout::normal_004_an_exclude_that_keeps_the_checkouts_passes` |
| `tests/ci_checkout.rs` `outside_ci_a_pull_leaves_untracked_files_alone` | `ci_checkout::edge_005_outside_ci_a_pull_leaves_untracked_files_alone` |
| `tests/ci_checkout.rs` `outside_gitlab_the_clean_flags_are_not_consulted` | `ci_checkout::edge_007_outside_gitlab_the_clean_flags_are_not_consulted` |
| `tests/ci_checkout.rs` `the_runner_default_clean_flags_are_checked_too` | `ci_checkout::edge_006_the_runner_default_clean_flags_are_checked_too` |
| `tests/cli.rs` `a_config_outside_a_git_repository_is_refused` | `config::error_001_a_config_outside_a_git_repository_is_refused` |
| `tests/cli.rs` `a_corrupt_artefact_is_not_taken_as_current` | `artefact::error_035_a_corrupt_artefact_is_not_taken_as_current` |
| `tests/cli.rs` `a_revision_a_dependency_asks_for_leaves_the_workspace_repo_alone` | `resolution::edge_017_a_revision_a_dependency_asks_for_leaves_the_workspace_repo_alone` |
| `tests/cli.rs` `a_second_pull_leaves_a_checkout_where_it_is` | `pull::normal_002_a_second_pull_leaves_a_checkout_where_it_is` |
| `tests/cli.rs` `a_store_maps_the_remote_branches_to_remote_tracking_refs` | `stores::normal_003_a_store_maps_the_remote_branches_to_remote_tracking_refs` |
| `tests/cli.rs` `a_tag_a_dependency_asks_for_lands_detached_and_readonly` | `resolution::normal_003_a_tag_a_dependency_asks_for_lands_detached_and_readonly` |
| `tests/cli.rs` `add_duplicate_fails` | `add::error_004_refuses_a_directory_already_declared` |
| `tests/cli.rs` `add_entry` | `add::normal_001_adds_an_entry_and_reports_it` |
| `tests/cli.rs` `add_entry_artefact` | `add::normal_002_adds_an_artefact_entry` |
| `tests/cli.rs` `add_preserves_clean_rules` | `add::edge_003_keeps_the_clean_table` |
| `tests/cli.rs` `add_refuses_an_entry_the_config_would_reject` | `add::error_005_refuses_an_entry_the_config_would_reject` |
| `tests/cli.rs` `an_abbreviated_sha_fails_with_a_hint` | `pull::error_014_an_abbreviated_sha_fails_with_a_hint` |
| `tests/cli.rs` `an_all_hex_tag_name_is_a_tag` | `pull::edge_007_an_all_hex_tag_name_is_a_tag` |
| `tests/cli.rs` `an_uncommitted_workspace_config_survives_its_own_clean` | `clean::edge_009_an_uncommitted_workspace_config_survives_its_own_clean` |
| `tests/cli.rs` `artefact_is_readonly_at_every_depth` | `artefact::edge_022_an_installed_image_is_readonly_at_every_depth` |
| `tests/cli.rs` `clean_dry_run_removes_nothing` | `clean::normal_002_a_dry_run_lists_and_removes_nothing` |
| `tests/cli.rs` `clean_keeps_a_checkout_nested_inside_another_repo` | `clean::edge_013_keeps_a_checkout_nested_inside_another` |
| `tests/cli.rs` `clean_keeps_managed_symlinks` | `clean::normal_004_keeps_managed_symlinks` |
| `tests/cli.rs` `clean_leaves_a_directory_whose_git_is_broken` | `clean::edge_015_leaves_a_directory_whose_git_is_broken` |
| `tests/cli.rs` `clean_leaves_a_stray_directory_holding_another_checkout` | `clean::edge_016_leaves_a_stray_directory_holding_another_checkout` |
| `tests/cli.rs` `clean_names_select_repos_and_the_root` | `clean::normal_006_names_select_repos_and_dot_the_root` |
| `tests/cli.rs` `clean_removes_a_directory_holding_no_repository_so_pull_works` | `clean::edge_014_removes_a_directory_holding_no_repository_so_pull_works` |
| `tests/cli.rs` `clean_removes_untracked_and_keeps_checkouts` | `clean::normal_001_force_removes_untracked_files_and_keeps_checkouts` |
| `tests/cli.rs` `clean_skips_artefact_repos` | `clean::normal_007_skips_artefact_checkouts` |
| `tests/cli.rs` `clean_works_in_a_readonly_repo` | `clean::edge_012_works_in_a_readonly_checkout` |
| `tests/cli.rs` `cli_exclude_applies_to_every_repo` | `clean::normal_005_a_command_line_exclude_applies_to_every_repo` |
| `tests/cli.rs` `commit_commits_dirty_repo` | `commit::normal_001_commits_every_change_in_a_dirty_checkout` |
| `tests/cli.rs` `commit_does_not_reach_the_workspace_through_an_empty_directory` | `commit::edge_005_does_not_reach_the_workspace_through_an_empty_directory` |
| `tests/cli.rs` `commit_rejects_empty_message` | `commit::error_006_refuses_an_empty_message` |
| `tests/cli.rs` `commit_skips_a_checkout_off_the_topic_and_says_how_to_bring_it_in` | `commit::normal_004_skips_a_checkout_off_the_topic_and_says_how_to_bring_it_in` |
| `tests/cli.rs` `commit_skips_clean_repo` | `commit::normal_002_skips_a_clean_checkout` |
| `tests/cli.rs` `commit_then_push_propagates` | `commit::normal_003_then_push_reaches_the_remote` |
| `tests/cli.rs` `fetch_artefact_local` | `artefact::normal_008_fetch_records_what_the_registry_holds_and_installs_nothing` |
| `tests/cli.rs` `fetch_git_cloned` | `fetch::normal_002_refreshes_the_store_of_a_checkout_and_leaves_the_checkout` |
| `tests/cli.rs` `fetch_git_not_cloned` | `fetch::normal_001_fills_the_store_whether_or_not_the_checkout_exists` |
| `tests/cli.rs` `hooks_post_sync` | `post_sync::normal_001_runs_for_an_allowed_repository` |
| `tests/cli.rs` `hooks_post_sync_config_cannot_allowlist_itself` | `post_sync::edge_003_a_config_cannot_allowlist_itself` |
| `tests/cli.rs` `hooks_post_sync_empty_allowlist_refuses_everything` | `post_sync::edge_006_an_empty_allowlist_refuses_everything` |
| `tests/cli.rs` `hooks_post_sync_failure` | `post_sync::error_007_a_failing_command_fails_the_pull` |
| `tests/cli.rs` `hooks_post_sync_local_workspace_matches_on_path` | `post_sync::edge_005_a_workspace_without_a_remote_matches_on_its_path` |
| `tests/cli.rs` `hooks_post_sync_lookalike_owner_is_refused` | `post_sync::edge_004_a_lookalike_owner_is_refused` |
| `tests/cli.rs` `hooks_post_sync_refused_for_unlisted_repo` | `post_sync::error_008_is_refused_for_an_unlisted_repository` |
| `tests/cli.rs` `hooks_post_sync_runs_when_invoked_directly` | `post_sync::normal_002_runs_when_the_user_invoked_gitscale` |
| `tests/cli.rs` `multiple_repos_mixed` | `pull::normal_004_pulls_checkouts_and_artefacts_together` |
| `tests/cli.rs` `option_like_clean_pattern_is_refused` | `clean::error_018_an_option_like_pattern_is_refused` |
| `tests/cli.rs` `pull_after_fetch_still_downloads_the_artefact` | `artefact::edge_023_pull_after_fetch_still_downloads_the_artefact` |
| `tests/cli.rs` `pull_artefact_local` | `artefact::normal_007_pull_takes_a_newly_published_image` |
| `tests/cli.rs` `pull_artefact_replace_installs_the_image` | `artefact::normal_006_pull_installs_a_replace_image` |
| `tests/cli.rs` `pull_artefact_up_to_date` | `artefact::perf_040_an_up_to_date_pull_asks_the_registry_nothing` |
| `tests/cli.rs` `pull_artefact_with_nothing_published` | `artefact::error_034_pull_fails_when_nothing_is_published` |
| `tests/cli.rs` `pull_artefact_without_a_registry` | `artefact::error_033_pull_without_a_registry_says_how_to_add_one` |
| `tests/cli.rs` `pull_clones_into_an_empty_directory_and_leaves_the_workspace_alone` | `pull::edge_010_clones_into_an_empty_directory_and_leaves_the_workspace_alone` |
| `tests/cli.rs` `pull_fails_when_local_changes_block_the_move` | `pull::error_017_fails_when_local_changes_block_the_move` |
| `tests/cli.rs` `pull_fails_when_the_remote_is_unreachable` | `pull::error_016_fails_when_the_remote_is_unreachable` |
| `tests/cli.rs` `pull_ignores_an_inherited_git_dir` | `pull::edge_008_ignores_an_inherited_git_dir` |
| `tests/cli.rs` `pull_inside_a_child_leaves_the_outer_workspace_link_alone` | `links::edge_016_pull_inside_a_child_leaves_the_outer_workspace_link_alone` |
| `tests/cli.rs` `pull_makes_each_checkout_a_detached_readonly_worktree_of_the_root_store` | `pull::normal_001_makes_each_checkout_a_detached_readonly_worktree_of_the_root_store` |
| `tests/cli.rs` `pull_moves_a_checkout_to_a_branch_made_after_it` | `pull::normal_005_moves_a_checkout_to_a_branch_made_after_it` |
| `tests/cli.rs` `pull_moves_to_a_revision_a_dependency_starts_asking_for` | `resolution::normal_004_pull_moves_to_a_revision_a_dependency_starts_asking_for` |
| `tests/cli.rs` `pull_refuses_a_directory_that_holds_something_else` | `pull::error_018_refuses_a_directory_that_holds_something_else` |
| `tests/cli.rs` `pull_refuses_to_lose_commits_made_at_a_pin` | `pull::edge_009_refuses_to_lose_commits_made_at_a_pin` |
| `tests/cli.rs` `pull_replaces_an_in_workspace_symlink_with_a_clone` | `links::edge_018_pull_replaces_an_in_workspace_symlink_with_a_clone` |
| `tests/cli.rs` `pull_selective` | `pull::normal_003_names_select_which_entries_to_pull` |
| `tests/cli.rs` `pull_unknown_name` | `pull::error_015_an_unknown_name_is_refused` |
| `tests/cli.rs` `pull_without_a_revision_follows_the_default_branch` | `pull::normal_006_without_a_revision_follows_the_default_branch` |
| `tests/cli.rs` `push_skip_artefact` | `push::normal_002_skips_an_artefact` |
| `tests/cli.rs` `push_skips_a_detached_tag_pin` | `push::edge_003_skips_a_detached_tag_pin` |
| `tests/cli.rs` `push_skips_checkouts_off_the_topic` | `push::normal_001_skips_checkouts_off_the_topic` |
| `tests/cli.rs` `recursive_artefact_with_config` | `resolution::normal_006_an_artefact_config_layer_declares_dependencies` |
| `tests/cli.rs` `recursive_basic_symlink` | `links::normal_001_a_dependency_of_a_dependency_links_to_the_root_checkout` |
| `tests/cli.rs` `recursive_disabled` | `resolution::edge_016_recursive_false_leaves_a_dependencys_config_unread` |
| `tests/cli.rs` `recursive_false_is_cleaned_without_its_own_keep_list` | `clean::edge_011_recursive_false_is_cleaned_without_its_own_keep_list` |
| `tests/cli.rs` `recursive_missing_dep_errors` | `resolution::normal_005_an_undeclared_dependency_is_checked_out_implicitly_where_allowed` |
| `tests/cli.rs` `recursive_revision_conflict` | `resolution::error_024_two_branches_nothing_orders_fail_the_pull` |
| `tests/cli.rs` `recursive_revision_from_a_dependency` | `resolution::normal_001_a_root_entry_without_a_revision_takes_a_dependencys` |
| `tests/cli.rs` `recursive_root_revision_wins` | `resolution::normal_002_the_root_revision_wins` |
| `tests/cli.rs` `remove_entry` | `remove::normal_001_removes_an_entry_and_keeps_the_rest` |
| `tests/cli.rs` `remove_unknown_fails` | `remove::error_002_refuses_an_undeclared_directory` |
| `tests/cli.rs` `root_clean_excludes_do_not_reach_subrepos` | `clean::edge_010_root_clean_excludes_do_not_reach_subrepos` |
| `tests/cli.rs` `status_artefact_missed` | `status::normal_004_table_shows_a_missing_artefact` |
| `tests/cli.rs` `status_artefact_ok` | `status::normal_005_table_shows_an_installed_artefact` |
| `tests/cli.rs` `status_fetch_reports_a_fetch_it_could_not_do` | `status::error_009_fetch_reports_a_fetch_it_could_not_do` |
| `tests/cli.rs` `status_flags_a_pin_the_checkout_has_not_followed` | `status::normal_006_flags_a_pin_the_checkout_has_not_followed` |
| `tests/cli.rs` `status_json` | `status::normal_003_json_lists_each_checkout` |
| `tests/cli.rs` `status_json_with_no_repos_is_json` | `status::edge_008_json_with_no_repos_is_json` |
| `tests/cli.rs` `status_shows_orphan` | `links::normal_013_status_shows_an_orphan` |
| `tests/cli.rs` `status_shows_unlinked` | `links::normal_009_status_shows_an_unlinked_clone` |
| `tests/cli.rs` `status_shows_unlinked_modified` | `links::normal_010_status_shows_an_unlinked_clone_as_modified` |
| `tests/cli.rs` `status_shows_unlinked_under_a_nested_entry` | `links::edge_011_status_shows_unlinked_under_a_nested_entry` |
| `tests/cli.rs` `status_table_missed` | `status::normal_001_table_shows_a_missing_checkout` |
| `tests/cli.rs` `status_table_ok` | `status::normal_002_table_shows_a_checkout_at_its_pin` |
| `tests/cli.rs` `status_tag_pinned_is_not_a_mismatch` | `status::edge_007_a_detached_tag_pin_is_not_a_mismatch` |
| `tests/cli.rs` `subrepo_clean_excludes_come_from_its_own_config` | `clean::normal_003_a_subrepo_takes_excludes_from_its_own_config` |
| `tests/cli.rs` `sync_force_relinks_clone_with_unpushed_commits` | `links::normal_004b_sync_force_relinks_a_clone_with_unpushed_commits` |
| `tests/cli.rs` `sync_force_relinks_modified_clone` | `links::normal_003b_sync_force_relinks_a_modified_clone` |
| `tests/cli.rs` `sync_force_removes_orphan_with_valid_target` | `links::normal_015b_sync_force_removes_an_orphan_with_a_valid_target` |
| `tests/cli.rs` `sync_full` | `sync::normal_001_pulls_a_fresh_workspace` |
| `tests/cli.rs` `sync_inside_a_child_keeps_the_links_and_a_named_pull_unlinks` | `links::edge_017_sync_inside_a_child_keeps_the_links_and_a_named_pull_unlinks` |
| `tests/cli.rs` `sync_relinks_clean_clone` | `links::normal_002_sync_relinks_a_clean_clone` |
| `tests/cli.rs` `sync_relinks_clone_holding_only_ignored_files` | `links::normal_007_sync_relinks_a_clone_holding_only_ignored_files` |
| `tests/cli.rs` `sync_removes_broken_orphan_by_default` | `links::normal_014_sync_removes_a_broken_orphan_by_default` |
| `tests/cli.rs` `sync_skips_clone_with_a_stash` | `links::edge_006_sync_skips_a_clone_with_a_stash` |
| `tests/cli.rs` `sync_skips_clone_with_commits_on_a_local_only_branch` | `links::edge_005_sync_skips_a_clone_with_commits_on_a_local_only_branch` |
| `tests/cli.rs` `sync_skips_clone_with_unpushed_commits` | `links::edge_004a_sync_skips_a_clone_with_unpushed_commits` |
| `tests/cli.rs` `sync_skips_modified_clone_without_force` | `links::edge_003a_sync_skips_a_modified_clone_without_force` |
| `tests/cli.rs` `sync_skips_orphan_with_valid_target_without_force` | `links::edge_015a_sync_skips_an_orphan_with_a_valid_target_without_force` |
| `tests/cli.rs` `sync_with_names_leaves_other_repos_links_alone` | `links::edge_008_sync_with_names_leaves_other_repos_links_alone` |
| `tests/hook.rs` `a_global_hook_refuses_a_payload_from_a_cloned_branch` | `hook::edge_016_a_global_hook_refuses_a_payload_from_a_cloned_branch` |
| `tests/hook.rs` `hook_run_refuses_when_the_shim_passed_no_allowlist` | `hook::error_020_run_refuses_when_the_shim_passed_no_allowlist` |
| `tests/hook.rs` `hook_run_rejects_an_unknown_hook_name` | `hook::error_017_run_rejects_an_unknown_hook_name` |
| `tests/hook.rs` `hook_status_recognises_a_repositorys_own_copy_of_the_shim` | `hook::edge_014_status_recognises_a_repositorys_own_copy_of_the_shim` |
| `tests/hook.rs` `hook_status_reports_installed_hooks` | `hook::normal_005_status_reports_installed_hooks` |
| `tests/hook.rs` `hook_status_reports_the_allowlist` | `hook::normal_008_status_reports_the_allowlist` |
| `tests/hook.rs` `hook_status_still_flags_a_repo_path_that_runs_no_gitscale` | `hook::edge_015_status_still_flags_a_repo_path_that_runs_no_gitscale` |
| `tests/hook.rs` `install_bakes_the_requested_allowlist_into_the_shim` | `hook::normal_006_install_bakes_the_requested_allowlist_into_the_shim` |
| `tests/hook.rs` `install_displaces_and_chains_an_existing_hook` | `hook::normal_002_install_displaces_and_chains_an_existing_hook` |
| `tests/hook.rs` `install_global_requires_an_allowlist` | `hook::error_018_install_global_requires_an_allowlist` |
| `tests/hook.rs` `install_local_allows_its_own_repository_by_default` | `hook::normal_007_install_local_allows_its_own_repository_by_default` |
| `tests/hook.rs` `install_local_from_a_linked_worktree_writes_the_shared_hooks` | `hook::edge_009_install_local_from_a_linked_worktree_writes_the_shared_hooks` |
| `tests/hook.rs` `install_local_writes_both_hooks` | `hook::normal_001_install_local_writes_both_hooks` |
| `tests/hook.rs` `install_refuses_a_pattern_that_would_break_the_shim` | `hook::error_019_install_refuses_a_pattern_that_would_break_the_shim` |
| `tests/hook.rs` `reinstall_keeps_the_existing_allowlist` | `hook::edge_013_reinstall_keeps_the_existing_allowlist` |
| `tests/hook.rs` `shim_chains_a_hook_whose_path_holds_a_quote` | `hook::edge_011_shim_chains_a_hook_whose_path_holds_a_quote` |
| `tests/hook.rs` `shim_ignores_a_file_checkout` | `hook::edge_012_shim_ignores_a_file_checkout` |
| `tests/hook.rs` `shim_is_silent_in_a_repo_without_a_gitscale_config` | `hook::edge_010_shim_is_silent_in_a_repo_without_a_gitscale_config` |
| `tests/hook.rs` `shim_runs_the_chained_hook_and_honours_the_recursion_guard` | `hook::normal_004_shim_runs_the_chained_hook_and_honours_the_recursion_guard` |
| `tests/hook.rs` `uninstall_restores_the_displaced_hook` | `hook::normal_003_uninstall_restores_the_displaced_hook` |
| `tests/registry_conformance.rs` `a_missing_tag_is_reported_by_a_real_registry` | `registry::error_008_a_missing_tag_is_reported_by_a_real_registry` |
| `tests/registry_conformance.rs` `gitscale_reads_what_other_tools_publish` | `registry::normal_005_gitscale_reads_what_other_tools_publish` |
| `tests/registry_conformance.rs` `other_tools_read_what_gitscale_publishes` | `registry::normal_004_other_tools_read_what_gitscale_publishes` |
| `tests/registry_conformance.rs` `publish_clone_and_pull_against_a_real_registry` | `registry::normal_003_publish_clone_and_pull_against_a_real_registry` |
| `tests/resolution.rs` `a_cycle_fails_before_anything_is_cloned` | `resolution::error_025_a_cycle_fails_before_anything_is_cloned` |
| `tests/resolution.rs` `a_dependency_raises_the_root_and_status_says_why` | `resolution::normal_007_a_dependency_raises_the_root_and_status_says_why` |
| `tests/resolution.rs` `a_missed_row_has_no_resolution_text` | `resolution::edge_023_a_missed_row_has_no_resolution_text` |
| `tests/resolution.rs` `a_move_never_overwrites_an_untracked_file` | `pull::edge_012_a_move_never_overwrites_an_untracked_file` |
| `tests/resolution.rs` `a_winner_behind_a_request_is_flagged_where_history_is_local` | `resolution::edge_022_a_winner_behind_a_request_is_flagged_where_history_is_local` |
| `tests/resolution.rs` `an_artefact_beside_the_source_of_one_repository` | `resolution::edge_021_an_artefact_beside_the_source_of_one_repository` |
| `tests/resolution.rs` `an_artefact_brings_its_dependencies_in_its_config_layer` | `resolution::normal_011_an_artefact_brings_its_dependencies_in_its_config_layer` |
| `tests/resolution.rs` `an_override_at_the_root_holds_a_dependency_down` | `resolution::normal_008_an_override_at_the_root_holds_a_dependency_down` |
| `tests/resolution.rs` `an_override_from_above_works_around_a_missing_revision` | `resolution::edge_018_an_override_from_above_works_around_a_missing_revision` |
| `tests/resolution.rs` `an_override_in_a_dependency_reaches_only_what_it_is_above` | `resolution::normal_013_an_override_in_a_dependency_reaches_only_what_it_is_above` |
| `tests/resolution.rs` `an_uncommitted_config_edit_takes_effect` | `resolution::edge_020_an_uncommitted_config_edit_takes_effect` |
| `tests/resolution.rs` `bad_resolution_config_is_refused_when_read` | `resolution::error_026_bad_resolution_config_is_refused_when_read` |
| `tests/resolution.rs` `cache_update_covers_implicit_dependencies` | `cache::normal_007_update_covers_implicit_dependencies` |
| `tests/resolution.rs` `calendar_versions_order_by_date_then_modifier` | `resolution::normal_010_calendar_versions_order_by_date_then_modifier` |
| `tests/resolution.rs` `clean_keeps_implicit_checkouts_and_their_links` | `clean::edge_017_keeps_implicit_checkouts_and_their_links` |
| `tests/resolution.rs` `fetch_fetches_the_declared_entries_when_resolution_fails` | `fetch::edge_003_fetches_the_declared_entries_when_resolution_fails` |
| `tests/resolution.rs` `fetch_reports_both_failures` | `fetch::error_004_reports_both_failures` |
| `tests/resolution.rs` `links_a_repository_does_not_ignore_are_flagged_apart_from_dirty` | `links::edge_012_links_a_repository_does_not_ignore_are_flagged_apart_from_dirty` |
| `tests/resolution.rs` `one_failing_entry_fails_the_command_after_the_rest_is_done` | `pull::error_019_one_failing_entry_fails_the_command_after_the_rest_is_done` |
| `tests/resolution.rs` `placement_follows_the_hoist_dir_majors_kind_and_names` | `resolution::normal_015_placement_follows_the_hoist_dir_majors_kind_and_names` |
| `tests/resolution.rs` `pull_does_not_move_a_detached_head_with_commits_on_no_branch` | `pull::edge_013_does_not_move_a_detached_head_with_commits_on_no_branch` |
| `tests/resolution.rs` `pull_moves_a_checkout_only_when_nothing_can_be_lost` | `pull::edge_011_moves_a_checkout_only_when_nothing_can_be_lost` |
| `tests/resolution.rs` `resolution_and_moves_in_ci` | `resolution::normal_012_in_ci_resolution_reads_each_config_from_its_commit` |
| `tests/resolution.rs` `resolving_in_ci_without_the_cache_fetches_no_history` | `resolution::perf_027_resolving_in_ci_without_the_cache_fetches_no_history` |
| `tests/resolution.rs` `status_is_unresolved_until_fetched` | `resolution::edge_019_status_is_unresolved_until_fetched` |
| `tests/resolution.rs` `sync_keeps_a_left_behind_checkout_with_work_and_fails` | `sync::edge_007_keeps_a_left_behind_checkout_with_work_and_fails` |
| `tests/resolution.rs` `sync_never_removes_a_directory_holding_a_wanted_checkout` | `sync::edge_008_never_removes_a_directory_holding_a_wanted_checkout` |
| `tests/resolution.rs` `sync_removes_a_checkout_whose_entry_was_removed` | `sync::normal_004_removes_a_checkout_whose_entry_was_removed` |
| `tests/resolution.rs` `sync_removes_a_checkout_whose_only_change_is_its_links` | `sync::edge_006_removes_a_checkout_whose_only_change_is_its_links` |
| `tests/resolution.rs` `sync_removes_a_left_behind_implicit_artefact` | `sync::normal_003_removes_a_left_behind_implicit_artefact` |
| `tests/resolution.rs` `sync_removes_an_implicit_checkout_left_behind` | `sync::normal_002_removes_an_implicit_checkout_left_behind` |
| `tests/resolution.rs` `sync_takes_an_implicit_checkout_by_name` | `sync::normal_005_takes_an_implicit_checkout_by_name` |
| `tests/resolution.rs` `two_majors_get_a_checkout_each_unless_one_is_a_singleton` | `resolution::normal_009_two_majors_get_a_checkout_each_unless_one_is_a_singleton` |
| `tests/resolution.rs` `upgrade_resolved_edits_table_style_entries` | `upgrade::edge_004_resolved_edits_table_style_entries` |
| `tests/resolution.rs` `upgrade_resolved_records_the_resolved_revisions_and_keeps_comments` | `upgrade::normal_001_resolved_records_the_resolved_revisions_and_keeps_comments` |
| `tests/resolution.rs` `why_shows_the_shared_checkouts_or_the_ones_named` | `resolution::normal_014_why_shows_the_shared_checkouts_or_the_ones_named` |
| `tests/skill.rs` `install_status_and_remove` | `skill::normal_001_install_status_and_remove` |
| `tests/skill.rs` `no_hint_once_a_skill_is_installed` | `skill::normal_003_no_hint_once_a_skill_is_installed` |
| `tests/skill.rs` `pull_and_status_stay_quiet_when_nobody_is_watching` | `skill::edge_005_pull_and_status_stay_quiet_when_nobody_is_watching` |
| `tests/skill.rs` `the_hint_shows_once_for_every_worktree_of_a_root` | `skill::normal_002_the_hint_shows_once_for_every_worktree_of_a_root` |
| `tests/skill.rs` `without_claude_code_only_the_shared_location` | `skill::edge_004_without_claude_code_only_the_shared_location` |
| `tests/worktrees.rs` `a_bare_root_shares_one_store_across_its_worktrees` | `stores::normal_002_a_bare_root_shares_one_store_across_its_worktrees` |
| `tests/worktrees.rs` `a_child_follows_its_remote_branch_of_the_topic` | `develop::normal_002_a_child_follows_its_remote_branch_of_the_topic` |
| `tests/worktrees.rs` `a_deleted_root_worktree_does_not_lock_its_topic` | `stores::edge_005_a_deleted_root_worktree_does_not_lock_its_topic` |
| `tests/worktrees.rs` `a_dependency_that_pins_the_topic_keeps_what_it_asks_for_at_its_pins` | `develop::edge_009_a_dependency_that_pins_the_topic_keeps_what_it_asks_for_at_its_pins` |
| `tests/worktrees.rs` `a_dirty_child_stays_when_the_root_leaves_the_topic` | `develop::edge_007_a_dirty_child_stays_when_the_root_leaves_the_topic` |
| `tests/worktrees.rs` `a_foreign_checkout_is_reported_and_left_alone` | `stores::edge_007_a_foreign_checkout_is_reported_and_left_alone` |
| `tests/worktrees.rs` `a_new_branch_from_a_topic_carries_its_children` | `develop::normal_003_a_new_branch_from_a_topic_carries_its_children` |
| `tests/worktrees.rs` `a_pinned_branch_follows_no_topic` | `develop::normal_005_a_pinned_branch_follows_no_topic` |
| `tests/worktrees.rs` `a_pipeline_on_a_topic_takes_children_from_their_branches` | `develop::normal_006_a_pipeline_on_a_topic_takes_children_from_their_branches` |
| `tests/worktrees.rs` `a_plain_clone_keeps_its_stores_in_its_own_git_directory` | `stores::normal_001_a_plain_clone_keeps_its_stores_in_its_own_git_directory` |
| `tests/worktrees.rs` `an_artefact_on_a_topic_is_the_tips_image_or_its_source` | `artefact::normal_017_an_artefact_on_a_topic_is_the_tips_image_or_its_source` |
| `tests/worktrees.rs` `an_empty_pinned_list_makes_the_default_branch_a_topic` | `develop::edge_008_an_empty_pinned_list_makes_the_default_branch_a_topic` |
| `tests/worktrees.rs` `an_older_major_develops_on_a_suffixed_branch` | `develop::edge_010_an_older_major_develops_on_a_suffixed_branch` |
| `tests/worktrees.rs` `an_overlay_lays_the_build_over_the_source` | `artefact::normal_018_an_overlay_lays_the_build_over_the_source` |
| `tests/worktrees.rs` `check_gates_only_merges_into_pinned_branches` | `check::normal_001_gates_only_merges_into_pinned_branches` |
| `tests/worktrees.rs` `develop_puts_a_child_on_the_topic_from_its_pin` | `develop::normal_001_puts_a_child_on_the_topic_from_its_pin` |
| `tests/worktrees.rs` `develop_refuses_an_entry_the_root_overrides` | `develop::error_012_refuses_an_entry_the_root_overrides` |
| `tests/worktrees.rs` `develop_refuses_off_a_topic` | `develop::error_011_refuses_off_a_topic` |
| `tests/worktrees.rs` `develop_stop_refuses_while_the_remote_has_the_branch` | `develop::error_013_stop_refuses_while_the_remote_has_the_branch` |
| `tests/worktrees.rs` `develop_stop_takes_a_child_back_to_its_pin` | `develop::normal_004_stop_takes_a_child_back_to_its_pin` |
| `tests/worktrees.rs` `pull_repairs_children_after_the_root_moves` | `stores::edge_006_pull_repairs_children_after_the_root_moves` |
| `tests/worktrees.rs` `status_warns_about_a_root_without_a_fetch_refspec` | `stores::edge_004_status_warns_about_a_root_without_a_fetch_refspec` |
| `tests/worktrees.rs` `upgrade_promotes_a_child_whose_change_is_released` | `upgrade::normal_002_promotes_a_child_whose_change_is_released` |
| `tests/worktrees.rs` `upgrade_raises_a_named_dependency_to_its_newest_release` | `upgrade::normal_003_raises_a_named_dependency_to_its_newest_release` |
