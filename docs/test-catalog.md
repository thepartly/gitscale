# Test catalog

Generated from the sources by `tests/it/catalog.rs` — do not edit by hand. Regenerate with `GITSCALE_UPDATE_CATALOG=1 cargo test --test it catalog::`. The naming rules are in [testing.md](testing.md).

650 integration tests (283 normal, 205 edge, 144 error, 18 perf), 44 of them ignored; 170 unit tests.

| Feature | normal | edge | error | perf | total |
|---|---:|---:|---:|---:|---:|
| [artefact](#artefact) | 26 | 14 | 25 | 7 | 72 |
| [cache](#cache) | 14 | 7 | 4 | 0 | 25 |
| [catalog](#catalog) | 2 | 0 | 0 | 0 | 2 |
| [check](#check) | 9 | 2 | 2 | 0 | 13 |
| [ci_auth](#ci_auth) | 4 | 10 | 2 | 0 | 16 |
| [ci_checkout](#ci_checkout) | 4 | 4 | 4 | 0 | 12 |
| [clean](#clean) | 15 | 18 | 11 | 0 | 44 |
| [cli](#cli) | 5 | 1 | 4 | 0 | 10 |
| [config](#config) | 1 | 1 | 5 | 0 | 7 |
| [exclude](#exclude) | 2 | 1 | 0 | 0 | 3 |
| [forward](#forward) | 22 | 1 | 3 | 1 | 27 |
| [hash](#hash) | 5 | 2 | 1 | 0 | 8 |
| [hook](#hook) | 19 | 22 | 10 | 1 | 52 |
| [links](#links) | 10 | 14 | 0 | 0 | 24 |
| [ls](#ls) | 22 | 9 | 3 | 1 | 35 |
| [man](#man) | 1 | 0 | 0 | 0 | 1 |
| [network](#network) | 2 | 0 | 1 | 0 | 3 |
| [output](#output) | 5 | 0 | 0 | 0 | 5 |
| [placement](#placement) | 6 | 14 | 8 | 1 | 29 |
| [post_sync](#post_sync) | 4 | 7 | 9 | 0 | 20 |
| [prefer](#prefer) | 2 | 1 | 1 | 0 | 4 |
| [registry](#registry) | 11 | 9 | 8 | 2 | 30 |
| [require](#require) | 3 | 4 | 6 | 0 | 13 |
| [resolution](#resolution) | 26 | 15 | 14 | 3 | 58 |
| [skill](#skill) | 5 | 5 | 3 | 0 | 13 |
| [stores](#stores) | 3 | 10 | 2 | 1 | 16 |
| [sync](#sync) | 5 | 7 | 0 | 1 | 13 |
| [topic](#topic) | 36 | 17 | 12 | 0 | 65 |
| [upgrade](#upgrade) | 12 | 9 | 5 | 0 | 26 |
| [workspace](#workspace) | 2 | 1 | 1 | 0 | 4 |

## Integration tests

### artefact

| ID | Kind | Test | What it holds |
|---|---|---|---|
| artefact-001 | normal | `normal_001_publish_tags_the_source_hash_and_annotates_the_image` | The image is tagged with the source hash of what it was built from and with the commit's version tags — never the commit — and records the commit, its tree and the hash. |
| artefact-002 | normal | `normal_002_a_dry_run_lists_the_layers_and_sends_nothing` |  |
| artefact-003 | normal | `normal_003_an_annotated_tag_is_a_release` |  |
| artefact-004 | error | `error_004_an_artefact_needs_a_release` | An artefact is only ever a release's: a commit, a branch, or no revision at all fails the checkout, naming who asks for it. |
| artefact-006 | normal | `normal_006_sync_installs_the_image_of_a_release` |  |
| artefact-007 | normal | `normal_007_sync_takes_the_image_of_a_newer_release` |  |
| artefact-008 | normal | `normal_008_fetch_records_what_the_registry_holds_and_installs_nothing` |  |
| artefact-009 | normal | `normal_009_ls_says_missing_after_a_fetch` | A release wanted that has no image yet is `missing` once a fetch has looked; published, the checkout is only at the wrong release until a sync. |
| artefact-010 | normal | `normal_010_a_republished_release_shows_as_changed_and_sync_takes_it` |  |
| artefact-011 | normal | `normal_011_ls_json_carries_the_installed_release_and_digest` |  |
| artefact-012 | normal | `normal_012_wanting_another_release_is_a_ref_mismatch` |  |
| artefact-013 | normal | `normal_013_records_live_in_the_git_directory_not_the_checkout` | What gitscale records about a checkout lives in the workspace's git directory, never in the checkout, whose every file is the artefact's. |
| artefact-014 | normal | `normal_014_show_says_what_the_registry_and_the_checkout_hold` |  |
| artefact-015 | normal | `normal_015_list_shows_every_release_with_its_source_hash` | `list` shows each release with an image, newest first, with the source hash it was built from — not the builds no release names. |
| artefact-016 | normal | `normal_016_placement_prunes_cold_images_once_a_day` | Placement prunes the image store by itself — at most once a day, so the cost on every other placement is one stat. |
| artefact-017 | normal | `normal_017_an_artefact_on_a_topic_is_its_sources` | On the topic, a checkout taken as an artefact is its sources: the topic's branch of it, followed. Off the topic, the image of its release again. |
| artefact-019 | edge | `edge_019_a_dry_run_lists_files_without_a_registry` | The way to see what would ship: a dry run lists the files even where the image cannot be worked out yet — no registry mapping, no commit. |
| artefact-022 | edge | `edge_022_an_installed_image_is_readonly_at_every_depth` | Read-only applies to the whole archive, not just its top level, dot files included — and an update still gets past the read-only files it replaces. What gitscale records about the checkout is kept outside it. |
| artefact-023 | edge | `edge_023_sync_after_fetch_still_downloads_the_artefact` | A fetch only looks: an artefact it found in the registry is still to be downloaded by the sync that follows. |
| artefact-024 | edge | `edge_024_an_old_style_checkout_is_replaced_on_sync` | A directory gitscale has no record of installing — one from an older gitscale, or made by hand — is replaced by `sync`, dot files and all. |
| artefact-025 | edge | `edge_025_a_deleted_checkout_is_cloned_again` | Deleting a checkout by hand is noticed: it is not installed any more, whatever was recorded. |
| artefact-026 | edge | `edge_026_list_of_a_repository_nothing_published_for` |  |
| artefact-027 | edge | `edge_027_a_damaged_blob_is_downloaded_again` | A blob that no longer matches its digest is never used: it is deleted and downloaded again. |
| artefact-028 | error | `error_028_a_different_build_of_a_published_commit_needs_force` |  |
| artefact-029 | error | `error_029_a_group_that_matches_nothing_fails_the_publish` |  |
| artefact-030 | error | `error_030_publishing_needs_an_artefact_table` |  |
| artefact-031 | error | `error_031_publish_refuses_files_that_break_the_artefact_policy` | Every file an image ships is either the commit's own, unmodified, or ignored build output: anything else could not be laid over a checkout of that commit, and `publish` refuses it — a dry run too. |
| artefact-032 | error | `error_032_an_abbreviated_sha_is_refused_with_directions` | An abbreviated commit is taken for a name the remote does not have, and the failure says to give the full SHA. |
| artefact-033 | error | `error_033_sync_without_a_registry_says_how_to_add_one` |  |
| artefact-034 | error | `error_034_sync_fails_when_nothing_is_published` |  |
| artefact-035 | error | `error_035_a_corrupt_artefact_is_not_taken_as_current` | An archive that cannot be unpacked leaves nothing that later passes for the artefact: running sync again tries again, rather than taking the half-made directory as done. |
| artefact-036 | error | `error_036_show_reports_an_entry_it_cannot_look_up_and_fails` |  |
| artefact-037 | normal | `normal_037_show_and_list_name_nothing_while_nothing_is_taken_as_an_artefact` |  |
| artefact-038 | perf | `perf_038_an_identical_republish_uploads_nothing` |  |
| artefact-039 | perf | `perf_039_a_sync_downloads_only_the_layer_that_changed` |  |
| artefact-040 | perf | `perf_040_an_up_to_date_sync_asks_the_registry_nothing` |  |
| artefact-041 | perf | `perf_041_a_second_root_worktree_downloads_nothing` | The root's image store is shared by its worktrees: a second one of the root installs from it, downloading nothing. |
| artefact-042 | perf | `perf_042_in_ci_without_the_cache_every_layer_is_downloaded` | A CI job without the cache keeps nothing: every layer is downloaded again, two blobs a sync. |
| artefact-043 | perf | `perf_043_parallel_cold_ci_jobs_download_each_blob_once` | N cold CI jobs on one runner, sharing its cache, download each blob once. |
| artefact-044 | normal | `normal_044_foreign_layer_media_types_unpack_in_order` | Images other tools publish unpack too: a Docker-typed gzip layer and an uncompressed OCI tar layer, in order, so a later layer's file replaces an earlier one's. gitscale's own publisher writes neither, but an artefact is an ordinary image, and the docs say any registry and tool works. |
| artefact-045 | normal | `normal_045_publish_in_a_gitlab_job_names_the_image_after_its_project` | In a GitLab job, `publish` names the image after the job's project (`CI_PROJECT_URL`), which is the URL consumers declare: under any other name no consumer would find it. |
| artefact-046 | normal | `normal_046_publish_in_a_github_workflow_names_the_image_after_its_repository` | The same in a GitHub workflow: `GITHUB_SERVER_URL`/`GITHUB_REPOSITORY` name the repository. |
| artefact-051 | edge | `edge_051_an_image_with_a_readonly_directory_installs_and_updates` | **ignored: bug: a layer with a 0555 directory cannot be unpacked, nor the checkout replaced** An image whose directories are read-only — as another tool may write them — still installs, and the next version still replaces it. gitscale makes the files read-only itself; a directory it cannot write into would leave the checkout impossible to install or update. |
| artefact-052 | edge | `edge_052_whiteout_files_are_unpacked_as_ordinary_files` | OCI whiteout files (`.wh.<name>`) are unpacked as ordinary files, and the file they delete stays. This pins current behaviour: gitscale's own images never hold whiteouts, and whether foreign multi-layer images should get OCI deletion semantics is the owner's call. |
| artefact-054 | edge | `edge_054_publish_ships_no_link_out_of_the_repository_unless_selected` | A repository holds symlinks out of itself that no artefact ships — the links gitscale plants for its own dependencies. `publish` ignores them, unless a pattern selects one: then it refuses, since such a link would point at nothing on the consumer's machine, or at something else. |
| artefact-055 | edge | `edge_055_a_dry_run_outside_git_says_what_it_could_not_check` | A dry run where there is no commit and no remote still lists every file, and says what it could not work out — the commit, the image, and so the policy — rather than failing or pretending it checked. |
| artefact-057 | edge | `edge_057_a_forced_republish_brings_the_dependencies_it_declares` | **ignored: bug: resolution reads the config in the manifest held for the release, not the republished one** A forced re-publish that changes the image's `.gitscale.toml` changes what resolution reads: after a fetch says the image changed, the sync that installs the new files also brings the dependencies the new config declares. Files from one build and dependencies from another would be a checkout nobody published. |
| artefact-058 | error | `error_058_a_registry_failing_mid_update_keeps_the_installed_version` | A registry that serves a damaged layer during an update leaves the installed version alone: everything is downloaded and checked before the old files go, as the docs promise. The installed files, the record of them and `ls` all still say the old release. |
| artefact-059 | error | `error_059_an_image_index_is_refused_rather_than_installed_empty` | **ignored: bug: a manifest without layers (an OCI index) is installed as an empty artefact** A tag that names an image index — what `docker buildx` pushes with provenance, or a multi-platform image — is not an image with no layers. Taking it for one would wipe the installed files and record an empty checkout as the artefact, and every later sync would succeed doing it again. |
| artefact-060 | error | `error_060_archive_entries_that_leave_the_checkout_are_refused` | Nothing in an archive may land outside the checkout or be anything but a file, a directory or a symlink inside it: a `..` path, an absolute path, a hard link, a FIFO, and symlinks out — relative or absolute — each fail the whole install, and leave no half-unpacked checkout behind. |
| artefact-061 | error | `error_061_a_symlink_through_a_symlink_cannot_point_out_of_the_checkout` | **ignored: bug: symlink targets are checked against the entry's path, not where it resolves** A symlink is judged by where it really resolves, not by its path in the archive: with `a/b -> ..`, an entry `a/b/l -> ../x` passes a check on its path (two levels down, one up) but lands at the top of the checkout, pointing out of it. The docs promise no symlink pointing out of the directory. |
| artefact-063 | error | `error_063_an_unknown_layer_media_type_is_refused` | A layer of a kind gitscale cannot unpack — a Helm chart, say — fails the install with the media type named, rather than being unpacked as a tar or skipped. |
| artefact-064 | error | `error_064_a_manifest_naming_an_invalid_digest_is_refused` | A manifest whose layer digest is not a well-formed sha256 digest is refused before the digest is ever used as a file name in the image store: `sha256:../../escape` would otherwise name a path outside it. |
| artefact-066 | error | `error_066_ls_fetch_without_the_registry_shows_the_last_fetched_state` | `ls --fetch` that cannot reach the registry still prints the table, says the entry's row is what the last successful fetch saw, and does not pass that off as fresh. |
| artefact-067 | normal | `normal_067_publish_releases_an_image_already_there` | A release named for a commit already published goes on its image, uploading nothing. |
| artefact-068 | normal | `normal_068_reuse_releases_the_image_of_the_same_sources` | A squash merge that changes no file gives `main` a new commit with the branch build's sources: `publish --reuse` releases that build's image, as it is, packing and uploading nothing. |
| artefact-069 | error | `error_069_reuse_fails_without_an_image_of_these_sources` | With no image of these sources there is nothing to reuse: `--reuse` fails, a dry run as well, so a release pipeline stops before it tags. |
| artefact-070 | normal | `normal_070_without_access_to_its_sources_a_checkout_is_its_artefact` | A repository whose sources cannot be read is its artefact, with nothing to configure: its release found in its registry, its dependencies in its image's manifest, and its source hash from the tree its image records. |
| artefact-071 | error | `error_071_without_access_a_branch_revision_needs_the_sources` | Without its sources, a branch cannot be resolved: only released versions. |
| artefact-072 | edge | `edge_072_an_unreachable_remote_is_not_taken_for_one_without_access` | A remote that cannot be reached is not one that refused: its registry is not asked instead, and the fetch error is what is said. |
| artefact-073 | error | `error_073_reuse_leaves_a_release_already_published_alone` | A release tag already naming an image stays where it is: `--reuse` refuses to move it, as a release is published once. |
| artefact-074 | error | `error_074_publish_fails_on_a_source_hash_it_cannot_take` | A dependency that does not resolve leaves the source hash unknown: a dry run says so and lists the layers, a publish fails, as `--reuse` could not find the image it would push. |
| artefact-075 | normal | `normal_075_without_access_a_raise_takes_the_newest_registry_version` | Without access to its sources, a raise takes the newest version its registry has: releases are cut in order on the producer's release branches, so the newest holds every one before it. |
| artefact-076 | perf | `perf_076_an_artefact_downloads_none_of_its_history` | A checkout taken as an artefact downloads none of its history: its refs are listed, its config read from its image, and nothing of git is kept for it — someone without access, or not wanting the sources, gets only the build. |
| artefact-077 | error | `error_077_a_release_must_be_a_version` | A release is named with a version; anything else is refused before anything is packed. |
| artefact-078 | error | `error_078_a_release_of_another_commit_is_refused` | A release already made of another commit is never taken over: refused before anything is packed or pushed. |
| artefact-080 | normal | `normal_080_a_build_pinned_by_its_source_hash_is_taken` | A build pinned by its source hash is taken as it is — here by a workspace that cannot read the sources, which no branch would reach: its files, its dependencies, and `build` as its revision's kind. |
| artefact-081 | error | `error_081_check_refuses_a_build_pin` | A build pin is for trying a build out, never for shipping: the merge gate refuses it, naming the config that holds it. |
| artefact-082 | error | `error_082_a_build_pin_needs_a_build` | A source hash that names no build fails, saying so; a malformed one too. |
| artefact-083 | normal | `normal_083_with_access_a_build_pin_is_its_commit` | With access to the sources, a build pin is a checkout of the commit the build was made from. |
| artefact-084 | normal | `normal_084_one_group_publishes_a_single_layer_image` | One group is one layer: the config travels in the manifest, so the image of a single group is a single layer — the form a deployer such as Argo CD takes as an OCI source. |
| artefact-085 | edge | `edge_085_no_group_ships_the_config_and_any_may_be_named_gitscale` | The repository's `.gitscale.toml` is never shipped in a layer, even by a group that matches it, and `gitscale` is a group name like any other. |

### cache

| ID | Kind | Test | What it holds |
|---|---|---|---|
| cache-001 | normal | `normal_001_ci_takes_a_pinned_commit_out_of_a_snapshot_entry` |  |
| cache-002 | normal | `normal_002_a_developer_sync_touches_no_cache` | Outside CI, `sync` never touches the cache. |
| cache-003 | normal | `normal_003_no_cache_in_ci_fetches_the_commit_from_the_remote` | `--no-cache` in CI talks to the remote: a depth-1 fetch of the one commit. |
| cache-004 | normal | `normal_004_a_second_job_re_pins_when_the_head_has_moved` |  |
| cache-005 | normal | `normal_005_update_warms_a_repo_nobody_has_placed` | `cache update` works anywhere, `CI` set or not: it adds the pin a job of this workspace would take. |
| cache-006 | normal | `normal_006_update_warms_images_without_a_checkout` | `cache update` works anywhere, CI or not: it adds what a job would take, so a runner image can be warmed ahead of time. |
| cache-007 | normal | `normal_007_update_covers_implicit_dependencies` | `cache update` warms the checkouts resolution settles on, the implicit ones included. |
| cache-008 | normal | `normal_008_compact_evicts_what_nothing_has_used` |  |
| cache-009 | normal | `normal_009_compact_drops_stale_pins_from_an_entry_it_keeps` |  |
| cache-010 | normal | `normal_010_compact_drops_cold_images_and_their_blobs` |  |
| cache-011 | normal | `normal_011_status_names_entries_after_the_repos_that_declare_them` |  |
| cache-012 | normal | `normal_012_status_lists_every_revision_a_snapshot_holds` |  |
| cache-013 | normal | `normal_013_status_has_an_images_column` |  |
| cache-014 | edge | `edge_014_ci_pins_an_annotated_tag_at_its_commit` | An annotated tag is an object of its own: what CI pins, and checks out, must be the commit it points at, not the tag object. |
| cache-015 | edge | `edge_015_ci_pins_the_branch_named_and_not_one_ending_in_the_name` | `git ls-remote <url> main` lists every ref whose name ends in `main`. The branch CI pins is the one called exactly that. |
| cache-016 | edge | `edge_016_a_job_is_unaffected_by_the_entry_being_deleted` |  |
| cache-017 | edge | `edge_017_a_sha_pinned_entry_needs_no_ref_advertisement` |  |
| cache-018 | edge | `edge_018_compact_works_from_outside_any_workspace` |  |
| cache-019 | error | `error_019_an_unknown_period_is_refused_before_anything_is_deleted` |  |
| cache-021 | normal | `normal_021_compact_evicts_a_cold_image_entry_whole` | An image entry nothing has used within the period goes whole, blobs and all, and `compact` counts it as an evicted entry. |
| cache-022 | edge | `edge_022_compact_keeps_the_images_resolution_just_read` | **ignored: bug: manifests resolution reads are recorded without a use marker, so compact drops them at once** What resolution read into the cache seconds ago — an artefact's manifest, read by a CI `fetch` for its config — is recently used: `compact` with a period of a month keeps it. |
| cache-023 | edge | `edge_023_compact_removes_half_written_downloads` | A download a killed job left half-written is garbage: `compact` removes it from an entry it keeps, and leaves every blob a held image needs. |
| cache-024 | error | `error_024_update_fails_for_an_unpublished_artefact_and_says_why` | `cache update` for an artefact whose release has no image fails, and says so, rather than warming something else. |
| cache-025 | error | `error_025_status_refuses_a_workspace_config_that_does_not_parse` | `cache status` does not need a workspace, but one whose config does not parse is an error, not a reason to guess at names. |
| cache-026 | error | `error_026_without_a_cache_location_the_commands_say_how_to_set_one` | With no `GITSCALE_CACHE_DIR`, `XDG_DATA_HOME` or `HOME` there is nowhere to keep a cache: the cache commands say which variable to set. |

### catalog

| ID | Kind | Test | What it holds |
|---|---|---|---|
| catalog-001 | normal | `normal_001_every_test_follows_the_naming_rules` | Every integration test is named `<kind>_<id>[<variant>]_<sentence>`, its id unique in its feature, and a variant letter only beside its siblings. |
| catalog-002 | normal | `normal_002_the_catalog_lists_every_test` | docs/test-catalog.md is what the sources say it is. |

### check

| ID | Kind | Test | What it holds |
|---|---|---|---|
| check-001 | normal | `normal_001_gates_only_merges_into_pinned_branches` | A merge request into a branch the root does not pin is not gated. |
| check-002 | normal | `normal_002_fails_a_merge_request_into_a_pinned_branch` | The gate's main case: a merge request into the branch the root pins, with a checkout taken from the topic, fails with exit 1 and says what would ship, what was tested and both ways out. |
| check-003 | normal | `normal_003_pinned_globs_decide_which_targets_are_gated` | `[branches] pinned` decides which merge targets are gated, globs included: a merge into `release/1.0` is checked, and with that list set, `main` — which it does not name — is not. |
| check-004 | normal | `normal_004_on_github_the_base_ref_and_the_event_default_decide` | On GitHub the target is `GITHUB_BASE_REF` and the default branch comes from the event payload: a pull request into the repository's default branch is gated even when that branch is not called `main`, and one into another branch is not. |
| check-005 | normal | `normal_005_names_the_config_of_the_dependency_that_pins_it` | The pin that would ship is named with the file it is written in: for a dependency's dependency, that repository's own `.gitscale.toml`. |
| check-006 | normal | `normal_006_lists_every_checkout_taken_from_the_topic` | Every checkout taken from the topic is named, not only the first: fixing one at a time against a gate that hides the rest is a pipeline per slot. |
| check-007 | normal | `normal_007_outside_ci_it_speaks_of_the_workspace` | Outside CI the gate speaks of the workspace and of running itself again, not of a pipeline that does not exist. |
| check-008 | normal | `normal_008_passes_when_every_checkout_is_at_a_pin` | No topic anywhere: the gate passes, and says so. |
| check-009 | edge | `edge_009_a_checkout_a_repository_holds_at_its_pin_does_not_block` | A checkout held at its pin because the repository above it pins the topic's branch is not from the topic, so it is not what blocks: only the repository that follows the branch is named. |
| check-010 | error | `error_010_in_ci_a_failed_fetch_fails_rather_than_answer_from_old_refs` | A gate that cannot fetch fails, though an earlier job fetched: runners keep the build directory between jobs, and refs fetched then would answer for a branch that has moved since. |
| check-011 | error | `error_011_a_repository_it_cannot_fetch_fails_the_check` | A repository the gate cannot fetch, and never has, is an error with exit 1: a gate that cannot resolve must not pass. |
| check-012 | normal | `normal_012_without_a_pipeline_default_origin_head_names_the_default_branch` | With no default branch from the pipeline — a GitHub event without a payload — the root's own `origin/HEAD` names it: a pull request into a default branch called `trunk` is gated. |
| check-013 | edge | `edge_013_an_artefact_from_the_topic_blocks` | An artefact taken from the topic — the image of its branch tip — blocks like a source checkout: a merge would ship the pinned image, not the one tested. |

### ci_auth

| ID | Kind | Test | What it holds |
|---|---|---|---|
| ci_auth-001 | normal | `normal_001_git_takes_the_token_from_the_environment` | The point of the helper: git gets the token by reading the environment at the moment it needs it, with nothing secret in the config or command line. |
| ci_auth-002 | normal | `normal_002_an_ssh_entry_on_the_ci_server_is_cloned_over_https` |  |
| ci_auth-003 | edge | `edge_003_another_host_is_never_offered_the_token` | The host check is the whole safety property: a dependency hosted elsewhere must never be offered the job token. |
| ci_auth-004 | edge | `edge_004_an_entry_on_another_host_keeps_its_ssh_url` |  |
| ci_auth-005 | normal | `normal_005_a_gitlab_job_fetches_its_servers_repositories_with_the_job_token` | End to end over real HTTP: in a GitLab job an entry declared over SSH on the CI server's own host is fetched from that server over HTTP(S), as `gitlab-ci-token` with the job token — and its `origin` is the plain URL. |
| ci_auth-006 | normal | `normal_006_a_github_actions_job_fetches_with_its_token` | The same on GitHub Actions: the runner marker plus `GITHUB_TOKEN` — or `GH_TOKEN` when that is the one exported — authenticate as `x-access-token` to `GITHUB_SERVER_URL`. |
| ci_auth-007 | edge | `edge_007_the_job_token_never_reaches_another_host` | The safety property over real HTTP: in a job on `localhost`, an entry on `127.0.0.1` — the same server, another host to git — is asked for credentials and never offered the job token, while the CI server's own entry in the same config is. |
| ci_auth-008 | edge | `edge_008_the_job_token_is_written_nowhere_on_disk` | The token is read from the environment when git asks, and lands nowhere on disk: not in a checkout's or the cache's git config, not in the breadcrumb a failed hook placement leaves, and not in the file of a `credential.helper = store` inherited from the user's git config — which git would hand every credential it approves, were the helper list not reset for the CI host. |
| ci_auth-009 | edge | `edge_009_the_ci_server_url_decides_where_entries_on_its_host_are_fetched` | Where the CI server is, and so which entries go to it: a port in `CI_SERVER_URL` is kept; without it the host, protocol and port variables are used; and a value that is not a plain http(s) URL turns the rewrite off rather than ending up in a git config key. |
| ci_auth-010 | edge | `edge_010_a_ci_server_under_a_path_keeps_the_path` | **ignored: bug: the path in CI_SERVER_URL is dropped, so a GitLab under a relative URL root gets 404s** GitLab can be served under a path (`https://host/gitlab`), and `CI_SERVER_URL` then carries it. Repositories live below that path, so the HTTPS URL an SSH entry is rewritten to must keep it. |
| ci_auth-011 | edge | `edge_011_github_tokens_without_the_runner_marker_change_nothing` | `GITHUB_TOKEN` and `GH_TOKEN` are often exported on developer machines; without the runner's own `GITHUB_ACTIONS` marker they change nothing, and an SSH entry stays SSH. |
| ci_auth-012 | edge | `edge_012_gitscale_no_ci_auth_turns_the_rewrite_off` | `GITSCALE_NO_CI_AUTH` turns the whole thing off: in a GitLab job an SSH entry on the CI server is fetched exactly as configured. |
| ci_auth-013 | edge | `edge_013_gitlab_wins_when_both_forges_are_detected` | A job that looks like both forges — a GitLab job token next to the GitHub runner marker — is a GitLab job, as documented: entries on the GitLab server are rewritten, entries on the GitHub one are not. |
| ci_auth-014 | edge | `edge_014_an_ssh_origin_left_in_a_checkout_is_repointed_before_fetching` | A checkout the runner kept from an earlier job may still have an SSH `origin` — restored from a cache, or set by hand. Before the next fetch it is repointed at the CI server's URL, and the pin moves. |
| ci_auth-015 | error | `error_015_a_403_from_the_ci_server_says_how_to_get_access` | A 403 from the CI server means the token was accepted but may not read that project. The failure says what to change, for the forge in question: GitLab's job token allowlist, or a better token on GitHub. |
| ci_auth-016 | error | `error_016_a_403_in_a_repository_name_is_not_a_refused_token` | **ignored: bug: any \"403\" in git's stderr, even in a path, is taken for a refused job token** Only a 403 from the server is a refused token. A failure that merely has `403` somewhere in its text — here in a repository's name — gets git's own message, not advice about job token permissions. |

### ci_checkout

| ID | Kind | Test | What it holds |
|---|---|---|---|
| ci_checkout-001 | normal | `normal_001_a_kept_checkout_follows_every_pin_change_without_the_cache` | Without a cache a shallow checkout's own refspec is the one branch it was cloned at, so updating it by `fetch` + `reset @{upstream}` never reached a tag, or another branch, the config had moved to — and reported `ok`. |
| ci_checkout-002 | normal | `normal_002_a_ci_sync_leaves_each_checkout_as_a_fresh_clone_would` | In place is what keeps the build cache valid, and what lets the last job's leftovers through: in CI the sync finishes the job a fresh clone would do. |
| ci_checkout-003 | normal | `normal_003_a_ci_sync_cleans_each_checkout_as_clean_fdx_does` | The CI sync ends with exactly `git scale clean -fdx <each checkout>`: the same rules — a checkout's own keep-list included — and the same report. |
| ci_checkout-004 | normal | `normal_004_an_exclude_that_keeps_the_checkouts_passes` |  |
| ci_checkout-005 | edge | `edge_005_outside_ci_a_sync_leaves_untracked_files_alone` |  |
| ci_checkout-006 | edge | `edge_006_the_runner_default_clean_flags_are_checked_too` |  |
| ci_checkout-007 | edge | `edge_007_outside_gitlab_the_clean_flags_are_not_consulted` |  |
| ci_checkout-008 | error | `error_008_a_pin_the_remote_does_not_have_fails_instead_of_staying_put` |  |
| ci_checkout-009 | error | `error_009_a_gitlab_hook_placement_fails_when_the_runner_would_delete_the_checkouts` |  |
| ci_checkout-010 | error | `error_010_an_exclude_covering_only_some_checkouts_names_the_rest` |  |
| ci_checkout-011 | edge | `edge_011_a_ci_sync_moves_a_checkout_whose_tracked_files_were_changed` | A CI checkout holds nobody's work: a tracked file the last job changed — a generated file, a version stamp — does not stop the next pin from being checked out over it, as it would on a developer's machine. |
| ci_checkout-012 | error | `error_012_a_gitlab_hook_reports_why_the_clean_check_could_not_run` | **ignored: bug: any error from the runner-clean check is reported as GIT_CLEAN_FLAGS deleting the checkouts** When the clean check cannot run at all — the config does not load, or git rejects the job's `GIT_CLEAN_FLAGS` — the failure says that, rather than claiming the flags would delete the checkouts. |

### clean

| ID | Kind | Test | What it holds |
|---|---|---|---|
| clean-001 | normal | `normal_001_force_removes_untracked_files_and_keeps_checkouts` |  |
| clean-002 | normal | `normal_002_a_dry_run_lists_and_removes_nothing` |  |
| clean-003 | normal | `normal_003_a_subrepo_takes_excludes_from_its_own_config` |  |
| clean-004 | normal | `normal_004_keeps_managed_symlinks` |  |
| clean-005 | normal | `normal_005_a_command_line_exclude_applies_to_every_repo` |  |
| clean-006 | normal | `normal_006_names_select_repos_and_dot_the_root` |  |
| clean-007 | normal | `normal_007_skips_artefact_checkouts` |  |
| clean-008 | normal | `normal_008_gc_drops_cold_images_and_their_blobs` | `git scale gc` drops the images nothing has used within the period, and the layers no remaining image needs. |
| clean-009 | edge | `edge_009_an_uncommitted_workspace_config_survives_its_own_clean` |  |
| clean-010 | edge | `edge_010_root_clean_excludes_do_not_reach_subrepos` |  |
| clean-011 | edge | `edge_011_recursive_false_is_cleaned_without_its_own_keep_list` | `recursive = false` puts the repo's own config out of reach, keep-list included, so it is cleaned by the command line's patterns alone — and what lives inside it that gitscale did not put there is left to git, which reports a nested clone rather than deleting it. |
| clean-012 | edge | `edge_012_works_in_a_readonly_checkout` |  |
| clean-013 | edge | `edge_013_keeps_a_checkout_nested_inside_another` |  |
| clean-014 | edge | `edge_014_removes_a_directory_holding_no_repository_so_sync_works` |  |
| clean-015 | edge | `edge_015_leaves_a_directory_whose_git_is_broken` |  |
| clean-016 | edge | `edge_016_leaves_a_stray_directory_holding_another_checkout` |  |
| clean-017 | edge | `edge_017_keeps_implicit_checkouts_and_their_links` | The root's clean keeps the implicit checkouts, which are untracked directories to it like any declared one, and a dependency's clean keeps the links planted in it. |
| clean-018 | error | `error_018_an_option_like_pattern_is_refused` |  |
| clean-019 | normal | `normal_019_a_dry_run_names_why_it_skips_what_it_skips` | The dry run says why it leaves a repository alone, so a skipped one is never mistaken for a clean one: an artefact, a checkout not cloned yet, and an entry that is a symlink each name their reason. |
| clean-020 | normal | `normal_020_an_anchored_exclude_keeps_only_the_match_at_each_repo_root` | A leading `/` anchors a pattern at the root of each repository cleaned — the workspace's and every checkout's — and nowhere deeper. |
| clean-021 | normal | `normal_021_gc_takes_its_period_from_the_config_unless_the_flag_overrides_it` | `gc` without `--keep-recent` keeps what `[clean] keep_recent` says, not the built-in three months, and the flag overrides the config. The summary counts the stores it collected. |
| clean-023 | edge | `edge_023_a_checkout_named_with_glob_characters_survives_the_root_clean` | **ignored: bug: checkout directories reach git clean -e unescaped, so a name with [ ] is a glob and the checkout is deleted** A declared checkout is kept by its directory's name, whatever characters that name holds: an artefact checkout, which has no `.git` for git to recognise, named `meta/app[1]` must survive the workspace's clean. |
| clean-024 | edge | `edge_024_a_stray_directory_is_removed_with_a_clone_made_inside_it` | An entry's directory holding no repository is removed whole, as the docs say — and that includes a repository somebody cloned by hand inside it, commits and all. This pins the current behaviour: it contradicts the rule that a nested repository gitscale does not manage is reported rather than deleted, and is listed as a decision for the owner. |
| clean-025 | edge | `edge_025_a_symlinked_entry_is_skipped_and_its_target_left_alone` | An entry that is a symlink to somebody's own checkout is skipped, and nothing is removed from what it points at. |
| clean-026 | edge | `edge_026_an_untracked_symlink_is_removed_without_following_it` | An untracked symlink to something outside the workspace is removed as the link it is: what it points at stays. |
| clean-027 | edge | `edge_027_a_dry_run_lists_exactly_what_force_removes` | What the dry run lists is what `-f` then removes — names with spaces, non-ASCII letters and a leading dash included — and the counts agree. |
| clean-028 | edge | `edge_028_a_checkouts_uncommitted_config_survives` | A checkout's own `.gitscale.toml` is kept even before it is committed, like the workspace's. |
| clean-029 | edge | `edge_029_excludes_do_not_reach_into_a_directory_holding_no_repository` | Exclusions describe untracked files inside a repository; a directory holding none is removed whole whatever they say. |
| clean-030 | edge | `edge_030_a_dangling_git_link_is_not_taken_for_a_stray_directory` | **ignored: bug: a dangling .git symlink is not seen as a checkout, so its directory is removed as stray** A `.git` that is a symlink to something gone still says a checkout was here, as a damaged `.git` directory does: what is beside it may be the only copy of somebody's work, and is left. |
| clean-031 | edge | `edge_031_gc_with_no_stores_reports_nothing_collected` | `gc` in a root that has fetched nothing yet compacts nothing, and says so rather than failing. |
| clean-032 | error | `error_032_an_empty_or_option_like_command_line_pattern_is_refused` | A command-line pattern that is empty, or that git would read as an option, is refused before anything is removed — with `-f` too. |
| clean-033 | error | `error_033_an_unreadable_checkout_config_stops_the_clean_before_anything_goes` | A checkout's `.gitscale.toml` that cannot be read stops the clean before anything goes, in that repository or any other: an unreadable keep-list is not an empty one. Broken TOML is caught when resolution reads the file; a keep-list git would read as an option, when clean reads its rules. |
| clean-034 | error | `error_034_a_graph_that_cannot_be_resolved_is_refused` | A graph that cannot be resolved leaves the set of links and implicit checkouts unknown, so clean refuses rather than guess. |
| clean-035 | error | `error_035_links_of_a_checkout_resolution_cannot_settle_are_kept` | **ignored: bug: offline, a checkout resolution leaves unresolved has its planted links deleted by clean -fdx** A checkout resolution cannot settle offline still has its links and its own checkout kept: clean must not delete what it cannot account for. |
| clean-036 | error | `error_036_an_unknown_name_fails_and_removes_nothing` | A name that is not a declared checkout fails the clean, and nothing is removed anywhere. |
| clean-037 | error | `error_037_a_repo_that_cannot_be_cleaned_fails_and_the_rest_are_cleaned` | A repository git cannot clean is a failure, exit 1, named on stderr; the others are still cleaned. |
| clean-038 | error | `error_038_gc_is_refused_in_ci` | `gc` compacts the root's own stores, and CI keeps none: it is refused there, pointing at the command that compacts the CI cache. |
| clean-039 | error | `error_039_gc_refuses_a_period_it_cannot_read_before_dropping_anything` | A period `gc` cannot read — a bare `m`, which is minutes to humantime and months to a person, or no period at all — is refused before any image is dropped, on the command line and in `[clean] keep_recent` alike. |
| clean-040 | error | `error_040_gc_takes_no_force_exclude_or_names` | `gc` cleans no working tree, so it takes no `-f`, no `-e` and no names. |
| clean-041 | error | `error_041_keep_recent_is_refused_by_clean` | `--keep-recent` is `gc`'s. Given to `clean`, the command is refused rather than run as a plain clean that ignores the period — with `-f`, a clean that deletes files the user meant to keep a compaction for. |
| clean-042 | normal | `normal_042_d_and_x_mean_what_they_mean_to_git` | `-f` alone removes untracked files; `-d` adds directories; `-x` adds ignored files — in the root and in each checkout, as git's do. |
| clean-043 | normal | `normal_043_capital_x_removes_only_ignored_files_and_never_a_checkout` | `-X` removes only ignored files — and, as `-e` patterns are more ignored files to it, never by way of what clean keeps: a checkout under an ignored directory stays. |
| clean-044 | normal | `normal_044_n_lists_and_wins_over_f` | With neither `-n` nor `-f` a clean only lists, and `-n` wins over `-f`. |
| clean-045 | normal | `normal_045_q_reports_only_failures` | `-q` reports only failures. |

### cli

| ID | Kind | Test | What it holds |
|---|---|---|---|
| cli-001 | normal | `normal_001_help_and_version_succeed_on_stdout` | `--help` and `--version` are answers, not failures: they print to stdout and exit 0, at the top level and for a subcommand's help. |
| cli-002 | normal | `normal_002_global_flags_go_before_or_after_the_subcommand` | `-v`, `--no-cache`, `-C` and `--color` are global: accepted before the subcommand or after it, as docs/cli.md says. |
| cli-003 | normal | `normal_003_root_option_before_the_subcommand_is_accepted` | `-C` before the subcommand is what docs/cli.md shows, and where a git command needs it: everything after the git command is git's. |
| cli-004 | normal | `normal_004_the_config_is_found_upward_from_the_working_directory` | Without `-C` the config is found from the working directory, searching upward: a command run from a directory inside the workspace acts on the whole workspace. |
| cli-005 | normal | `normal_005_a_terminal_gets_progress_lines_and_the_same_exit_status` | On a terminal the per-repository lines are live progress lines, and a failure still fails the run: the transcript shows each repository's outcome and the summary, and the exit status is 1. |
| cli-006 | edge | `edge_006_a_terminal_places_nested_entries_like_a_plain_run` | **ignored: bug: on a terminal nested entries are placed in parallel; when deps/inner lands first, deps fails with git worktree add: already exists (intermittent)** A terminal run is parallel. Two entries where one sits inside the other (`deps` and `deps/inner`, which config allows) must both be placed there as they are on the plain, sequential path: the inner one's directory must not get in the way of the outer one. A race: the test tries many times. |
| cli-007 | error | `error_007_an_unknown_command_goes_to_git` | Any command GitScale does not know is a git command: one git does not know either fails there, with git's own words and the summary line. |
| cli-008 | error | `error_008_an_unknown_option_of_a_gitscale_command_is_a_usage_error` | A GitScale command's own options are checked: one it does not take is a usage error, and nothing runs. |
| cli-009 | error | `error_009_a_nonexistent_root_path_is_named` | A `-C` path that does not exist is named in the error, so the user can see which of their arguments was wrong. |
| cli-010 | error | `error_010_outside_a_workspace_says_so` | Outside any workspace there is nothing to act on: the command fails, says so, and creates nothing — for GitScale's commands and git's alike. |

### config

| ID | Kind | Test | What it holds |
|---|---|---|---|
| config-001 | error | `error_001_a_config_outside_a_git_repository_is_refused` | The workspace is the top of a git repository; anything else is refused. |
| config-002 | normal | `normal_002_a_root_option_inside_the_workspace_finds_its_config` | `-C` starts the upward search, so pointing it at a directory inside the workspace finds the workspace's config and acts on every entry, placed relative to the config rather than to that directory. |
| config-003 | edge | `edge_003_a_misspelt_entry_key_is_ignored_as_documented` | Unknown keys in an entry are ignored on read, as docs/configuration.md says — so a misspelt `revision` leaves the entry with none, and it follows the default branch. Pinned as documented; it is a trap worth knowing. |
| config-004 | error | `error_004_invalid_toml_is_refused_naming_the_file` | A config that is not TOML is refused by every command, naming the file and the parser's reason, and nothing is cloned. |
| config-005 | error | `error_005_the_workspace_itself_or_its_git_directory_is_refused` | **ignored: bug: check_directory only refuses '..' and absolute paths, so '.', './' and '.git/hooks' are accepted** A checkout directory must be a directory of its own inside the workspace: not the workspace itself (`.`), and nothing inside its git directory, where a checkout's files would be git's own — hooks included. |
| config-006 | error | `error_006_two_entries_naming_one_directory_are_refused` | **ignored: bug: entries are told apart by their spelling, so 'libs/x' and 'libs/x/' (or './libs/x') both load** Two entries that name one directory, spelt differently, would fight over one checkout on every placement. They are refused when the config is read. |
| config-007 | error | `error_007_a_refused_value_says_why` | **ignored: bug: these errors are wrapped in a context naming only the key, and the CLI prints only the outermost message** A refused value says what is wrong with it, not only where it is: the CLI prints the outermost message alone, so the reason must be in it. |

### exclude

| ID | Kind | Test | What it holds |
|---|---|---|---|
| exclude-001 | normal | `normal_001_checkouts_are_not_untracked_files_of_the_root` | With no `.gitignore` entry for them, the checkouts are not untracked files of the root: `git status` is clean, and `git scale add -A` stages none of them into the root. |
| exclude-002 | normal | `normal_002_each_worktree_has_its_own_block_and_other_lines_stay` | Lines of one's own in `info/exclude` stay; every root worktree has a block of its own; the block of a worktree that is gone goes. |
| exclude-003 | edge | `edge_003_nested_checkouts_and_odd_names_are_excluded_literally` | A checkout's own block lists the checkouts inside it, and a name with pattern characters in it is taken literally. |

### forward

| ID | Kind | Test | What it holds |
|---|---|---|---|
| forward-001 | normal | `normal_001_runs_on_the_topic_dependencies_first_and_the_root_last` | By default a git command runs in the root and every checkout on the topic: dependencies before the repositories that ask for them, the root last. |
| forward-002 | normal | `normal_002_off_a_topic_runs_in_the_root_alone` | Off a topic, the root alone: every checkout sits at its pin. |
| forward-003 | normal | `normal_003_for_names_the_repositories_and_foreach_adds_the_pinned_ones` | `--for` names exactly the repositories to run in — `.` is the root — and `--foreach` adds the checkouts at their pins. |
| forward-004 | normal | `normal_004_for_takes_paths_from_the_current_directory` | `--for` takes a path from the current directory, and a link path names the checkout it points at. |
| forward-005 | normal | `normal_005_commit_skips_a_repository_with_nothing_to_commit_but_not_amend` | `commit` skips a repository with nothing to commit, saying so in its header, which still counts; `--amend` is never skipped. |
| forward-006 | normal | `normal_006_a_first_push_sets_the_upstream` | A first `push` of a topic branch sets its upstream: git is given `push.autoSetupRemote`. |
| forward-007 | normal | `normal_007_changes_outside_the_selection_are_named` | A checkout with changes that the command did not run in — not on the topic — gets one line on stderr saying how to bring it in. Links planted in a checkout are not changes. |
| forward-008 | normal | `normal_008_pull_places_detached_checkouts` | `pull` brings every checkout up to date, detached ones included: a checkout pinned to a branch that moved, and one whose remote has the topic a colleague started there. |
| forward-009 | normal | `normal_009_fetch_refreshes_every_store_and_moves_nothing` | `fetch` refreshes every store, including one no checkout uses any more, and moves nothing. |
| forward-010 | normal | `normal_010_another_command_places_only_when_a_head_moved_fetching_on_miss` | Any other command places the workspace only when it moved a `HEAD`, and then fetches only what resolution lacks: here, the tag a branch of the root's newly asks for. |
| forward-011 | normal | `normal_011_an_alias_of_pull_counts_as_pull_and_a_shell_alias_does_not` | An alias of `pull` is `pull`: placed online afterwards. A shell alias is a command of its own name. |
| forward-012 | normal | `normal_012_force_sync_relinks_and_force_goes_to_git` | `--force-sync` makes the placement a forced one; `--force` is git's. |
| forward-013 | normal | `normal_013_forward_parallel_is_the_default_for_pull_alone` | `[forward] parallel` makes `pull` parallel, leaves `log` sequential, and `--parallel=1` runs sequentially. A parallel run reports each failure on a line of its own; a sequential one leaves that to git's output. |
| forward-014 | normal | `normal_014_parallel_output_is_kept_per_repository_in_order` | In parallel, each repository's output is printed whole after its own header, in running order. |
| forward-015 | normal | `normal_015_a_failing_repository_fails_the_command_after_the_rest` | The exit status is 1 when a repository failed, with a last line naming each, and the rest still ran. |
| forward-016 | normal | `normal_016_headers_count_pad_and_end_in_sixteen_dashes` | Headers: the count includes skipped repositories, a count from 10 is padded, a header off a terminal ends in 16 dashes, and `placing` comes only when something is placed. |
| forward-017 | normal | `normal_017_inside_a_child_the_workspace_is_addressed` | Typed inside a child, a git command runs across the workspace the child belongs to. |
| forward-018 | normal | `normal_018_an_editor_gets_the_terminal` | An editor gets the terminal, run in sequence: stdin and stdout are the user's. |
| forward-019 | normal | `normal_019_log_goes_through_one_pager_and_commit_never_does` | `log` on a terminal goes through one pager, headers included; `commit` never does. |
| forward-020 | normal | `normal_020_a_parallel_editor_fails_and_names_the_flag` | Under `--parallel` nothing has a terminal: a command that needs an editor fails, and says to run without the flag. |
| forward-021 | normal | `normal_021_a_parallel_credential_prompt_fails_instead_of_hanging` | Under `--parallel`, a remote that asks for credentials fails the fetch at once rather than wait for an answer nobody can give. |
| forward-022 | error | `error_022_for_refuses_a_checkout_off_the_topic` | Naming a checkout that is not on the topic fails before anything runs, with the command that brings it in. |
| forward-023 | error | `error_023_force_sync_with_fetch_is_refused` | `--force-sync` asks for a forced placement, which `fetch` does not make. |
| forward-024 | error | `error_024_forwarding_options_with_a_gitscale_command_are_refused` | Forwarding options are for git commands: before a GitScale command they are refused rather than ignored. |
| forward-025 | edge | `edge_025_add_all_never_stages_the_links_gitscale_planted` | The links gitscale plants in a checkout are not its owner's work: staging everything on the topic stages the real changes, never the links. |
| forward-026 | perf | `perf_026_a_moved_head_places_without_fetching_what_is_not_missing` | A command that moved a `HEAD` but left nothing for resolution to lack places the workspace without fetching a single store; `pull` fetches each one. |
| forward-027 | normal | `normal_027_gitscale_options_after_the_git_command_are_gitscales` | GitScale's options may follow the git command too, up to `--`; what is left is git's. |

### hash

| ID | Kind | Test | What it holds |
|---|---|---|---|
| hash-001 | normal | `normal_001_a_checkout_hashes_as_its_own_pipeline_would` | A repository's hash in another workspace is the one its own pipeline takes, though that workspace raised one of its dependencies: B is hashed with D at v1.0.0, which it asks for, not at the v1.1.0 checked out. |
| hash-002 | normal | `normal_002_it_follows_trees_not_commits` | The hash is of trees: a commit that changes no file keeps it, a dependency at other content changes it. |
| hash-003 | normal | `normal_003_a_checkout_on_the_topic_hashes_with_the_topics_branches` | A checkout at its pin hashes with its dependencies at their pins; joined to the topic, with the topic's branches — as its pipeline on the topic branch would build it. |
| hash-004 | edge | `edge_004_recursive_false_keeps_an_entrys_dependencies_out` | An entry with `recursive = false` reaches only that repository, not its dependencies. |
| hash-005 | edge | `edge_005_ls_json_carries_each_checkouts_source_hash` | `ls --format json` carries each checkout's source hash. |
| hash-006 | error | `error_006_uncommitted_changes_fail_unless_committed` | Uncommitted changes in a source fail the hash, which would not be of anything built; `--committed` hashes the commit, changes left out. |
| hash-007 | normal | `normal_007_a_checkout_hashes_the_same_as_source_and_as_artefact` | How the workspace takes a checkout changes nothing of what it was built from: the same hash as its sources and as its artefact. |
| hash-008 | normal | `normal_008_a_ci_job_hashes_a_checkout_whose_dependency_was_raised` | A CI job keeps no stores of its own: the tree of a dependency at the version a checkout's own pipeline takes — below the one its workspace raised it to — comes from that commit, fetched depth 1 with no files, not from an image the dependency never published. With the cache on or off, the job hashes B as a developer machine does. |

### hook

| ID | Kind | Test | What it holds |
|---|---|---|---|
| hook-001 | normal | `normal_001_install_local_writes_both_hooks` |  |
| hook-002 | normal | `normal_002_install_displaces_and_chains_an_existing_hook` |  |
| hook-003 | normal | `normal_003_uninstall_restores_the_displaced_hook` |  |
| hook-004 | normal | `normal_004_shim_runs_the_chained_hook_and_honours_the_recursion_guard` |  |
| hook-005 | normal | `normal_005_status_reports_installed_hooks` |  |
| hook-006 | normal | `normal_006_install_bakes_the_requested_allowlist_into_the_shim` |  |
| hook-007 | normal | `normal_007_install_local_allows_its_own_repository_by_default` | A `--local` install is already a decision about one repository, and the file it writes cannot travel to anyone else — so it needs no patterns. |
| hook-008 | normal | `normal_008_status_reports_the_allowlist` |  |
| hook-009 | edge | `edge_009_install_local_from_a_linked_worktree_writes_the_shared_hooks` | git runs every worktree's hooks from the common `.git/hooks`; a linked worktree's own git dir (`.git/worktrees/<name>`) is never consulted. |
| hook-010 | edge | `edge_010_shim_is_silent_in_a_repo_without_a_gitscale_config` |  |
| hook-011 | edge | `edge_011_shim_chains_a_hook_whose_path_holds_a_quote` | The shim names the chained hook by path, and a path may hold a quote. |
| hook-012 | edge | `edge_012_shim_ignores_a_file_checkout` | `git checkout -- <path>` fires post-checkout too, with a third argument of 0. It moves no revision, and a build restoring a file must not have its sub-repositories reset and cleaned underneath it. |
| hook-013 | edge | `edge_013_reinstall_keeps_the_existing_allowlist` | Upgrading gitscale means re-running install; that must not silently widen or narrow what the machine already allows. |
| hook-014 | edge | `edge_014_status_recognises_a_repositorys_own_copy_of_the_shim` | A monorepo that commits its own copies of the shim — `.githooks/`, pointed at by a repo-local `core.hooksPath` — is running gitscale, and status must say so rather than call them foreign and advise installing over them. Uninstall, which acts only on shims gitscale wrote, leaves them alone. |
| hook-015 | edge | `edge_015_status_still_flags_a_repo_path_that_runs_no_gitscale` | Without copies of its own in the repo's hooks directory, the advice stands. |
| hook-016 | edge | `edge_016_a_global_hook_refuses_a_payload_from_a_cloned_branch` | The reported attack, end to end: a global hook is installed, an attacker's branch carries a `.gitscale.toml` with a payload, and a developer clones it to review. Nothing but the allowlist stands in the way. |
| hook-017 | error | `error_017_run_rejects_an_unknown_hook_name` |  |
| hook-018 | error | `error_018_install_global_requires_an_allowlist` | A global install arms every clone on the machine, so it has to be told what it may run. Guessing a default here is the bug this exists to prevent. |
| hook-019 | error | `error_019_install_refuses_a_pattern_that_would_break_the_shim` |  |
| hook-020 | error | `error_020_run_refuses_when_the_shim_passed_no_allowlist` | A shim written before the allowlist existed passes no patterns. Running wide open in that case would leave the hole in place across an upgrade. |
| hook-021 | normal | `normal_021_global_install_keeps_each_repositorys_own_git_hooks_running` | **ignored: bug: a --global install never runs a repository's own .git/hooks** A global `core.hooksPath` replaces every repository's `.git/hooks`, so the docs promise that the installed hook still runs the repository's own hook first. A repository's own `post-checkout` — and its `pre-commit`, which gitscale never installs — must keep running once a global install is in place, or a secret scanner or git-lfs hook silently stops working. |
| hook-022 | normal | `normal_022_on_pull_error_decides_whether_a_failed_hook_placement_fails_the_checkout` | `on_pull_error` decides whether a failed hook-triggered placement fails the git operation: unset, CI fails fast and a developer machine only warns; set, it means what it says either way. Every failure leaves a breadcrumb. |
| hook-023 | normal | `normal_023_status_reports_a_failed_hook_placement_until_one_succeeds` | The breadcrumb is what lets a later, more confusing failure be traced back: `hook status` shows the failed placement until a hook-triggered one succeeds, which removes it. |
| hook-024 | normal | `normal_024_uninstall_global_removes_the_shims_and_unsets_core_hooks_path` | Uninstalling a global install takes back exactly what it set up: the shims in ~/.config/gitscale/hooks and the global `core.hooksPath` pointing at them — and nothing else in the user's git config. |
| hook-025 | normal | `normal_025_shim_passes_its_arguments_to_the_chained_hook_and_returns_its_status` | The shim hands git's arguments to the repository's own hook, and that hook's exit status is the shim's: a hook that rejects an operation still rejects it once gitscale is in front of it. |
| hook-026 | normal | `normal_026_git_merge_places_the_workspace_through_post_merge` | `git merge` and `git pull` fire `post-merge`, not `post-checkout`: a merge that brings in a new declared checkout gets it placed. |
| hook-027 | normal | `normal_027_a_global_hook_populates_a_fresh_clone_and_a_new_worktree` | The way a workspace is meant to be set up: with a global hook, `git clone` and nothing else populates every declared checkout, and so does `git worktree add` for a second worktree. |
| hook-028 | edge | `edge_028_reinstall_keeps_chaining_to_the_displaced_hook` | Re-installing — what an upgrade of gitscale means — keeps the shim chained to the hook it displaced the first time. Losing the link would leave the repository's own hook in `<name>.local`, never run again. |
| hook-029 | edge | `edge_029_uninstall_with_nothing_installed_says_so_and_changes_nothing` | Uninstalling where gitscale installed nothing says so and touches nothing: not somebody else's hook, and not a `core.hooksPath` it did not set. |
| hook-030 | edge | `edge_030_uninstall_removes_the_shim_when_the_displaced_hook_is_gone` | The displaced hook may have been deleted by hand since the install. The uninstall still removes gitscale's own shim, and says it removed it rather than claiming a restore. |
| hook-031 | edge | `edge_031_shim_reports_a_missing_binary_only_where_the_repo_opted_in` | A shim whose gitscale binary has gone says so — but only in a repository that opted in with a `.gitscale.toml`; every other repository stays silent. Either way the git operation is not failed for it. |
| hook-032 | edge | `edge_032_shim_skips_a_chained_hook_that_is_not_executable` | git itself skips a hook file that is not executable; the shim does the same with a displaced one, rather than failing the checkout on it. |
| hook-033 | edge | `edge_033_status_shows_a_global_hooks_path_that_hides_a_local_install` | A `--local` install writes `.git/hooks`, which git ignores once a global `core.hooksPath` is set. The install does not say so (see the decisions in the review); `hook status` is what shows it: the hooks directory git really uses, and no gitscale hook in it. Pins current behaviour. |
| hook-034 | edge | `edge_034_status_reports_the_system_and_global_hooks_paths` | `hook status` reads the system and global settings from their own files — what a test can point `GIT_CONFIG_SYSTEM` at, since `git config --system` ignores `GIT_CONFIG_NOSYSTEM`. Read-only: nothing here installs at `--system` scope. |
| hook-035 | edge | `edge_035_status_outside_a_repository_says_so` | Outside any repository `hook status` still reports the machine-wide settings and says there is no repository, rather than failing. |
| hook-036 | edge | `edge_036_run_in_a_repo_without_a_config_does_nothing` | `hook run` re-checks the opt-in the shim makes: a repository with no `.gitscale.toml` at its root gets no placement and no output. |
| hook-037 | edge | `edge_037_allow_patterns_reach_gitscale_verbatim_and_never_run` | The patterns are written into a shell script. Shell syntax in one — `$(…)`, backticks, `;` — must reach gitscale as the literal pattern and never run. |
| hook-038 | edge | `edge_038_a_refused_post_sync_leaves_a_breadcrumb_and_follows_the_policy` | A `post_sync` the allowlist refuses is a failed hook placement like any other: it leaves a breadcrumb, and `on_pull_error` decides — off CI the checkout goes on, in CI it fails. The command never runs either way. |
| hook-039 | edge | `edge_039_install_beside_a_repositorys_own_copy_runs_gitscale_once` | **ignored: bug: install displaces a repository's own gitscale copy and chains to it, so every checkout is placed twice** A monorepo may commit its own copy of the shim and point `core.hooksPath` at it; `hook status` already recognises such a copy. A `--local` install there must not end up running gitscale twice on every checkout — once from the displaced copy and again from the new shim. |
| hook-040a | error | `error_040a_install_global_refuses_a_foreign_core_hooks_path` | A global `core.hooksPath` gitscale did not set belongs to somebody — a team's shared hooks, say. Installing over it would silently disable them, so the install refuses, and leaves the config and the hooks as they were. |
| hook-040b | error | `error_040b_install_global_with_force_replaces_a_foreign_core_hooks_path` | With `--force` the install replaces the foreign `core.hooksPath`, leaving the directory it named alone. A later uninstall unsets `core.hooksPath` rather than restoring the old value — current behaviour, pinned here and listed for the owner to decide. |
| hook-041a | error | `error_041a_install_refuses_to_overwrite_a_displaced_hook` | A hook already displaced to `<name>.local` is somebody's: a second foreign hook would overwrite it. The install refuses, and writes nothing. |
| hook-041b | error | `error_041b_install_with_force_displaces_over_an_older_displaced_hook` | With `--force` the hook in place is displaced over the older `.local`, which is lost — what `--force` is documented to do. |
| hook-042 | error | `error_042_a_refused_install_leaves_every_hook_untouched` | **ignored: bug: a .local collision on post-merge is found after post-checkout was already displaced and rewritten** A refused install leaves every hook as it found it — including the ones it would have handled before reaching the one that made it refuse. |
| hook-043 | error | `error_043_status_never_prints_credentials_from_the_origin` | An `origin` can carry credentials — older GitLab runners check out with the job token in the URL. `hook status` names the repository by host and path and never prints them. |
| hook-044 | perf | `perf_044_a_hooked_clone_runs_gitscale_exactly_once` | One clone runs gitscale once. The placement it starts checks out every declared repository with git, which fires the same global hook in each — and each carries a `.gitscale.toml` here, so only the recursion guard stops that from recursing. |
| hook-045 | normal | `normal_045_a_git_pull_in_a_child_places_its_siblings_fetching_on_miss` | A pull in a child that changes what it asks for places its siblings — fetching the tag their store lacks — and leaves the child where the pull put it. |
| hook-046 | normal | `normal_046_a_child_switched_to_its_topic_is_writable_and_elsewhere_warned` | Switched with plain git to its slot's topic branch, a child is made writable, as `git topic join` makes it; switched anywhere else it stays, with a warning that the next placement moves it back. |
| hook-047 | edge | `edge_047_a_child_without_a_config_or_not_recursive_places_the_workspace` | A child with no config of its own, and one the root declares with `recursive = false`, place the workspace like any other. |
| hook-048 | edge | `edge_048_nothing_happens_mid_rebase_or_at_a_bisect_step` | In the middle of a rebase, a merge or a bisect, the child is not where it will end: the hook does nothing. |
| hook-049 | normal | `normal_049_the_allowlist_is_matched_on_the_roots_remote` | The allowlist is matched against the workspace root's remote — whose `post_sync` runs — not the child's. |
| hook-050 | normal | `normal_050_the_installed_shim_fires_in_a_child` | End to end: the installed shim recognises a child by its common dir and hands it to `hook run --child`, so `git switch` to the topic in a child makes it writable. |

### links

| ID | Kind | Test | What it holds |
|---|---|---|---|
| links-001 | normal | `normal_001_a_dependency_of_a_dependency_links_to_the_root_checkout` |  |
| links-002 | normal | `normal_002_sync_relinks_a_clean_clone` |  |
| links-003a | edge | `edge_003a_sync_skips_a_modified_clone_without_force` |  |
| links-003b | normal | `normal_003b_sync_force_relinks_a_modified_clone` |  |
| links-004a | edge | `edge_004a_sync_skips_a_clone_with_unpushed_commits` |  |
| links-004b | normal | `normal_004b_sync_force_relinks_a_clone_with_unpushed_commits` |  |
| links-005 | edge | `edge_005_sync_skips_a_clone_with_commits_on_a_local_only_branch` | A commit on a branch the remote has never seen is as much local work as an unpushed commit on a tracked one. With no upstream there is nothing to be ahead of, so it has to be found some other way — or `sync` deletes it. |
| links-006 | edge | `edge_006_sync_skips_a_clone_with_a_stash` | A stash is work set aside, not thrown away: it lives in the clone's own refs, so replacing the clone would delete it. |
| links-007 | normal | `normal_007_sync_relinks_a_clone_holding_only_ignored_files` | Ignored files are what a build leaves behind, not work: they do not hold a relink back. |
| links-008 | edge | `edge_008_sync_with_names_leaves_other_repos_links_alone` | Names narrow a sync to the repos named. A modified clone inside repoA is repoA's business: syncing an unrelated repo, even with `--force`, must not replace it. |
| links-009 | normal | `normal_009_ls_shows_an_unlinked_clone` |  |
| links-010 | normal | `normal_010_ls_shows_an_unlinked_clone_as_modified` |  |
| links-011 | edge | `edge_011_ls_shows_unlinked_under_a_nested_entry` | The repo that owns a replaced link is the declared entry it sits under, however many path components that entry spans. |
| links-012 | edge | `edge_012_planted_links_are_kept_out_of_gits_sight` | The links GitScale plants in a checkout are not its owner's work: they are kept out of git's sight, in the checkout's `info/exclude`, so they make nothing dirty and `git add -A` stages none. Should that block go, `ls` flags them apart from dirty, and the next placement puts it back. |
| links-013 | normal | `normal_013_ls_shows_an_orphan` |  |
| links-014 | normal | `normal_014_sync_removes_a_broken_orphan_by_default` |  |
| links-015a | edge | `edge_015a_sync_skips_an_orphan_with_a_valid_target_without_force` |  |
| links-015b | normal | `normal_015b_sync_force_removes_an_orphan_with_a_valid_target` |  |
| links-016 | edge | `edge_016_sync_inside_a_child_places_the_workspace_and_keeps_the_link` | A child is never a workspace: `sync` typed inside one places the whole workspace, whose root decides the dependency's revision — the link inside the child stays, and the checkout it points at stays at the root's pin. |
| links-017 | edge | `edge_017_naming_a_link_inside_a_child_places_its_checkout_and_keeps_the_link` | A link path named from inside a child names the checkout it points at: that checkout is placed, at the root's revision, and the link is not replaced by a checkout of its own. |
| links-018 | edge | `edge_018_placement_replaces_an_in_workspace_symlink_with_a_checkout` | Any other symlink at an entry path is replaced by a real checkout: placement lands where a fresh one would, and the checkout the link pointed at is left untouched. |
| links-019 | edge | `edge_019_sync_skips_a_clone_holding_untracked_files` | Untracked files are work too, even unstaged: a clone holding one is not replaced by the link without `--force`. |
| links-020 | edge | `edge_020_sync_never_replaces_a_plain_directory_at_a_link_path` | **ignored: bug: is_tree_modified runs git in the plain directory, which answers for repoA; with libs/ ignored there the directory is deleted** Where a link belongs, a plain directory with no repository of its own — files copied in by hand, say — holds nobody knows what. `sync` must not take it for a clean clone (git run inside it answers for the repository around it) and delete it. |
| links-021 | edge | `edge_021_sync_leaves_a_symlink_the_repository_tracks` | **ignored: bug: orphan detection only looks at where a link points, so a tracked symlink to a root child is removed as an orphan** A symlink the repository itself tracks is its content, not a link gitscale planted — even one that points at a direct child of the root, broken or not. `sync` leaves it alone, `--force` included. |

### ls

| ID | Kind | Test | What it holds |
|---|---|---|---|
| ls-001 | normal | `normal_001_table_shows_a_missing_checkout` |  |
| ls-002 | normal | `normal_002_table_shows_a_checkout_at_its_pin` |  |
| ls-003 | normal | `normal_003_json_lists_each_checkout` |  |
| ls-004 | normal | `normal_004_table_shows_a_missing_artefact` |  |
| ls-005 | normal | `normal_005_table_shows_an_installed_artefact` |  |
| ls-006 | normal | `normal_006_flags_a_pin_the_checkout_has_not_followed` | The other side of the same rule: detached somewhere the revision does not point is a mismatch, which comparing the two columns as text used to miss for exactly the repos that are pinned. |
| ls-007 | edge | `edge_007_a_detached_tag_pin_is_not_a_mismatch` | A checkout at its pin is detached, so REF reads as a commit while EXPECTED reads as the tag. That is not a mismatch, and `ls` must not dress it as one — the yellow REF and a `ref-mismatch` flag both have to key off where HEAD actually is. |
| ls-008 | edge | `edge_008_json_with_no_repos_is_json` | A consumer of `--format json` must get JSON whatever the workspace holds. |
| ls-009 | error | `error_009_fetch_reports_a_fetch_it_could_not_do` | `--fetch` asked for fresh state; one it could not get is said so, not silently replaced by whatever the last fetch saw. |
| ls-010 | normal | `normal_010_a_dirty_checkout_in_the_table_and_the_json` | Uncommitted work alone: `dirty` in STATUS with the bright red `!`, and `clean: false` in the JSON. |
| ls-011 | normal | `normal_011_counts_commits_ahead_of_and_behind_the_upstream` | On a topic branch with an upstream, commits on either side are counted: `+N` with `⇑`, `-N` with `⇓`, both with `⇅` — in the table and as `ahead` and `behind` in the JSON. |
| ls-012 | normal | `normal_012_a_symlinked_entry_shows_where_it_points` | An entry whose directory is a symlink to a checkout somewhere else is reported as `symlink` alone, with `⤷`, where it points in PATH and the ref of what it points at — and the same in the JSON. |
| ls-013 | normal | `normal_013_json_lists_the_untracked_links` | Planted links git sees as untracked — once GitScale's block in the checkout's `info/exclude` is gone — are listed in the JSON, relative to the checkout, and do not make it unclean there either. |
| ls-014 | normal | `normal_014_json_carries_an_artefacts_remote_state_and_flags` | An artefact's JSON carries what the last fetch saw beside what is installed, and the flags they make: `ref-mismatch` and `missing` while the release wanted has no image, `ref-mismatch` once it has. The table's icon follows: `!`, then `≠`, both in bright red. |
| ls-015 | normal | `normal_015_fetch_reports_what_the_registry_has_now` | `ls --fetch` asks the registry itself: the same run reports what it just found, with no separate `fetch` before it. |
| ls-016 | normal | `normal_016_a_topic_is_named_with_what_may_merge_next` | On a topic, the table starts with the topic's branch and what may merge next, and the JSON carries both as an object of their own, with the row's `topic` saying which branch it is on and that it is joined here. |
| ls-017 | normal | `normal_017_a_topic_branch_only_the_remote_has_is_from_remote` | A topic branch only the remote has yet is `topic, from remote` until a placement puts it on a local branch. |
| ls-018 | normal | `normal_018_explain_marks_the_override_and_what_it_overruled` | `git explain` marks the requests an override beat and the override itself. |
| ls-019 | normal | `normal_019_a_workspace_with_no_repos_says_so` | A workspace that declares nothing says so, rather than printing an empty table. |
| ls-020 | normal | `normal_020_explain_with_nothing_shared_says_so` | `git explain` alone, when no checkout is asked for by more than one repository, says so rather than printing nothing. |
| ls-021a | edge | `edge_021a_an_orphan_link_whose_target_resolves` | An orphaned link whose target still resolves is `orphan` in yellow; one whose target is gone is `orphan, broken` in bright red. Both carry `⊘`, and both are rows of their own in the JSON. |
| ls-021b | edge | `edge_021b_an_orphan_link_whose_target_is_gone` | See `edge_021a`: the same with the target gone. |
| ls-022 | edge | `edge_022_explain_takes_a_directory_with_a_trailing_slash` | A directory named with the trailing slash a shell completes is the same checkout to `git explain`. |
| ls-023 | edge | `edge_023_an_unlinked_clone_in_a_repository_that_does_not_ignore_it` | A clone standing where a link belongs is `unlinked`, and that is the whole story: the path is kept out of the owner's git status, so the owner is not called `dirty` for it. |
| ls-024 | edge | `edge_024_fetch_falls_back_to_what_was_fetched_before` | A remote that cannot be reached when `--fetch` asks for it: `ls` still reports, from what was fetched before, and says so on stderr. |
| ls-025 | error | `error_025_a_graph_that_cannot_be_resolved_still_gets_a_table` | A graph that cannot be resolved as a whole still gets its table: the error on stderr as `error: …`, and every declared entry marked `unresolved`, pointing at it. JSON stays JSON. This pins the current exit status, 0; whether a failed resolution should fail `ls` is a decision for the owner. |
| ls-026 | error | `error_026_verbose_fetch_keeps_json_parseable` | **ignored: bug: ls -v --fetch --format json prints 'Fetching …' into the JSON on stdout** `-v` names each artefact as `--fetch` asks the registry about it — on stderr, or anywhere but in the JSON a consumer parses. |
| ls-027 | perf | `perf_027_without_fetch_ls_asks_nothing_of_the_network` | Without `--fetch`, `ls` reads only this machine: not one registry request, and no attempt at a remote that has gone away. |
| ls-028 | normal | `normal_028_a_topic_slot_waits_on_the_topic_slots_it_asks_for` | A topic slot that asks for another topic slot not yet promoted waits on it, and the one waited on is what may merge next. |
| ls-029 | normal | `normal_029_a_topic_branch_behind_the_pin_says_rebase_it` | A topic branch cut before the release the graph now pins is behind it, and the row says what to do: rebase. |
| ls-031 | edge | `edge_031_a_shallow_checkout_behind_its_upstream_is_stale` | `stale` stands in for a behind count where a depth-1 checkout has no history to count: a shallow checkout on a branch whose upstream has moved is `stale`, with `≠` in bright red, and `"stale": true` in the JSON. gitscale's own CI checkouts are detached and so have no upstream; this puts one on a branch by hand to reach the flag. |
| ls-032 | edge | `edge_032_a_checkout_git_cannot_read_is_not_ok` | **ignored: bug: ls reads a failed git status as clean, so a checkout git cannot read shows ok** A checkout git cannot read — its `.git` names a git directory that is gone — is not `ok`: git could not say whether it is clean, or where its HEAD is, and `ls` must not claim either. |
| ls-033 | normal | `normal_033_a_topic_branch_never_pushed_is_ahead_by_what_no_remote_has` | A topic branch never pushed has no upstream: its commits that no remote branch or tag has are its `+N` — work only this machine holds. Pushed, they are counted against the upstream again, and none is left. |
| ls-034 | normal | `normal_034_at_a_terminal_ls_says_when_its_stores_were_fetched_long_ago` | Read at a terminal, `ls` says when what it read from the stores is an hour old or more, and how to refresh it; not after `--fetch`, and never off a terminal, where a script reads it. |
| ls-035 | normal | `normal_035_changes_in_the_root_on_a_pinned_branch_are_pointed_at_a_topic` | The root is never read-only, so changes made in it on the branch it pins are pointed at a topic; on a topic, where they belong, nothing is said. |

### man

| ID | Kind | Test | What it holds |
|---|---|---|---|
| man-001 | normal | `normal_001_hook_install_writes_the_pages_where_man_finds_them` | `hook install` writes every page, stamped with this version; a page another version wrote is rewritten by the next interactive run; and `man`'s own search path, from `PATH` alone, holds them. |

### network

| ID | Kind | Test | What it holds |
|---|---|---|---|
| network-001 | normal | `normal_001_in_ci_a_moved_branch_resolves_to_its_new_commit` | In CI a branch resolves to where its remote has it now, though the build directory still holds what the last job fetched. |
| network-002 | error | `error_002_in_ci_an_unreachable_remote_fails_instead_of_using_old_refs` | In CI a remote that cannot be reached fails resolution, rather than building what an earlier job fetched; off CI the refs fetched before still answer, with a warning. |
| network-003 | normal | `normal_003_a_forwarded_command_places_online_only_in_ci` | After a git command that moved a `HEAD`, a developer machine fetches only what resolution lacks, so a branch it has is not fetched again; in CI the placement is online. |

### output

| ID | Kind | Test | What it holds |
|---|---|---|---|
| output-001 | normal | `normal_001_a_piped_table_has_no_escape_codes` | The table piped — into a file, a CI log — has no escape codes. |
| output-002 | normal | `normal_002_without_colour_result_lines_are_the_words_alone` | Without colour the result lines are exactly the words: `  ok    …`. |
| output-003 | normal | `normal_003_color_always_colours_piped_output` | `--color always` colours piped output, icons included. |
| output-004 | normal | `normal_004_no_color_dumb_terminals_and_never_turn_colour_off` | On a terminal, `NO_COLOR`, `TERM=dumb` and `--color never` each turn colour and icons off; with none of them they are on. |
| output-005 | normal | `normal_005_git_colours_exactly_when_gitscale_does` | Git is told `color.ui=always` exactly when GitScale colours its own output, so the two agree. |

### placement

| ID | Kind | Test | What it holds |
|---|---|---|---|
| placement-001 | normal | `normal_001_makes_each_checkout_a_detached_readonly_worktree_of_the_root_store` | Every checkout is a worktree of the root's own store for its repository, detached at the commit its revision names, every file read-only. |
| placement-002 | normal | `normal_002_a_second_placement_leaves_a_checkout_where_it_is` |  |
| placement-003 | normal | `normal_003_directories_select_which_entries_to_place` |  |
| placement-004 | normal | `normal_004_places_checkouts_and_artefacts_together` |  |
| placement-005 | normal | `normal_005_moves_a_checkout_to_a_branch_made_after_it` | The refs come first, then the move: a checkout pinned to a branch that did not exist when it was made finds it, and lands detached at its tip. |
| placement-006 | normal | `normal_006_without_a_revision_follows_the_default_branch` | No revision means the remote's default branch: sync moves to its new head. |
| placement-007 | edge | `edge_007_an_all_hex_tag_name_is_a_tag` | A tag whose name is all hex digits is a tag, not a commit. |
| placement-008 | edge | `edge_008_ignores_an_inherited_git_dir` | git exports `GIT_DIR` to hooks — during `git clone`, the new root's `.git`. A hook-triggered placement that let its git calls inherit it ran them against the root: the pinned tag "did not exist", and with an existing store the fetch rewrote the root's refs and checked the sub-repo's tag out over it. |
| placement-009 | edge | `edge_009_refuses_to_lose_commits_made_at_a_pin` | Commits made on a checkout's detached HEAD are somebody's work no branch holds: sync will not move away from them. |
| placement-010 | edge | `edge_010_clones_into_an_empty_directory_and_leaves_the_workspace_alone` |  |
| placement-011 | edge | `edge_011_moves_a_checkout_only_when_nothing_can_be_lost` |  |
| placement-012 | edge | `edge_012_a_move_never_overwrites_an_untracked_file` | A checkout moves with a plain checkout, so an untracked file in the way of the new revision stops the move rather than being overwritten. |
| placement-013 | edge | `edge_013_does_not_move_a_detached_head_with_commits_on_no_branch` | A detached HEAD with a commit no branch holds is not moved: switching away would leave that commit reachable only through the reflog. |
| placement-014 | error | `error_014_an_abbreviated_sha_fails_with_a_hint` | A commit is only ever a full SHA. An abbreviated one is taken for a branch or tag name, which the remote does not have — and the failure says why. |
| placement-015 | error | `error_015_an_unknown_name_is_refused` |  |
| placement-016 | error | `error_016_fails_when_the_remote_is_unreachable` | A sync that cannot reach the remote fails, rather than reporting `ok` for a checkout it never updated. |
| placement-017 | error | `error_017_fails_when_local_changes_block_the_move` | A move that local changes block fails, and the changes survive. |
| placement-018 | error | `error_018_refuses_a_directory_that_holds_something_else` |  |
| placement-019 | error | `error_019_one_failing_entry_fails_the_command_after_the_rest_is_done` | One entry that cannot be brought up to date — an artefact whose pipeline has not published yet — fails the command, but only after everything else is done: the rest placed and linked. |
| placement-020 | edge | `edge_020_pins_an_annotated_tag_at_its_commit` | An annotated tag is an object of its own; the checkout lands on the commit it points at, as git itself would check it out. |
| placement-021 | edge | `edge_021_a_full_sha_pins_that_commit` | A full SHA is the commit itself, however it is spelt: a commit behind the tip is checked out exactly, in lower or upper case. |
| placement-022 | edge | `edge_022_a_branch_wins_over_a_tag_of_the_same_name` | A branch and a tag may share a name; the branch wins, as the resolver documents (`ls_remote_revision`). |
| placement-023 | edge | `edge_023_with_no_entries_says_there_is_nothing_to_sync` | A config with no entries has nothing to sync: the command says so and succeeds. |
| placement-024 | edge | `edge_024_a_commit_on_a_local_branch_does_not_block_a_move` | Commits on a local branch are kept by the branch, so they do not block a move: the checkout moves to its new pin and the branch keeps the work. |
| placement-025 | edge | `edge_025_moving_an_outer_checkout_leaves_a_nested_checkouts_write_bits` | **ignored: bug: restore_writable/apply_readonly on the outer checkout walk into the nested one, leaving its files read-only** Entries may nest (`deps` and `deps/inner`). Moving the outer checkout must leave the inner one's files as they were: a joined inner checkout stays writable. |
| placement-026 | error | `error_026_a_revision_the_remote_lacks_fails_naming_it` | A revision the remote has neither as a branch nor as a tag fails the entry, naming the revision, and leaves nothing behind. |
| placement-027 | error | `error_027_never_places_a_checkout_outside_the_workspace_through_a_symlink` | **ignored: bug: directories are checked lexically only, so a symlinked component lets a checkout land outside the workspace** A checkout stays inside the workspace even when a directory on its path is a symlink out of it — one the root repository tracks arrives with any branch, just as the config does. |
| placement-028 | perf | `perf_028_fetches_each_store_once_per_placement` | Each store is fetched once per sync, however many times resolution and placement ask for it — the first sync and every later one. |
| placement-029 | edge | `edge_029_a_name_with_a_trailing_slash_selects_its_entry` | **ignored: bug: names are matched as exact strings, so `sync libs/lib1/` is 'Unknown repos: libs/lib1/'** A name with a trailing slash — what shell completion of a directory gives — selects the entry, as it does for `git topic join`. |

### post_sync

| ID | Kind | Test | What it holds |
|---|---|---|---|
| post_sync-001 | normal | `normal_001_runs_for_an_allowed_repository` |  |
| post_sync-002 | normal | `normal_002_runs_when_the_user_invoked_gitscale` | A `git scale sync` the user typed is not a drive-by: they chose the directory and the moment, so the allowlist — which belongs to the installed hook — does not apply. |
| post_sync-003 | edge | `edge_003_a_config_cannot_allowlist_itself` | A repository must not be able to vouch for itself: the allowlist reaches gitscale from the hook shim, never from the `.gitscale.toml` under test. |
| post_sync-004 | edge | `edge_004_a_lookalike_owner_is_refused` | An owner pattern ends at the slash, so a lookalike owner is a different owner. This is the whole value of the allowlist. |
| post_sync-005 | edge | `edge_005_a_workspace_without_a_remote_matches_on_its_path` | A workspace with no remote has no host or owner to match on, so only a path pattern can name it. |
| post_sync-006 | edge | `edge_006_an_empty_allowlist_refuses_everything` | A hook installed with an empty allowlist runs nothing at all, rather than reading as "unrestricted". |
| post_sync-007 | error | `error_007_a_failing_command_fails_the_sync` |  |
| post_sync-008 | error | `error_008_is_refused_for_an_unlisted_repository` | The reported attack: a branch carries its own `.gitscale.toml`, a developer clones it to review, and the shared hook runs the payload as them. The allowlist the hook was installed with is what has to stop it. |
| post_sync-009 | normal | `normal_009_sync_runs_it_exactly_once` | `sync` places the checkouts as one of its steps, and the docs promise `post_sync` runs once, not twice: a command that installs or migrates something must not run again on top of itself. |
| post_sync-010 | normal | `normal_010_every_spelling_of_an_allowed_repository_is_allowed` | Patterns are written against `host/owner/repo`, which every spelling of one repository shares: SSH or HTTPS, any case, a default port, a trailing slash, a `.git` suffix, credentials in the URL. |
| post_sync-011 | edge | `edge_011_verbose_output_names_the_pattern_and_the_command` | With `-v` an allowed command says which pattern let it through and what it runs — what someone checking an allowlist wants to see. |
| post_sync-012 | edge | `edge_012_a_blank_command_runs_nothing_and_is_never_refused` | A blank `post_sync` is no command at all: it is not run, and so there is nothing for even an empty allowlist to refuse. |
| post_sync-013 | edge | `edge_013_a_path_pattern_also_admits_a_workspace_that_has_a_remote` | A path pattern is documented "for a workspace with no remote", but the workspace's path is matched whatever its remote: a path pattern admits every repository cloned under that directory, whoever it came from. Pins current behaviour; listed for the owner to decide. |
| post_sync-014 | error | `error_014_is_skipped_after_a_failed_sync` | `post_sync` runs after a sync that worked. When the sync fails it does not run — a build step must not run over checkouts that are not there. |
| post_sync-015 | error | `error_015_disguised_remotes_are_refused` | Remotes spelled to look like an allowed repository while naming another: the allowed host in the credentials or the path, a `..` that climbs out of the owner (plain or percent-encoded), a lookalike Unicode host, a lookalike owner. None of them passes `github.com/thepartly/*`. |
| post_sync-016 | error | `error_016_an_encoded_slash_does_not_pass_for_a_path_under_the_owner` | **ignored: bug: a %2F-encoded `..` path passes for a repository under the owner it climbs out of** A percent-encoded slash keeps `..` segments out of sight of the dot-segment check: `acme/x%2F..%2F..%2Fevil%2Fy` reads, to a glob, as a repository under `acme`, while a server that decodes `%2F` serves evil's. Such a remote must answer to no owner pattern, as a plain `..` does. |
| post_sync-017 | error | `error_017_a_star_owner_pattern_does_not_admit_a_subgroup_named_like_the_owner` | **ignored: bug: `*/acme/*` matches gitlab.com/evil/acme/x, not only owner acme** The docs give `*/acme/*` as "that owner on any host". Because `*` crosses `/`, it also matches any path with an `acme` segment further down — an attacker's own group with a subgroup called `acme`, on any host. |
| post_sync-018 | error | `error_018_the_refusal_never_prints_credentials_from_the_origin` | An `origin` can carry credentials — older GitLab runners check out with the job token in the URL. The refusal names the repository by host and path, and never prints them. |
| post_sync-019 | error | `error_019_the_refusal_escapes_and_shortens_the_command` | The refused command is echoed for the developer to read, so it is made safe for a terminal first: escape sequences that could repaint the screen show as text, and a long command is cut short. |
| post_sync-020 | error | `error_020_a_host_that_only_folds_to_an_allowed_one_is_refused` | **ignored: bug: matching lowercases with Unicode rules, so a U+212A host passes for one spelled with k** Some non-ASCII letters lowercase to ASCII ones: U+212A KELVIN SIGN becomes `k`. A host spelled with it is a different host from the one spelled with `k`, and must not pass for it. |

### prefer

| ID | Kind | Test | What it holds |
|---|---|---|---|
| prefer-001 | normal | `normal_001_prefer_records_and_the_next_placement_applies_it` | `prefer` records and changes nothing; `ls` shows the form to come, and the next placement applies it — artefact, and back to sources. |
| prefer-002 | normal | `normal_002_a_preference_follows_the_repository_into_every_worktree` | A preference is the repository's, not the directory's: a checkout moved to another directory keeps it, and every worktree of the root sees it. |
| prefer-003 | edge | `edge_003_a_checkout_holding_work_keeps_its_sources` | A checkout holding work is never replaced by its artefact: the placement reports it and keeps it, and the work stays. |
| prefer-004 | error | `error_004_a_form_needs_checkouts_it_names` | A form needs the checkouts it is for; a name no checkout has is an error, and nothing is recorded. |

### registry

| ID | Kind | Test | What it holds |
|---|---|---|---|
| registry-001 | normal | `normal_001_a_ci_job_logs_in_with_its_job_token` |  |
| registry-002 | normal | `normal_002_a_docker_login_is_used_outside_ci` |  |
| registry-003 | normal | `normal_003_publish_and_sync_against_a_real_registry` |  |
| registry-004 | normal | `normal_004_other_tools_read_what_gitscale_publishes` |  |
| registry-005 | normal | `normal_005_gitscale_reads_what_other_tools_publish` |  |
| registry-006 | edge | `edge_006_the_job_token_never_goes_to_a_token_service_elsewhere` |  |
| registry-007 | error | `error_007_a_refusal_says_how_to_get_access` |  |
| registry-008 | error | `error_008_a_missing_tag_is_reported_by_a_real_registry` |  |
| registry-009 | normal | `normal_009_a_basic_challenge_is_answered_with_the_stored_login` | A registry that challenges with `Basic` rather than a token service gets the stored login directly, as `docker` does. |
| registry-010 | normal | `normal_010_a_stored_identity_token_is_exchanged_for_a_bearer_token` | What `docker login` stores for some registries is an OAuth2 refresh token (`identitytoken`), which gitscale exchanges at the token service for an access token rather than sending as a password. |
| registry-011 | normal | `normal_011_a_credential_helper_is_asked_for_the_registry` | A login kept by a credential helper (`credsStore`, as Docker Desktop and `pass` set up) is asked for by running `docker-credential-<name> get` with the registry on its input. |
| registry-012 | normal | `normal_012_a_podman_auth_file_is_read_when_docker_has_no_login` | With no Docker login, Podman's auth file (`REGISTRY_AUTH_FILE`) is read: `podman login` and `oras login` write it. |
| registry-013 | normal | `normal_013_a_token_named_access_token_is_used` | A token service may hand the token out as `access_token` (the OAuth2 name) rather than `token`. |
| registry-014 | normal | `normal_014_publish_logs_in_for_push_access` | `publish` to a registry that needs a login asks the token service for a token that may push (`pull,push` on the image's repository), with the stored login, and publishes. |
| registry-015 | edge | `edge_015_a_plain_http_registry_off_this_machine_gets_no_bearer_token` | **ignored: bug: a Bearer token is sent to a plain-HTTP registry off this machine** A registry spoken to over plain HTTP, off this machine, is used anonymously, as the docs promise: no credential crosses the network in the clear. That covers the token a token service hands out for a stored login, not only the login itself. |
| registry-016 | edge | `edge_016_a_plain_http_registry_off_this_machine_gets_no_basic_login` | The same registry answering with a `Basic` challenge gets no stored login: that would be the password itself, in the clear. |
| registry-017 | edge | `edge_017_the_job_token_never_goes_to_a_registry_the_ci_server_does_not_own` | The CI job token goes only to the registry the CI server owns (`CI_REGISTRY`): another registry gets nothing, even one whose token service is on its own host. |
| registry-018a | edge | `edge_018a_an_upload_location_on_another_host_gets_no_token` | **ignored: bug: the cached Authorization header goes to whatever URL the upload location names** An upload location on another host gets none of the registry's credentials: the token the registry's token service issued is for the registry. |
| registry-018b | edge | `edge_018b_an_upload_location_on_another_host_never_gets_the_job_token` | **ignored: bug: the job token is sent to an upload location on another host** The same upload location, asking for a `Basic` login, never gets the CI job token: it goes only to the registry the CI server owns. |
| registry-019 | edge | `edge_019_a_tag_list_page_on_another_host_gets_no_token` | **ignored: bug: the cached Authorization header goes to whatever URL the tag list's next page names** A tag list whose next page is on another host gets none of the registry's credentials there. |
| registry-020 | edge | `edge_020_a_blob_redirect_to_another_host_carries_no_authorization` | A blob download the registry redirects to another host — storage or a CDN, as most registries do — carries no `Authorization` there. |
| registry-021 | edge | `edge_021_a_manifest_without_a_content_digest_header_is_digested_locally` | Not every registry sends `Docker-Content-Digest`: without it, the digest is read from the manifest itself, and the sync works the same. |
| registry-022 | error | `error_022_registry_errors_are_reported_with_their_status` | A registry that answers with a server error or a rate limit is reported with the status and what it said, and nothing is installed. |
| registry-023 | error | `error_023_a_refused_login_says_how_to_log_in_and_hides_the_password` | A stored login the token service refuses ends in a refusal that says how to log in, and never shows the password. |
| registry-024 | error | `error_024_registry_errors_never_print_credentials` | **ignored: bug: error bodies are printed verbatim, credentials the registry echoes included** What a registry says back in an error is shown to help — but never a credential it echoes: a job log would keep the job token, base64-encoded where CI masking does not recognise it. |
| registry-025 | error | `error_025_a_refused_publish_in_ci_says_what_the_job_token_may_do` | A GitLab job token refused for a push says the one thing that can fix it: the job token publishes only to its own project's registry. |
| registry-026 | error | `error_026_a_manifest_not_matching_its_digest_is_refused` | A registry whose manifest does not match the digest it claims for it is not believed: the sync fails, and nothing is installed. |
| registry-027 | error | `error_027_a_cut_off_download_leaves_no_partial_file_and_the_old_install` | A layer download cut off part way fails the sync, leaves no partial file in the image store for a reader to find, and leaves the installed version alone. |
| registry-028 | perf | `perf_028_one_token_exchange_per_scope_per_command` | Tokens are kept for the length of a command: a sync asks the token service once for its one scope, however many manifests and blobs it reads; a publish once for reading and once for pushing. |
| registry-029 | perf | `perf_029_a_tag_list_that_never_ends_is_cut_off` | A registry that names a next page of its tag list forever is not followed forever: `artefact list` stops after a thousand pages and says why. |

### require

| ID | Kind | Test | What it holds |
|---|---|---|---|
| require-001 | normal | `normal_001_requires_an_entry_reports_it_and_checks_it_out` | The entry is written, said, and checked out at the revision it names. |
| require-003 | edge | `edge_003_keeps_comments_and_every_other_table` | Edited in place: comments, key order, keys of other entries and tables gitscale does not know about are all still there, after `require` and after `unrequire`. |
| require-004 | error | `error_004_refuses_a_directory_already_declared` |  |
| require-005 | error | `error_005_refuses_an_entry_the_config_would_reject` | `require` holds an entry to the rules every later command reads the config by, rather than writing one that leaves the workspace unloadable. |
| require-006 | edge | `edge_006_creates_a_config_when_none_exists` | With no config anywhere above, `require` starts one at the top of the repository, and every later command can read it. |
| require-007 | error | `error_007_leaves_an_unparseable_config_untouched` | A config that does not read stops `require`, and stays as it is. |
| require-008 | error | `error_008_refuses_a_directory_already_declared_under_another_spelling` | The duplicate check is about directories, not spellings: `libs/core/` and `./libs/core` are the directory `libs/core` already declares. |
| require-009 | normal | `normal_009_unrequire_removes_the_entry_and_its_clean_checkout` | Taken out: the entry goes, the rest of the file stays, and its checkout — holding nothing of anyone's — goes with it. |
| require-010 | normal | `normal_010_without_a_revision_none_is_written` | No revision given, none is written: the entry follows the default branch. |
| require-011 | error | `error_011_unrequire_refuses_an_undeclared_directory` |  |
| require-012 | edge | `edge_012_an_unrequired_checkout_with_work_is_kept` | An unrequired checkout that holds work stays, and the command says so and fails, as every placement does. |
| require-013 | edge | `edge_013_the_directory_is_relative_to_the_current_directory` | The directory is a path from where the command runs. |
| require-014 | error | `error_014_unrequire_leaves_an_unparseable_config_untouched` | A config that does not read is left exactly as it is. |

### resolution

| ID | Kind | Test | What it holds |
|---|---|---|---|
| resolution-001 | normal | `normal_001_a_root_entry_without_a_revision_takes_a_dependencys` | A root entry without a revision gets the one a dependency asks for. |
| resolution-002 | normal | `normal_002_the_root_revision_wins` |  |
| resolution-003 | normal | `normal_003_a_tag_a_dependency_asks_for_lands_detached_and_readonly` | A tag a dependency asks for, for a root entry without a revision, is where the checkout lands — not on the default branch's tip — and its files are readonly. |
| resolution-004 | normal | `normal_004_sync_moves_to_a_revision_a_dependency_starts_asking_for` | A revision a child pins for a root entry that has none is applied by sync too, as a fresh clone would — including one the sync itself brings in. |
| resolution-005 | normal | `normal_005_an_undeclared_dependency_is_checked_out_implicitly_where_allowed` | A dependency the root does not declare is checked out implicitly, under `imports/` and readonly — but only from somewhere the allowlist covers. The root's own entries allow their host and owner; a local path is never allowed that way, only by `[resolve] allow`. |
| resolution-006 | normal | `normal_006_an_artefacts_dependencies_are_linked_inside_it` | A checkout taken as an artefact has the dependencies its repository's config declares, linked inside it like any checkout's. |
| resolution-007 | normal | `normal_007_a_dependency_raises_the_root_and_ls_and_explain_say_why` |  |
| resolution-008 | normal | `normal_008_an_override_at_the_root_holds_a_dependency_down` |  |
| resolution-009 | normal | `normal_009_two_majors_get_a_checkout_each_unless_one_is_a_singleton` |  |
| resolution-010 | normal | `normal_010_calendar_versions_order_by_date_then_modifier` |  |
| resolution-011 | normal | `normal_011_an_artefacts_dependencies_are_checked_out_implicitly` | The dependencies of a checkout taken as an artefact are resolved from its repository's config before anything is installed, checked out implicitly and linked inside the artefact. |
| resolution-012 | normal | `normal_012_in_ci_resolution_reads_each_config_from_its_commit` | In CI, resolution reads each config from the commit the checkout is built from, with no history fetched; and a move is never refused. |
| resolution-013 | normal | `normal_013_an_override_in_a_dependency_reaches_only_what_it_is_above` | An override in a dependency: it wins over what that dependency is above, and must agree with what it is not. |
| resolution-014 | normal | `normal_014_explain_shows_the_shared_checkouts_or_the_ones_named` |  |
| resolution-015 | normal | `normal_015_placement_follows_the_hoist_dir_majors_kind_and_names` |  |
| resolution-016 | edge | `edge_016_recursive_false_leaves_a_dependencys_config_unread` |  |
| resolution-017 | edge | `edge_017_a_revision_a_dependency_asks_for_leaves_the_workspace_repo_alone` | An artefact has no git checkout to move: a revision a dependency asks for must not send `git checkout` up into the workspace repo. |
| resolution-018 | edge | `edge_018_an_override_from_above_works_around_a_missing_revision` | A dependency pinned to a revision its repository does not have fails resolution — unless the root overrides that dependency, which is how it works around a broken pin it cannot edit. |
| resolution-019 | edge | `edge_019_ls_is_unresolved_until_fetched` | Offline, `ls` reads what is on this machine: a repository nothing has fetched yet is unresolved until `ls --fetch`. In CI, where checkouts hold no history, that is the light stores resolution keeps. |
| resolution-020 | edge | `edge_020_an_uncommitted_config_edit_takes_effect` | A dependency's checkout at the selected commit is read from disk, so an edit to its `.gitscale.toml` not yet committed takes effect at once. |
| resolution-021 | edge | `edge_021_one_checkout_whatever_form_the_workspace_takes_it_in` | How the workspace takes a repository plays no part in resolution: two requests for it are one checkout, the image when the workspace prefers its artefact, and both requesters link to it. |
| resolution-022 | edge | `edge_022_a_winner_behind_a_request_is_flagged_where_history_is_local` | Where the root's store holds history, a winner by position that is behind what a losing request asked for is flagged — and still wins. |
| resolution-023 | edge | `edge_023_a_missed_row_has_no_resolution_text` | A row with nothing on disk says `missed` and nothing more, even where resolution has a story to tell. |
| resolution-024 | error | `error_024_two_branches_nothing_orders_fail_the_sync` |  |
| resolution-025 | error | `error_025_a_cycle_fails_before_anything_is_cloned` |  |
| resolution-026 | error | `error_026_bad_resolution_config_is_refused_when_read` |  |
| resolution-027 | perf | `perf_027_resolving_in_ci_without_the_cache_fetches_no_history` | In CI without the cache, resolution reads refs from `ls-remote` and each config from one commit fetched at depth 1: no history crosses the wire. |
| resolution-028 | normal | `normal_028_an_override_at_the_root_settles_a_singletons_two_majors` | A root override settles a singleton that two dependencies want at two majors: one checkout, at the override, the other request noted as held. Without it the singleton refuses; this is the way out the docs give. |
| resolution-029 | normal | `normal_029_the_root_declares_each_major_under_its_own_name` | The root may declare each major of one repository itself, under names of its own: each dependency is linked to the checkout of the major it asked for, and nothing is hoisted. |
| resolution-030 | normal | `normal_030_a_root_entry_without_a_revision_takes_the_lowest_major` | A root entry with no revision for a repository its dependencies want at two majors is the lowest of them; the other is placed beside it. |
| resolution-031 | normal | `normal_031_two_streams_of_one_monorepo_are_ordered_by_position` | Tags of two streams of one monorepo are not compared as versions: the request from the repository above the other wins, and two siblings cannot be ordered at all. |
| resolution-032 | normal | `normal_032_semver_and_calendar_versions_of_one_repository_get_a_checkout_each` | A repository that moved from semver to calendar versions at its next major, asked for in both: two majors, so each gets a checkout rather than an error. |
| resolution-033 | normal | `normal_033_a_pre_release_beats_the_version_before_it_and_loses_to_its_release` | A pre-release is above the version before it and below its own release, as semver orders them. |
| resolution-034 | normal | `normal_034_the_roots_origin_allows_implicit_dependencies_from_its_owner` | The root repository's own origin allows implicit dependencies from its host and owner, with no `[resolve] allow`: a workspace of one organisation needs no allowlist for that organisation's repositories. |
| resolution-035 | normal | `normal_035_ssh_and_https_spellings_of_one_repository_share_a_checkout` | The SSH and HTTPS spellings of one repository are one repository: a dependency asking for it over SSH raises the root's HTTPS entry and is linked to that one checkout, instead of getting a second checkout of it. |
| resolution-036 | normal | `normal_036_an_implicit_checkout_takes_the_workspaces_preference` | An implicit checkout takes the form the workspace prefers, as a declared one does: here the artefact of its release. |
| resolution-037 | edge | `edge_037_two_requests_naming_one_commit_agree_whatever_they_name` | Two requests naming one commit agree, whatever they name — here a branch and a tag from two siblings, which position alone could not order. |
| resolution-038 | edge | `edge_038_a_new_lower_major_takes_the_plain_name_without_losing_work` | A new, lower major takes the plain name, and the checkout there moves to it — but never at the cost of work: a commit on no branch stops the move, and once it is gone the old major lands beside it. |
| resolution-039 | edge | `edge_039_a_branch_beats_a_tag_of_the_same_name_and_ref_prefixes_choose` | A name that is both a branch and a tag is the branch, as git has it; `refs/tags/` and `refs/heads/` choose explicitly. |
| resolution-040 | edge | `edge_040_a_branch_named_like_a_version_is_a_branch` | Only a tag is read as a version: a branch called `v9.0.0` is a branch, so it joins the checkout of the versions asked for instead of making a major 9 of its own. |
| resolution-041 | edge | `edge_041_an_implicit_checkout_reads_its_dependencies_unless_every_request_says_not` | An implicit checkout's own dependencies are read unless every request for it says `recursive = false`. |
| resolution-042 | edge | `edge_042_an_implicit_calendar_major_beside_a_root_major_takes_its_suffix` | An implicit checkout of a calendar major whose plain name is the root's checkout of another major is placed beside it, with its major's suffix. |
| resolution-043 | error | `error_043_a_repository_depending_on_itself_is_a_cycle` | A repository that lists itself as a dependency is a cycle, refused before anything is cloned. |
| resolution-044 | error | `error_044_a_dependency_on_the_root_repository_is_a_cycle` | A dependency that asks for the root repository — known by its origin — closes a loop through the root, and is refused like any cycle rather than checking the root out inside itself. |
| resolution-045 | error | `error_045_conflicting_overrides_from_siblings_fail` | Two overrides at different commits, neither from a repository above the other, are a conflict only an override above both can settle. |
| resolution-046 | error | `error_046_a_missing_revision_is_not_covered_by_a_siblings_override` | A revision a dependency's repository does not have is covered only by an override from a repository above that dependency; a sibling's override does not excuse it. |
| resolution-047 | error | `error_047_requests_whose_pairwise_winners_go_round_in_a_circle_fail` | Pairwise wins that go round in a circle settle nothing: x above y asks for v1.5.0 over y's `main`, y above z asks for `main` over z's v1.6.0, and v1.6.0 beats v1.5.0. No request is at least every other, so resolution fails rather than letting the order requests were read in pick the winner. |
| resolution-048 | error | `error_048_two_root_entries_of_one_major_are_refused` | Two root entries of one repository in one major are refused: they would be one checkout under two names. |
| resolution-049 | error | `error_049_a_branch_asked_for_beside_two_majors_is_refused` | A branch asked for while the repository is checked out at two majors names no checkout in particular, and is refused with what to ask for instead. |
| resolution-050 | error | `error_050_a_missing_commit_in_a_dependency_fails_reading_its_config` | A full SHA is not checked by name: one the repository does not have fails when its config is read, saying which commit could not be fetched. |
| resolution-051 | perf | `perf_051_a_wide_graph_fetches_each_repository_once` | A wide graph — eight dependencies, each asking for a shared one at a different version — fetches every repository's store exactly once per command, however many requests and resolution rounds there are, and ends with one checkout of the shared one at the highest version. |
| resolution-052 | perf | `perf_052_a_deep_chain_fetches_each_repository_once` | A chain ten deep settles — resolution takes a round per level — while still fetching each repository's store exactly once. |
| resolution-053 | edge | `edge_053_a_chain_seventy_deep_settles` | **ignored: bug: a chain 64 or more deep fails 'did not settle after 64 rounds' (MAX_ROUNDS)** A chain seventy deep — deeper than the rounds resolution allows itself — still settles: a graph without cycles always does. |
| resolution-054 | error | `error_054_an_invalid_dependency_config_names_where_it_was_read` | A dependency's config that is not valid TOML fails resolution, naming the file by the checkout and revision it was read at. |
| resolution-055 | error | `error_055_a_cycle_at_a_revision_that_loses_still_fails` | A cycle is refused even when it exists only at a revision that loses: c v1.0.0 asks for b, c v1.1.0 does not, and b raises c to v1.1.0. The check covers every revision resolution considers, which is what guarantees it ends. |
| resolution-056 | normal | `normal_056_explain_says_when_no_checkout_is_shared` | `git explain` with nothing named and no checkout more than one repository asks for says so, rather than printing nothing. |
| resolution-057 | normal | `normal_057_calendar_majors_get_a_checkout_each` | Calendar versions of two majors are two checkouts, as semver majors are. |
| resolution-058 | error | `error_058_prefixed_tags_from_siblings_cannot_be_ordered` | A tag with a prefix is not a version, so two of them asked for by siblings cannot be ordered, however their numbers compare. |

### skill

| ID | Kind | Test | What it holds |
|---|---|---|---|
| skill-001 | normal | `normal_001_install_status_and_remove` |  |
| skill-002 | normal | `normal_002_the_hint_shows_once_for_every_worktree_of_a_root` |  |
| skill-003 | normal | `normal_003_no_hint_once_a_skill_is_installed` |  |
| skill-004 | edge | `edge_004_without_claude_code_only_the_shared_location` |  |
| skill-005 | edge | `edge_005_sync_and_ls_stay_quiet_when_nobody_is_watching` |  |
| skill-006 | error | `error_006_install_replaces_a_file_gitscale_did_not_write_only_with_force` | A file at the skill's path that gitscale did not write is somebody else's: `install` refuses to replace it until told to with `--force`. |
| skill-007 | error | `error_007_skill_commands_need_a_home` | Without a home there is nowhere to put the skill: each `skill` command says so, for `HOME` unset and for `HOME` empty alike. |
| skill-008 | edge | `edge_008_a_copy_edited_in_its_frontmatter_counts_as_edited_by_hand` | **ignored: bug: the skill's hash leaves out its frontmatter, so edits there are overwritten** The skill's frontmatter is part of what gitscale wrote: a copy whose `description` was edited by hand is edited, and neither `status` nor `install` may treat it as untouched — `install` would overwrite the edit. |
| skill-009 | normal | `normal_009_an_interactive_run_refreshes_an_older_skill` | An interactive run outside CI rewrites an installed skill older than the binary, and says so on stderr: the skill follows the binary without anyone reinstalling it. |
| skill-010 | normal | `normal_010_an_interactive_ls_table_hints_once` | The hint, end to end: an interactive `ls` table with no skill installed prints it once, and every later run in that root stays quiet. |
| skill-011 | edge | `edge_011_no_hint_for_json_in_ci_or_after_a_failure` | No hint where nobody should see one: `ls` as JSON, a run in CI, and a command that failed. None of them uses up the one hint either. |
| skill-012 | error | `error_012_remove_with_a_foreign_copy_removes_nothing` | **ignored: bug: skill remove deletes the first copy, then fails on a foreign second one** `skill remove` removes all of what gitscale wrote or none of it: a file gitscale did not write at one path stops the whole removal before any copy is deleted, as `install` checks every path before writing any. |
| skill-013 | edge | `edge_013_install_replaces_a_newer_copy` | An explicit `install` replaces a copy a newer gitscale wrote. |

### stores

| ID | Kind | Test | What it holds |
|---|---|---|---|
| stores-001 | normal | `normal_001_a_plain_clone_keeps_its_stores_in_its_own_git_directory` | A plain clone: the stores live in its own `.git/gitscale`, and the child is a worktree of one, detached at the pin. |
| stores-002 | normal | `normal_002_a_bare_root_shares_one_store_across_its_worktrees` | A bare root with worktrees, Angel's layout: every root worktree's children are worktrees of the one store in the bare repository, so a second root worktree downloads nothing and sees the first one's branches. |
| stores-003 | normal | `normal_003_a_store_maps_the_remote_branches_to_remote_tracking_refs` | The store keeps the remote's branches apart from its own: a fetch never touches the branches joined checkouts are on, and no other refs come. |
| stores-004 | edge | `edge_004_ls_warns_about_a_root_without_a_fetch_refspec` | `git clone --bare` sets no fetch refspec; `ls` says what that costs. |
| stores-005 | edge | `edge_005_a_deleted_root_worktree_does_not_lock_its_topic` | A root worktree deleted with its children in it leaves entries in the stores that keep its branches locked; the next command prunes them. |
| stores-006 | edge | `edge_006_sync_repairs_children_after_the_root_moves` | Moving a plain-clone root breaks its children's links to the store, which is inside it; sync repairs them. |
| stores-007 | edge | `edge_007_a_foreign_checkout_is_reported_and_left_alone` | A checkout gitscale did not make — a clone of its own, from an older gitscale or by hand — is reported and left alone. |
| stores-008 | edge | `edge_008_a_deleted_store_leaves_its_checkouts_alone_and_says_so` | A store deleted while its checkouts still use it: each checkout is reported as not gitscale's, and left as it is — its files, and whatever work they hold, are not gitscale's to remove. |
| stores-009 | edge | `edge_009_a_deleted_joined_checkout_comes_back_on_its_topic_branch` | A joined checkout deleted by hand comes back, on its topic branch with its commits, on the next sync: the work was in the store all along. |
| stores-010 | edge | `edge_010_an_interrupted_store_creation_is_redone` | A store whose creation was interrupted leaves a staging directory, never a half-made store: the next sync builds the store afresh. |
| stores-011 | edge | `edge_011_a_corrupt_checkout_record_is_rebuilt_by_sync` | An unreadable checkout record is rebuilt by the next sync from what is on disk, so `sync` can still remove a checkout nothing needs any more. |
| stores-012 | edge | `edge_012_a_root_worktrees_sync_leaves_another_worktrees_checkouts_alone` | Each root worktree keeps its own checkout record: a `sync` in one removes only the checkouts it made, never another worktree's. |
| stores-013 | edge | `edge_013_concurrent_syncs_in_two_root_worktrees_both_succeed` | Two root worktrees synced at the same moment share one store: each waits for the other's fetch, and both end up with a worktree of it. |
| stores-014 | error | `error_014_a_half_deleted_store_is_reported_and_left_alone` | A store directory that lost its `HEAD` is no store: the sync fails that entry, names the directory, and deletes nothing — the directory may still hold the records of other checkouts. |
| stores-015 | error | `error_015_a_store_whose_origin_was_removed_gets_it_back` | **ignored: bug: a store whose origin was removed is never given one back** A store keeps its `origin` pointing at the entry's URL: one whose remote was removed by hand gets it back on the next sync instead of failing every fetch from then on. |
| stores-016 | perf | `perf_016_a_sync_fetches_a_store_shared_by_two_majors_once` | Two checkouts of one repository — two majors — share its store, and a sync fetches that store once, not once per checkout. |

### sync

| ID | Kind | Test | What it holds |
|---|---|---|---|
| sync-001 | normal | `normal_001_places_a_fresh_workspace` |  |
| sync-002 | normal | `normal_002_removes_an_implicit_checkout_left_behind` | An implicit checkout nothing asks for any more is removed by `sync` when that loses nothing; a clone made by hand under the hoist directory never. |
| sync-003 | normal | `normal_003_removes_a_left_behind_implicit_artefact` | An implicit artefact nothing asks for any more is removed too, with what gitscale recorded about installing it. |
| sync-004 | normal | `normal_004_removes_a_checkout_whose_entry_was_removed` | An entry removed from the root config goes with the next sync, as long as that loses nothing; renamed, the old checkout goes and the new one comes. |
| sync-005 | normal | `normal_005_takes_an_implicit_checkout_by_name` | `sync` takes the name of an implicit checkout, as it does a declared one. |
| sync-006 | edge | `edge_006_removes_a_checkout_whose_only_change_is_its_links` | A checkout nothing needs, whose only change is the links gitscale planted in it, holds nothing to lose and goes. |
| sync-007 | edge | `edge_007_keeps_a_left_behind_checkout_with_work_and_fails` | A left-behind implicit checkout holding work is kept, and the sync fails at the end saying so; `--force` removes it. |
| sync-008 | edge | `edge_008_never_removes_a_directory_holding_a_wanted_checkout` | A recorded checkout nothing needs is still kept while it holds a checkout that is wanted. |
| sync-009 | edge | `edge_009_never_removes_a_clone_the_user_made_at_a_removed_entry` | **ignored: bug: sync records any directory with a .git at an entry path as its own, so a later sync deletes the user's clone** `sync` removes only checkouts gitscale made. A clone the user made at an entry's path — which `sync` refused to touch — is not gitscale's, and stays when the entry is removed. |
| sync-010 | edge | `edge_010_never_deletes_the_workspace_after_a_dot_entry_is_removed` | **ignored: bug: an entry '.' is accepted and placement records the root as a checkout; once the entry goes, sync deletes the whole workspace, .git included** However a config names the workspace itself — an entry at `.` — no later `sync` deletes the workspace. Here the root is clean and pushed, the case in which a checkout nothing needs would be removed. |
| sync-011 | edge | `edge_011_a_relink_refusal_fails_the_sync_after_placing_and_nothing_is_pushed` | Relink refusing an unlinked checkout with work fails the sync with its reason, after the checkouts are placed — and a sync pushes nothing, then or ever. |
| sync-012 | edge | `edge_012_keeps_a_left_behind_checkout_with_unpushed_commits` | A checkout nothing needs any more is kept while it holds commits no remote has, even with a clean working tree. |
| sync-013 | perf | `perf_013_fetches_each_store_once_per_sync` | However often a sync's steps ask for a store, each store is fetched once per sync. |

### topic

| ID | Kind | Test | What it holds |
|---|---|---|---|
| topic-001 | normal | `normal_001_puts_a_child_on_the_topic_from_its_pin` |  |
| topic-002 | normal | `normal_002_a_child_follows_its_remote_branch_of_the_topic` | The remote has the topic branch: a sync puts the child on it, writable, tracking the remote's. |
| topic-003 | normal | `normal_003_a_new_branch_from_a_topic_carries_its_children` | `git switch -c feat/y` from topic feat/x carries the joined children to feat/y, at the commits they are at, edits included. |
| topic-004 | normal | `normal_004_leave_takes_a_child_back_to_its_pin` |  |
| topic-005 | normal | `normal_005_a_pinned_branch_follows_no_topic` | A branch the root pins builds every child from pins, even one whose remote has a branch of that name. |
| topic-006 | normal | `normal_006_a_pipeline_on_a_topic_takes_children_from_their_branches` | A pipeline on the root's topic branch takes each child whose remote has that branch from it — detached at its tip — and `check` fails until the pins say so. |
| topic-007 | edge | `edge_007_a_dirty_child_stays_when_the_root_leaves_the_topic` | A child with uncommitted changes does not move when the root changes branch: its entry fails, the rest goes on. |
| topic-008 | edge | `edge_008_an_empty_pinned_list_makes_the_default_branch_a_topic` | `pinned = []` pins nothing: the root's main follows every child's main. |
| topic-009 | edge | `edge_009_a_dependency_that_pins_the_topic_keeps_what_it_asks_for_at_its_pins` | A dependency that pins the topic branch keeps its own dependencies at their pins: the root builds what that dependency's pinned branch built. |
| topic-010 | edge | `edge_010_an_older_major_joins_on_a_suffixed_branch` | Two checkouts of one repository share a store, so their topic branches need names of their own: the newest major takes the topic's, the older `<topic>@v<major>`. |
| topic-011 | error | `error_011_refuses_off_a_topic` |  |
| topic-012 | error | `error_012_refuses_an_entry_the_root_overrides` |  |
| topic-013 | error | `error_013_leave_refuses_while_the_remote_has_the_branch` | A topic branch the remote has keeps being followed: stopping refuses until it is deleted there. |
| topic-014 | normal | `normal_014_leaving_and_rejoining_a_topic_keeps_unpushed_commits` | Leaving a topic never loses its commits: a joined child with commits nobody pushed goes back to its pin when the root leaves, and back onto its branch, commits and all, when the root returns — as the topics guide promises. |
| topic-015 | normal | `normal_015_join_carries_uncommitted_edits_onto_the_topic` | Edits made in a checkout before it is joined come along onto the topic branch: `git topic join` is how they get somewhere they can be committed. |
| topic-016 | normal | `normal_016_join_turns_an_installed_image_into_a_source_worktree` | Joining a checkout that is installed as its image makes it a worktree of its source, at the commit the image was built from, on the topic branch — and a sync keeps it so. |
| topic-017 | normal | `normal_017_leave_puts_an_artefact_back_to_its_image` | `git topic leave` on a joined artefact puts its image back: off the topic an artefact is what its registry holds, not a source checkout. |
| topic-018 | normal | `normal_018_sync_fast_forwards_a_joined_child_to_its_pushed_upstream` | A joined child whose topic branch a colleague pushed to is fast-forwarded to it by the next sync. |
| topic-019 | normal | `normal_019_push_pushes_the_root_topic_branch_unless_s_names_others` | On a topic, `git scale push` pushes the root's topic branch too, as its upstream; with `-s`, only the checkouts named. |
| topic-020 | normal | `normal_020_join_names_a_checkout_by_a_dependencys_link` | A checkout can be named by the link a repository has to it: joining `imports/b/libs/d` joins the hoisted `imports/d`. |
| topic-021 | normal | `normal_021_a_new_branch_from_a_topic_carries_older_majors_by_their_suffix` | A new branch from a topic carries an older major's suffixed branch too: `feat/x@v1` becomes `feat/y@v1`, beside `feat/x` becoming `feat/y`. |
| topic-022 | edge | `edge_022_join_on_an_existing_topic_puts_the_checkout_on_its_branch` | **ignored: bug: join says 'already on' a topic branch the checkout is not on** `git topic join` on a topic whose branch the store already has, before any placement has put the checkout on it, puts it on that branch: a checkout left detached and read-only while `git topic join` reports it joined invites commits on no branch. |
| topic-023 | edge | `edge_023_join_in_a_new_root_worktree_of_a_topic_makes_the_checkout` | **ignored: bug: join says 'already on' for a checkout that does not exist yet** In a new root worktree on a topic joined elsewhere, `git topic join` before any placement makes the checkout, on the topic branch — not just a message that it is there. |
| topic-024 | edge | `edge_024_join_keeps_commits_made_at_a_detached_pin` | Commits made at a detached pin are where `git topic join` starts the topic branch, so they end up on it rather than lost. |
| topic-025 | edge | `edge_025_sync_leaves_a_diverged_topic_branch_alone` | A topic branch that has diverged from its upstream is left where it is: a fast-forward cannot reconcile it, and a sync never merges or resets somebody's work. |
| topic-026 | edge | `edge_026_a_followed_branch_deleted_upstream_stays_as_a_local_branch` | A child that followed its remote's topic branch keeps its local copy of it once the remote deletes the branch: a local branch of the topic is rule one of where a checkout goes. `git topic leave` then refuses, since those commits are on no remote any more. |
| topic-027 | edge | `edge_027_sync_on_a_detached_root_puts_joined_children_at_their_pins` | A detached root has no topic: a sync puts joined children back at their pins, and their topic branches keep their commits. |
| topic-028 | edge | `edge_028_commit_on_a_detached_root_commits_the_root_on_no_branch` | On a detached root `commit` commits the root, on no branch, as it does on any root off a topic. |
| topic-029 | edge | `edge_029_dropping_the_newer_major_moves_the_older_onto_the_topics_own_branch` | With the newer major's entry and checkout gone, the older major becomes the highest, and its branch of the topic is the topic's own name — which the store already has, holding the newer major's work. The next sync moves the older major's checkout onto it; its own work stays on `feat/x@v1`. |
| topic-030 | edge | `edge_030_joining_twice_says_it_is_already_on_the_topic` | Joining a checkout that is already on the topic says so and changes nothing. |
| topic-031 | error | `error_031_leave_refuses_a_branch_with_unpushed_commits` | `git topic leave` refuses while the topic branch holds commits no remote has: deleting the branch would lose them. |
| topic-032 | error | `error_032_leave_keeps_unpushed_commits_of_a_branch_the_checkout_is_not_on` | **ignored: bug: leave deletes unpushed commits on a topic branch the checkout is not on** `git topic leave` judges the topic branch, not only where the checkout is: a checkout left detached at its pin (the root came back to the topic without a placement) must not have its branch's unpushed commits deleted. |
| topic-033 | error | `error_033_join_onto_a_remote_topic_keeps_commits_made_at_the_pin` | **ignored: bug: join onto a remote topic branch orphans commits made at the detached pin** Joining onto a topic branch the remote already has moves the checkout to that branch; commits made at its detached pin must survive that move — `sync` refuses the same move for exactly this reason. |
| topic-034 | error | `error_034_join_without_a_directory_fails` |  |
| topic-035 | error | `error_035_an_unknown_directory_fails_and_the_rest_are_joined` | One name that matches no checkout fails, and the others still join. |
| topic-036 | error | `error_036_join_refuses_in_ci` | CI checkouts are copies of exact commits: `git topic join` refuses there. |
| topic-037 | error | `error_037_a_topic_clashing_with_a_store_branch_fails_and_leaves_the_checkout` | A topic whose name git cannot store beside a branch the store already has — `feat/x` after `feat` — fails that checkout and leaves it at its pin. |
| topic-038 | normal | `normal_038_topic_prints_the_branch_of_the_checkout_it_is_run_in` | `git topic` prints the topic: the root's branch in the root, the slot's own branch in a child — `<topic>@v<major>` for an older major's — and nothing, exiting 1, off a topic. |
| topic-039 | error | `error_039_join_and_leave_need_a_directory_and_hint_inside_a_checkout` | `join` and `leave` with nothing named fail, as `git add` does; inside a checkout the hint says how to name it. |
| topic-040 | normal | `normal_040_directories_are_paths_from_the_current_directory` | Directory arguments are paths from the current directory: `.` in a checkout, a relative path from a subdirectory; a path out of the workspace, or the root where a checkout is wanted, fails. |
| topic-041 | normal | `normal_041_start_in_a_plain_clone_branches_from_the_remote_default` | In a plain clone, `start` makes the branch from the remote's default branch, not tracking it, and places the children. |
| topic-042 | error | `error_042_start_refuses_existing_and_pinned_names` | `start` refuses a name that exists here or on the remote, and one the root pins. |
| topic-043 | normal | `normal_043_start_from_a_topic_carries_its_joined_children` | `--from` a topic carries the children joined to it. |
| topic-044 | normal | `normal_044_start_in_a_bare_clone_adds_a_worktree` | In a bare clone, `start` adds a worktree beside the others, named after the branch with every `/` a `-`, and says where to go; an existing directory is refused, and `--dir` chooses another. |
| topic-045 | normal | `normal_045_start_with_worktree_in_a_plain_clone_goes_beside_it` | `--worktree` in a plain clone adds the topic's worktree beside the clone. |
| topic-046 | normal | `normal_046_switch_goes_to_a_colleagues_topic_and_back` | `switch` goes to a colleague's branch, tracking the remote's, and the children with that branch join it; a pinned branch works too; a name that exists nowhere is refused with the `start` hint. |
| topic-047 | normal | `normal_047_switch_in_a_bare_clone_adds_or_finds_the_worktree` | In a bare clone, `switch` adds a worktree for the branch — after which a colleague's new branch is found, the remote branches being fetched now — or, when one is already on it, says where it is. |
| topic-048 | normal | `normal_048_a_branch_prefix_names_the_branch_and_not_the_directory` | `[topic] prefix`: `{user}` is `gitscale.user`, else `$USER`; with neither, `start` says how to set one. A name already prefixed is not prefixed again, the worktree's directory drops the prefix, and `switch` and `finish` take the name either way. |
| topic-049 | normal | `normal_049_without_a_prefix_nothing_is_added` | Without `[topic] prefix` nothing is added. |
| topic-050 | normal | `normal_050_status_says_what_each_joined_repository_still_needs` | `status` shows the root and the joined children, with what each still needs, the merge order and the next command; off a topic it fails. |
| topic-051 | normal | `normal_051_list_shows_every_topic_and_where_it_stands` | `list` shows every topic: the current one marked, its worktree, its joined children and whether it is pushed or merged — a squash merge counting. |
| topic-052 | normal | `normal_052_finish_ends_a_merged_topic_in_a_plain_clone` | `finish` refuses a topic not merged and one with a child still on it — `--force` drops either — and uncommitted changes; then, merged, the clone is back on main with every child at its pin, the branches gone here and kept on the remotes. |
| topic-053 | normal | `normal_053_finish_force_drops_an_unmerged_topic` | `finish --force` drops a topic not merged: the local branch goes, its commit no remote has named with the tip it was at, and the remote's branch stays. |
| topic-054 | normal | `normal_054_finish_in_a_bare_clone_removes_the_worktree` | In a bare clone, `finish` removes the topic's worktree, says where to go when it was run from inside it, and deletes the branch. |
| topic-055 | edge | `edge_055_start_while_on_a_topic_carries_nothing_without_from` | `start` while on another topic begins afresh from the default branch: nothing joined to the old topic comes along — only `--from` carries. |
| topic-056 | normal | `normal_056_a_topic_with_nothing_on_it_is_new` | A topic with nothing on it yet is `new` in `list`, and `status` shows no push state for a repository with nothing to push. |
| topic-057 | normal | `normal_057_finish_force_abandons_a_topic` | `finish --force` abandons a topic with a child still on it: the child goes back to its pin, uncommitted changes refused first, and its commit no remote has is dropped. `switch` brings back what was pushed. |
| topic-058 | edge | `edge_058_finish_force_in_a_bare_clone_drops_a_childs_branch` | In a bare clone, `finish --force` removes the worktree of an abandoned topic with its checkouts, and drops the child's branch from the store. |
| topic-059 | edge | `edge_059_finish_refuses_to_drop_commits_no_remote_has` | A merged topic still has a branch in a store with a commit no remote has — a major's own, left behind: `finish` refuses to drop it unless `--force`. |
| topic-060 | edge | `edge_060_a_worktree_deleted_by_hand_comes_back_with_switch` | A topic's worktree deleted by hand: `list` shows the topic with no worktree, and `switch` makes it again, the joined checkout back on its branch with the commit no remote has. |
| topic-061 | normal | `normal_061_join_dependants_of_a_dependency_joins_those_asking_for_less` | `git topic join --dependants <dir>` joins the checkouts whose configs ask for less than the dependency's newest release — how a raise begins — and leaves those already asking for it. |
| topic-062 | normal | `normal_062_join_dependants_climbs_one_level_from_the_topics_changes` | `git topic join --dependants` alone climbs one level from the topic's changes: the dependants of each checkout carrying one — commits on the topic, or uncommitted work — while none of them is on the topic. One taken off stays off, a checkout joined with nothing to carry is not climbed from, and the root is never joined. |
| topic-063 | normal | `normal_063_off_a_topic_a_terminal_is_told_where_it_is` | Off a topic, `git topic` prints nothing for a script to read, but tells a person at a terminal where they are, on stderr: on a pinned branch, on no branch, or in a checkout the topic holds at its pin. |
| topic-064 | normal | `normal_064_status_fetches_first_and_offline_says_how_old_it_is` | `git topic status` asks the remotes first: a merge of the root's branch since the last fetch is in its answer. `--offline` reads this machine only, and at a terminal says how old that is, once it is an hour or more. |
| topic-065 | normal | `normal_065_a_merge_commit_counts_as_merged` | A root merged with a merge commit, rather than a squash, has every one of its commits in the default branch's history: it is merged, as a topic just started — on the same commit as the default branch — is not. |

### upgrade

| ID | Kind | Test | What it holds |
|---|---|---|---|
| upgrade-002 | normal | `normal_002_promotes_a_child_whose_change_is_released` | Once a release tag holds a joined child's change, `upgrade` writes the tag into the root's config, deletes the child's topic branch and detaches it at the tag. |
| upgrade-003 | normal | `normal_003_raises_a_named_dependency_to_its_newest_release` | `upgrade <dir>` raises a dependency to its newest release in the root's config. |
| upgrade-004 | edge | `edge_004_a_raise_edits_table_style_entries` | A raise edits table-style entries as well as inline ones, comments kept. |
| upgrade-005 | normal | `normal_005_a_raise_edits_the_topics_configs_and_names_requesters_off_it` | `upgrade <dir>` edits only the configs on the topic: a requester off it is named with the command that joins it, and left as it is. Joined by `git topic join --dependants`, it is edited in place, comments kept, and with `--commit` each config is committed alone, as `pin <dep> <tag>`. |
| upgrade-006 | normal | `normal_006_a_dry_run_raise_changes_no_file` | `upgrade <dir> --dry-run` prints every edit and changes nothing. |
| upgrade-008 | normal | `normal_008_a_raise_stays_in_its_major_and_skips_pre_releases_unless_asked` | The newest release a raise picks stays in the pin's major and skips pre-releases; a pre-release pin may move to a later pre-release, and `--major` crosses majors. |
| upgrade-012 | normal | `normal_012_a_dry_run_promotion_changes_nothing_and_the_real_one_deletes_the_remote_branch` | `upgrade --dry-run` on a topic reports the promotion, its edits and the remote branch it would delete, and changes nothing; the real run deletes the slot's topic branch on its remote, which placement and CI would otherwise keep matching by name. |
| upgrade-013 | edge | `edge_013_promotion_leaves_a_slot_whose_change_no_tag_holds` | A topic slot whose change no release holds is left on the topic: its branch, with commits nobody else has, survives `upgrade`, and no pin moves. Promotion deletes the branch of what it promotes, so saying "promoted" here would lose those commits. |
| upgrade-014 | edge | `edge_014_promotion_holds_a_slot_with_uncommitted_work` | Uncommitted work holds a slot on the topic even once a release holds its committed change: the work stays, the branch stays, no pin moves. |
| upgrade-015 | edge | `edge_015_promotion_leaves_another_majors_entry_alone` | **ignored: bug: promotion rewrites every root entry of the repository below the tag, across majors** Promotion stays in the major each pin is in: promoting a slot to v1.1.0 leaves the root's entry for the same repository at v0.9.0 alone — rewritten, the two entries would be one major and the workspace would no longer resolve. |
| upgrade-016 | edge | `edge_016_a_raise_takes_a_dependency_by_its_link_path` | A dependency may be named by the link a repository has to it, as that repository calls it. |
| upgrade-017 | edge | `edge_017_a_requester_on_a_branch_is_reported_and_left_alone` | A requester asking for a branch is reported and left alone. |
| upgrade-018 | edge | `edge_018_root_overrides_hold_what_a_raise_would_change` | Overrides stop a raise where they stand: a dependency the root overrides is not raised at all, and a requester the root overrides cannot be joined, so its pin is reported and left — while the root's own entry is still raised. |
| upgrade-019 | edge | `edge_019_a_raise_with_nothing_to_raise_says_why_and_changes_nothing` | Nothing to raise is said, not done: a pin already at its newest release, and an entry pinned to a branch. |
| upgrade-020 | error | `error_020_misused_options_are_refused_and_change_nothing` | Each misuse of `upgrade` is refused with what to do instead, and leaves the config and the root's branch as they were. Off a topic both forms refuse: only `git topic start` begins one. |
| upgrade-021 | error | `error_021_upgrade_refuses_in_ci` | `upgrade` edits the configs of a developer machine's checkouts; in CI, which keeps none, it refuses. |
| upgrade-022 | normal | `normal_022_a_calendar_raise_stays_in_its_major_and_major_crosses_it` | A calendar raise stays in its major; `--major` crosses to the next. |
| upgrade-023 | error | `error_023_promotion_fails_while_the_remote_branch_holds_unreleased_work` | A promotion that would delete a remote branch holding work its release lacks fails before anything changes: the branch, the config and the checkout's place on the topic all stay. |
| upgrade-024 | error | `error_024_a_refused_deletion_changes_no_config_and_a_second_run_completes` | A remote that refuses the deletion fails the promotion with no config edited; once the deletion can go through, running it again completes. |
| upgrade-025 | normal | `normal_025_promoting_an_implicit_checkout_edits_the_requesters_on_the_topic` | Promoting a checkout only its dependants ask for: the release goes into the config of each requester on the topic, a requester left at its pin keeps its older request, and the higher one wins once both are read. |
| upgrade-026 | normal | `normal_026_a_raise_skips_a_line_split_off_before_the_pin_unless_crossing_a_major` | A raise only takes a release whose history holds the pin: a newer tag on a line split off before the pin would lose what the pin had. `--major` relaxes that, since a new major is often cut on a line of its own. |
| upgrade-027 | normal | `normal_027_promotion_takes_the_newest_release_that_holds_the_change` | Promotion takes the newest release that holds the change: a newer hotfix cut from the pin, without the change, is walked past rather than reported as `no tag`. |
| upgrade-028 | normal | `normal_028_pinned_branches_bind_the_tags_a_raise_takes` | A repository's releases are the tags its pinned branches hold — by default its default branch alone — as its default branch's config says, so a pin older than the policy follows it too. |
| upgrade-029 | error | `error_029_pinned_branches_matching_no_branch_say_so` | Pinned branches that name no branch of the repository are an error that says which repository — not an empty list of candidates that would read as "no release". |
| upgrade-030 | edge | `edge_030_a_pin_no_release_holds_says_so` | A pin no release holds — cut on a branch the repository does not pin — says so, rather than `no tag`. |
| upgrade-031 | normal | `normal_031_promotion_waits_for_the_image_of_a_repository_that_publishes_one` | A repository that publishes artefacts is released when its tag's image is there too — whatever form this workspace takes it in. Until then the promotion waits, saying so. |

### workspace

| ID | Kind | Test | What it holds |
|---|---|---|---|
| workspace-001 | normal | `normal_001_from_a_child_or_inside_one_the_root_is_the_workspace` | From a child with a config of its own, or from deep inside it, the workspace is the root's: a child is never a workspace. |
| workspace-002 | edge | `edge_002_a_child_without_a_config_finds_the_root` | A child with no config of its own finds the root the same way. |
| workspace-003 | normal | `normal_003_a_clone_made_by_hand_is_a_workspace_of_its_own` | A clone made by hand inside the workspace, with a config at its top, is a workspace of its own. |
| workspace-004 | error | `error_004_a_config_not_at_a_repository_top_is_no_workspace` | A config that is not at the top of a repository makes no workspace; the error says so. |

## Unit tests

In `#[cfg(test)]` modules beside the code, by source file.

### src/artefact.rs

- `patterns_mean_what_the_docs_say`
- `patterns_stay_inside_the_root`
- `first_match_wins_and_unmatched_files_stay_behind`
- `exclude_applies_within_its_group`
- `a_group_that_matches_nothing_fails`
- `the_same_files_always_give_the_same_digests`
- `git_metadata_is_never_packed`
- `symlinks_are_kept_inside_the_root_and_refused_outside`
- `executable_bits_survive_a_round_trip`
- `any_name_is_the_artefacts_own`
- `hostile_archives_are_refused`
- `state_reads_missing_changed_and_mismatch`
- `credentials_are_stripped_from_a_published_source_url`

### src/cache.rs

- `periods_parse_the_way_the_flag_spells_them`
- `small_units_are_not_read_as_large_ones`
- `a_bare_m_is_refused_as_ambiguous`
- `an_overflowing_period_is_an_error_not_a_panic`
- `counts_are_not_written_1_entries`
- `elapsed_time_reads_in_the_roughest_useful_unit`
- `sizes_read_like_du`

### src/ci.rs

- `detects_gitlab_job`
- `keeps_the_port_the_ci_server_url_carries`
- `falls_back_to_ci_server_host_parts`
- `no_token_means_no_credentials`
- `opt_out_disables_detection`
- `github_needs_the_runner_marker_not_just_a_token`
- `rewrites_both_ssh_spellings`
- `keeps_nested_subgroups`
- `leaves_matching_https_urls_alone`
- `never_touches_another_host`
- `config_args_scope_to_the_ci_host_and_omit_the_token`
- `the_job_token_goes_only_to_the_ci_servers_own_registry`
- `github_owns_ghcr_and_enterprise_owns_its_containers_host`
- `forbidden_hint_points_at_the_allowlist`

### src/commands/clean.rs

- `the_workspace_root_holds_every_declared_checkout`
- `a_repo_holds_the_checkouts_declared_inside_it_but_not_itself`
- `a_leaf_repo_holds_nothing`

### src/commands/forward.rs

- `gitscale_options_are_taken_up_to_the_separator`
- `a_parallel_editor_fails_and_says_so`

### src/commands/hook.rs

- `a_quoted_value_reads_back_as_itself`
- `a_shim_written_before_quoting_still_reads`

### src/config.rs

- `ordinary_urls_still_load`
- `remote_helper_urls_are_refused`
- `urls_containing_a_double_colon_are_left_alone`
- `option_like_values_are_refused`
- `a_registry_is_given_without_a_scheme`
- `hex_revisions_are_names_unless_they_are_full_shas`
- `a_single_group_can_skip_the_layer_tables`
- `layer_groups_keep_their_order`
- `broken_artefact_tables_are_refused`
- `pinned_branches_and_keep_recent_are_read`
- `a_dependency_config_says_which_branches_it_pins`
- `set_revisions_keeps_comments_and_adds_a_missing_revision`
- `directories_must_stay_inside_the_workspace`

### src/exclude.rs

- `a_block_is_replaced_and_everything_else_kept`
- `a_path_is_matched_literally_at_the_top`

### src/git.rs

- `only_the_users_own_writes_keep_their_askpass_helpers` — A run nobody may be watching blanks every askpass helper; a remote write the user's own command makes keeps theirs.
- `picks_fatal_line_over_leading_warning`
- `falls_back_to_last_line_when_no_fatal_marker`
- `git_versions_compare_by_major_and_minor`
- `falls_back_to_unknown_error_when_empty`
- `explains_a_403_from_the_ci_server`
- `leaves_unrelated_failures_unannotated`

### src/gitlab.rs

- `a_removed_parent_takes_the_checkout_with_it`
- `a_sibling_with_a_shared_prefix_is_not_a_parent`
- `suggests_one_exclude_per_top_level_directory`
- `a_directory_already_excluded_is_not_suggested_again`
- `an_exclude_of_one_checkout_still_gets_its_directory`
- `outside_gitlab_there_is_nothing_to_check`
- `a_runner_told_not_to_clean_needs_no_exclude`

### src/lib.rs

- `only_a_person_at_a_terminal_is_watched` — A command run at a terminal is watched, so git keeps the user's askpass helpers; with no terminal, in CI, or from a git hook — which may have the terminal of the git command that fired it — it is not.
- `every_command_the_skill_names_exists`
- `every_doc_the_skill_links_exists`

### src/man.rs

- `every_page_is_written_and_stamped`
- `pages_from_another_version_are_refreshed_and_missing_ones_not_made`
- `a_directory_that_cannot_be_written_is_skipped`

### src/oci_layout.rs

- `gc_keeps_what_held_manifests_need_and_nothing_else`
- `a_forced_republish_moves_the_tag_and_frees_the_old_image`
- `a_damaged_blob_is_dropped_rather_than_served`
- `a_digest_never_becomes_a_path_outside_the_layout`

### src/output.rs

- `colour_follows_the_flag_then_the_environment_then_the_terminal`
- `plain_result_lines_are_the_words_alone`
- `a_plain_header_ends_in_sixteen_dashes`
- `a_terminal_header_fills_the_width`

### src/paths.rs

- `arguments_are_relative_to_the_current_directory`
- `a_path_is_shown_from_the_current_directory`

### src/prefer.rs

- `source_is_the_default_and_removes_a_preference`
- `it_reads_back_what_it_writes`

### src/progress.rs

- `prints_a_shared_hint_once_after_the_failures`

### src/promote.rs

- `the_newest_release_of_the_pins_kind_and_major`
- `a_pre_release_only_for_a_pin_that_is_one`
- `the_merge_order_follows_the_topic_requests`

### src/registry.rs

- `forges_map_to_their_registries`
- `transports_of_one_repo_share_an_image`
- `an_unknown_host_names_the_setting_to_add`
- `configured_hosts_and_prefixes_win`
- `the_ci_server_maps_to_the_registry_it_owns`
- `names_the_registry_cannot_hold_are_refused`
- `only_this_machine_gets_plain_http`
- `challenges_parse_with_commas_in_quotes`
- `the_next_page_is_read_from_the_link_header`
- `digests_are_checked_before_use_as_names`
- `auth_file_keys_reduce_to_the_registry`
- `stored_logins_are_read_from_an_auth_file`

### src/resolution.rs

- `the_highest_request_wins_and_the_root_takes_part`
- `an_undeclared_dependency_is_placed_under_the_hoist_dir`
- `two_majors_are_placed_side_by_side_and_singleton_refuses`
- `an_override_wins_over_what_it_dominates`
- `an_override_must_agree_with_what_it_does_not_dominate`
- `a_branch_is_decided_by_position_not_history`
- `history_on_disk_only_adds_a_warning`
- `a_missing_revision_fails_unless_an_override_above_covers_it`
- `tied_overrides_each_win_over_what_they_are_above` — Two tied overrides, neither above the other: each wins over what its own repository is above.
- `an_implicit_major_never_lands_on_a_root_checkout` — The root declares one major at the plain name; a dependency's other major is placed beside it rather than on it.
- `a_dependency_config_is_read_for_its_repos_only` — Only `[repos]` and `singleton` of a dependency's config are read: what else it holds is its own business, and cannot break the parent.
- `cycles_fail`
- `a_losing_revision_withdraws_its_requests`
- `implicit_dependencies_need_the_allowlist`
- `a_local_topic_branch_beats_every_request_and_keeps_the_pin`
- `a_topic_slots_own_config_is_read_where_it_stands`
- `a_remote_branch_of_the_topics_name_is_followed_unless_the_root_overrides`
- `each_major_has_a_branch_of_its_own`
- `a_requester_that_pins_the_topic_keeps_its_dependencies_at_their_pins` — A requester whose own config pins the topic branch keeps what it asks for at its pins, all the way down; pinned wins over a requester that does not pin it.

### src/resolve.rs

- `test_relative_path_sibling`
- `test_relative_path_shared_prefix`
- `test_relative_path_root_level`
- `test_lexical_join_parent`

### src/skill.rs

- `the_skill_is_an_agent_skill_with_a_header`
- `install_goes_to_the_shared_location_and_claudes_when_present`
- `refresh_updates_an_older_copy_only`
- `remove_takes_only_what_gitscale_wrote`

### src/ssh.rs

- `ignores_failures_other_than_ssh_auth`
- `empty_agent_asks_for_ssh_add`
- `missing_agent_in_an_ssh_session_suggests_forwarding`
- `stale_socket_is_named`
- `loaded_agent_points_at_the_registered_key`
- `finds_the_remote_that_refused`

### src/store.rs

- `transport_forms_of_one_repo_share_a_store`
- `different_repos_never_share_one`
- `a_store_name_is_one_harmless_path_component`
- `an_image_entry_is_named_like_its_store`
- `markers_older_than_a_cutoff_and_missing_ones_count_as_old`

### src/topic.rs

- `a_gitlab_merge_request_pipeline_is_for_its_source_branch`
- `a_tag_pipeline_is_for_no_branch`
- `github_reads_a_pull_request_head_or_a_pushed_branch`
- `no_pipeline_without_its_platform_or_commit`
- `a_prefix_is_added_once_and_dropped_from_the_directory`
- `the_default_branch_is_pinned_unless_the_list_says_otherwise`
- `a_version_branch_belongs_to_its_topic`

### src/trust.rs

- `an_empty_allowlist_denies_everything`
- `patterns_match_at_every_granularity`
- `a_dot_dot_path_does_not_escape_its_owner` — `..` in an scp-style path names another owner's repository; it must not pass for one under the owner it climbs out of.
- `a_comma_separated_list_is_a_union`
- `ssh_and_https_spellings_are_the_same_repository`
- `an_owner_pattern_ends_at_the_slash`
- `a_star_crosses_slashes_so_subgroups_are_covered`
- `a_workspace_without_a_forge_remote_is_named_by_its_path`
- `a_local_mirror_can_be_named_by_its_remote_path`
- `specs_that_would_break_the_shim_are_refused`
- `a_refusal_names_the_repo_and_the_command_that_would_allow_it`
- `sanitize_defangs_escape_sequences`

### src/urls.rs

- `owner_and_repo_survive_every_transport`
- `test_normalize_url`

### src/version.rs

- `semver_with_or_without_v_is_the_same_version`
- `semver_precedence`
- `classes_follow_cargo_and_the_calendar_major`
- `a_calendar_version_needs_its_major`
- `calendar_modifiers_order_around_the_bare_date`
- `semver_and_calendar_versions_never_compare`
- `other_tags_are_not_versions`
