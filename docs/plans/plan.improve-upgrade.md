# Plan: which tags `upgrade` may move a pin to

Status: proposed, not started. Both changes land together: 1 is the default,
2 is opt-in.

- [The problem](#the-problem)
- [1. A new tag must contain the pin](#1-a-new-tag-must-contain-the-pin)
- [2. Opt-in: release branches](#2-opt-in-release-branches)
- [Where the code changes](#where-the-code-changes)
- [Tests](#tests)
- [Docs](#docs)
- [Open questions](#open-questions)

## The problem

Today a candidate tag is chosen by its name alone (`promote::newest`): same
stream and scheme as the pin, same major unless `--major`, pre-release only
from a pre-release, highest wins. Which branch the tag was cut on, and whether
it descends from the pin at all, is never asked.

| Case | Today |
|---|---|
| A calver hotfix tagged later on a maintenance line that split off **before** the pin | `upgrade <dir>` raises to it, losing what the pin had. Promotion takes it as "the newest", fails the content check, and says `not tagged yet` for good — even when a `main` tag already holds the change |
| A hotfix branched **from** the pin, tagged after `main`'s newest | Same as above, except nothing the pin had is lost — only what `main` gained since |

Git records no branch for a tag, so "the branch the pin is on" cannot be
inferred: every branch containing the pin's commit qualifies — `main`, every
feature branch since, and a `release/*` cut at the pin.

## 1. A new tag must contain the pin

Default, no configuration.

- **Rule.** A candidate must have the pin's commit in its history:
  `git tag --contains <pin>` in the slot's store, one call, then the existing
  name filter of `promote::newest` on what it lists.
- **`--major` relaxes it.** A new major is commonly cut on a line that split
  from the old one before the pin (a `v1` maintenance branch next to `v2` on
  `main`). Crossing a major keeps today's name-only choice.
- **Promotion picks the newest tag that holds the change**, among those
  containing the pin — not only the newest tag. Walk the candidates newest
  first; the first whose `merge-tree` gives its own tree wins; `Conflicts` on
  the newest still reports `cannot tell` rather than falling back silently.
- **A pin no tag contains** — its tag was moved, or the store lacks the
  history — reports `no release contains <pin>` instead of `not tagged yet`.

Not solved: a hotfix branched *from* the pin still outranks `main`'s newest on
a raise. That is what 2 is for; separate streams (`hotfix-v…`) already work.

## 2. Opt-in: release branches

A repository names the branches it releases from, once, in its own
`.gitscale.toml`; only tags reachable from one of them are candidates for
every workspace that depends on it. Applies with 1, not instead of it.

```toml
# core's own .gitscale.toml
[release]
branches = ["main", "release/*"]
```

- **Producer-side**, like `[artefact]`: where a repository tags is its own
  knowledge, so consumers don't repeat it.
- **Read from the dependency's default branch** (`origin/HEAD` in its store,
  `git show origin/HEAD:.gitscale.toml`), not from the pinned commit: the
  pin can predate the policy, and choosing a *new* tag should follow the
  policy as it is now. No default branch known: the config at the pin.
- **No `[release]`, or no `.gitscale.toml`:** rule 1 alone.
- **Override, for a repository you don't control** (one with no
  `.gitscale.toml` of its own): `releases` on the consumer's entry wins over
  the dependency's `[release]`.

  ```toml
  [repos]
  "imports/thirdparty" = { url = "https://github.com/other/lib.git", revision = "v1.4.0", releases = ["main"] }
  ```

- **Patterns** as `[develop] pinned` writes them (`trust::wildcard_match`),
  matched against the store's `refs/remotes/origin/*`.
- **Rule.** `git tag --merged origin/<b>` for each matching branch, unioned,
  intersected with 1's list.
- **No match** — no remote branch fits the patterns — is an error naming the
  repository and where the patterns came from, not an empty candidate list
  that reads as "no release".
- **Scope.** Only `upgrade` (promotion and raise) and the promotion state
  `status` shows. Resolution is untouched: it compares revisions already
  written, and picks no tags.
- **`--major`** still applies the branch filter: a declared release branch is
  explicit, unlike 1's ancestry guess.

## Where the code changes

| Where | Change |
|---|---|
| `src/config.rs` | `[release] branches: Vec<String>`; `RepoEntry.releases: Option<Vec<String>>` as the override; both validated as non-empty patterns; `releases` kept by `require` / `unrequire` and `upgrade`'s in-place edits |
| `src/promote.rs`, new `release_branches(store, entry)` | The entry's `releases`, else `[release] branches` from the dependency's default branch, else none |
| `src/promote.rs`, `newest` | Takes the candidate list already filtered; stays name-only |
| `src/promote.rs`, new `candidates(store, pin, releases, cross_major)` | Applies 1 and 2, returns tags newest first |
| `src/promote.rs`, `assess` | Uses `candidates`; walks them for the newest that holds the change; new `State` for "no release contains the pin" |
| `src/commands/upgrade.rs`, `release_tags` | Lists tags from the store instead of `ls-remote`, so ancestry can be asked. Resolution has already fetched the stores (`upgrade.rs` line 151); confirm the raise path runs after that fetch |
| `src/commands/status.rs` | Nothing beyond the new state's wording — it shares `assess` |

The store already fetches `+refs/heads/*:refs/remotes/origin/*` and
`+refs/tags/*:refs/tags/*` (`src/store.rs`), so both rules work offline once
fetched. `upgrade` refuses in CI, so the full history is always there.

## Tests

In the `upgrade` and `status` features, through the playground remotes:

- A calver hotfix tagged on a line split before the pin is not raised to; the
  `main` tag below it is.
- Promotion with that hotfix as the newest tag promotes to the `main` tag that
  holds the change, instead of `not tagged yet`.
- A hotfix branched from the pin is raised to without `[release]`, and not
  when the dependency declares `branches = ["main"]`.
- The dependency's `[release]` is read from its default branch: a pin older
  than the policy still gets it.
- A consumer's `releases` overrides the dependency's `[release]`.
- `--major` crosses to a major on a line that does not contain the pin.
- A moved pin tag reports `no release contains`.
- Patterns naming no remote branch are an error naming the repository and
  where the patterns came from.
- `releases` survives `require` / `unrequire` and `upgrade` editing the file.

Regenerate the test catalog afterwards.

## Docs

- `docs/topics.md`, *Promotion* and *Raising a dependency*: the containment
  rule, the newest-that-holds walk, `--major`'s exception, the new state.
- `docs/configuration.md`: `[release] branches`, and the `releases` override
  on an entry.
- `docs/cli.md`, `gitscale upgrade`: one line on which tags qualify.

## Open questions

- **Should the release branches also bind promotion's `not tagged yet`
  hint** to name the branch the change still has to reach (`not on main
  yet`)?
