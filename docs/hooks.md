# 2.7 Hooks

Two different things share the word "hook", and it is worth separating them up
front:

- **[Config hooks](#config-hooks)** — the `[hooks]` table in `.gitscale.toml`:
  commands GitScale runs after its own operations.
- **[Git hooks](#git-hooks)** — installing GitScale *into git*, so that
  [placement](stores.md#placement) runs whenever a checkout or merge changes
  the working tree.

They meet in the [hook allowlist](#the-hook-allowlist), which decides whose
config hooks an installed git hook is allowed to run.

- [Config hooks](#config-hooks)
  - [post_sync](#post_sync)
  - [on_pull_error](#on_pull_error)
- [Git hooks](#git-hooks)
  - [Scopes](#scopes)
  - [Which hooks, and what they cover](#which-hooks-and-what-they-cover)
  - [The hook in a child](#the-hook-in-a-child)
  - [Coexisting with other hooks](#coexisting-with-other-hooks)
  - [Checking an install](#checking-an-install)
  - [Uninstalling](#uninstalling)
  - [When a hook-triggered placement fails](#when-a-hook-triggered-placement-fails)
  - [Git hooks in CI](#git-hooks-in-ci)
  - [Caveats](#caveats)
- [The hook allowlist](#the-hook-allowlist)

## Config hooks

```toml
[hooks]
post_sync = "make install"
on_pull_error = "warn"
```

### post_sync

Runs at the end of every [placement](stores.md#placement), when every step
of it succeeded: `git scale sync`, `git scale pull`, `require` and `unrequire`,
any git command run across the workspace that moved a `HEAD`, and the
[git hook](#git-hooks)'s. It is executed via `sh -c` with the workspace root as
the working directory. A non-zero exit fails the GitScale command.

It does not run after `git scale fetch`, `ls`, `explain`, `clean` or `gc`, nor
after a git command that moved nothing.

When a [git hook](#git-hooks) ran the placement, `post_sync` runs only if the
workspace is on that hook's [allowlist](#the-hook-allowlist); otherwise GitScale
refuses, prints the command it would have run (with control characters escaped),
and explains how to allow it. A placement you started yourself is not
restricted — you chose the directory and the moment.

### on_pull_error

What a placement a **git hook** ran should do when it fails:

| Value | Effect |
|---|---|
| `"fail"` | Return non-zero, failing the git operation that triggered the hook |
| `"warn"` | Report on stderr and let the git operation succeed |

Unset, it defaults to `"fail"` under CI and `"warn"` otherwise — so a broken
placement cannot make unrelated `git checkout` calls look like failures on a
developer machine, while CI still fails fast. It has no effect on a command you
run yourself, which always reports its own exit status.

## Git hooks

```
git scale hook install --local
```

registers GitScale with git so that [placement](stores.md#placement) runs
whenever a checkout or merge changes the working tree. A fresh clone, a `git
switch` onto or off a [topic](topics.md), a `git checkout` that moves the
declared revisions, or a `git worktree add` then puts every checkout where it
goes without anyone remembering to run anything. A hook in the root places
online — see [when resolution asks the remotes](recursive-dependencies.md#when-resolution-asks-the-remotes).
`hook install` also writes GitScale's [man pages](cli.md#man-pages).

This is how a workspace is normally set up: `git clone` and nothing else.

### Scopes

| Scope | What it writes | Use for |
|---|---|---|
| `--local` (default) | a hook file in this repository's hooks directory | one repository; repositories where a global install is shadowed |
| `--global` | `core.hooksPath` in `~/.gitconfig`, pointing at `~/.config/gitscale/hooks` | every repository for the current user |
| `--system` | `core.hooksPath` in `/etc/gitconfig`, pointing at `/etc/gitscale/hooks` | build machines, where every user should get it |

```
git scale hook install --local
git scale hook install --global --allow 'github.com/acme/*'
git scale hook install --system --allow 'github.com/acme/*,git.internal.example/*'
```

`--global` and `--system` require [`--allow`](#the-hook-allowlist), because they
fire on clones of repositories nobody has vetted.

`--local` does **not** survive a fresh clone — git never transfers `.git/hooks`
— so it cannot bootstrap a brand-new checkout. Use `--global` or `--system` for
that. If the repository sets its own `core.hooksPath` (husky, lefthook,
pre-commit), `--local` installs into that directory and says so.

Install refuses to replace an existing `core.hooksPath` it did not set, or to
overwrite an already-displaced hook, without `--force` — doing either silently
would disable somebody's hooks.

### Which hooks, and what they cover

Only `post-checkout` and `post-merge` are installed. Between them:

| Operation | Covered |
|---|---|
| `clone` | yes (`post-checkout`) |
| `checkout` / `switch` | yes |
| `worktree add` | yes (`post-checkout`) |
| `fetch --depth=1` + `checkout FETCH_HEAD` (how CI checks out) | yes |
| `merge`, `git pull` | yes (`post-merge`) |
| `rebase`, `git pull --rebase` | yes (fires `post-checkout` too) |
| `git reset --hard` | **no** — git fires no working-tree hook at all |
| plain `fetch` | not needed; the working tree does not change |

`git scale ls` remains the way to spot drift after a `reset --hard`.

The installed hook acts in two kinds of repository: one with a
`.gitscale.toml` at its own top — a workspace root — and a
[child](#the-hook-in-a-child). That is deliberately not GitScale's usual upward
search: an unrelated repository cloned inside a workspace would otherwise
inherit the parent config and trigger a placement of the whole thing.
Repositories that never opted in stay completely silent.

The hook also does nothing when it is GitScale that invoked git — every git call
GitScale makes is marked, so a placement running `git checkout` cannot recurse
without bound, and a git command run across the workspace with `git scale` is
placed once, afterwards, rather than once per repository.

### The hook in a child

A **child** is a checkout GitScale made: a worktree of one of the root's
[stores](stores.md), whose git common dir is
`<root common dir>/gitscale/repos/<name>.git`. A child is never a workspace of
its own. A `git switch`, `git pull` or `git merge` run inside one fires the
hook there, and the hook places the whole workspace around it.

The installed hook decides, in order:

1. `GITSCALE_HOOK` is set — GitScale itself ran git: exit.
2. A file checkout — `post-checkout` with a third argument of `0`: exit.
3. `git rev-parse --git-common-dir` matches `*/gitscale/repos/*.git`: a child.
   Run `gitscale hook run <hook> --child <child top> -C <child top>`.
4. `.gitscale.toml` at the repository's top: a root. Run `gitscale hook run
   <hook> -C <top>`.
5. Otherwise: exit.

For a child, `hook run`:

- finds the workspace root, as every command does — see
  [finding the workspace](cli.md#finding-the-workspace);
- does nothing while the child is in the middle of a rebase, merge,
  cherry-pick, revert or bisect — `rebase-merge`, `rebase-apply`,
  `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD` or `BISECT_LOG` in its git
  directory;
- places the workspace, fetching only what resolution lacks (online in CI —
  see [when resolution asks the remotes](recursive-dependencies.md#when-resolution-asks-the-remotes)),
  and leaves the child's `HEAD` and files where git put them. The links inside
  it are still planted. Its result line says so:
  `skip  imports/core (left where git put it)`;
- on the slot's **topic branch**, makes the child writable, as
  [`git topic join`](topics.md#git-topic-join--leave) does. Nothing is moved;
- on **any other branch**, leaves it, with a warning:

  ```
  imports/core is on feat/x, not the topic PROJ-12-price-cache; the next placement moves it back to its pin
  ```

- checks the [allowlist](#the-hook-allowlist) against the workspace root's
  remote, not the child's: the root's `post_sync` is what runs.

A child the root declares with [`recursive = false`](recursive-dependencies.md#turning-it-off-recursive--false)
is handled the same way.

So plain git in a child works like `git topic join`:

```
git switch "$(git topic)"        # in imports/core: on its topic branch, writable
```

### Coexisting with other hooks

A global `core.hooksPath` *replaces* `.git/hooks` rather than adding to it, so
the installed hook always runs the repository's own hook first and reports its
exit status. If a hook of the same name already exists, install moves it aside
to `<name>.local` and chains to it; `uninstall` puts it back. Re-installing over
GitScale's own hook keeps whatever it was chaining to.

If the gitscale binary is missing but the repository does have a
`.gitscale.toml`, the hook says so on stderr rather than failing silently.

### Checking an install

```
git scale hook status
```

reports, in order: the `system` and `global` `core.hooksPath` settings; the
repository and the hooks directory git will actually use for it; whether the
repository's own `core.hooksPath` is shadowing a global install; the state of
each hook file (`gitscale`, `other (not gitscale)`, `not installed`); the
allowlist in effect and whether this repository is inside it; and the last
hook-triggered placement failure, if there is one — `Last hook-triggered
placement FAILED:` and what failed.

### Uninstalling

```
git scale hook uninstall --local
git scale hook uninstall --global
git scale hook uninstall --system
```

Removes only GitScale's own hook files, restores any hook that was displaced,
and unsets `core.hooksPath` if it still points at GitScale's directory. Hooks
somebody else installed are left alone.

### When a hook-triggered placement fails

The failure is reported three ways, because none alone is enough:

1. On stderr, which reaches you through `git clone` or `git checkout`.
2. In a breadcrumb file, `gitscale-pull-failed` in the workspace root's git
   directory — `the placement the post-checkout hook ran failed: …` — which
   `git scale hook status` surfaces. It is removed on the next successful
   hook-triggered placement.
3. In the exit status — under `on_pull_error = "fail"` only.

The exit status is the weakest of the three: git collapses any non-zero hook
exit to `1` and attributes it to the *checkout*, which did not fail. The
breadcrumb is what makes a later, more confusing failure traceable.

### Git hooks in CI

Hooks cannot bootstrap a checkout on hosted CI: GitLab fetches sources before
any job script runs, and hosted runners keep no global git config between jobs.
On runners you control, `gitscale hook install --system` works. Everywhere else,
run `gitscale sync` as an explicit step after checkout.

On GitLab, a hook install needs one more line. The runner cleans the build
directory *after* its checkout, so the hook populates the declared checkouts and
the default `GIT_CLEAN_FLAGS` (`-ffdx`) deletes them again: they are ignored,
and `-ff` removes nested repositories too. Exclude them from that clean, in
`.gitlab-ci.yml` or in the runner's environment:

```yaml
variables:
  # One -e per top-level checkout directory; gitignore syntax, anchored at
  # the workspace root.
  GIT_CLEAN_FLAGS: -ffdx -e /imports/
```

A missing exclude fails the checkout. In a GitLab job (`GITLAB_CI=true`), the
hook's placement ends by asking git what the runner's clean is about to do —
`git clean -n` with the job's own `GIT_CLEAN_FLAGS` — and if that would delete a
declared checkout, the hook fails whatever
[`on_pull_error`](#on_pull_error) says, with the line to add:

```
gitscale: post-checkout hook — GIT_CLEAN_FLAGS="-ffdx" would delete imports/core — GitLab Runner cleans after its checkout, so the job would start without it.
Exclude the checkouts from that clean, in .gitlab-ci.yml or the runner's environment:

    GIT_CLEAN_FLAGS: -ffdx -e /imports/
```

The job stops at *Getting source from Git repository* instead of at the first
build step that goes looking for a sub-repository. `GIT_CLEAN_FLAGS: none`
passes, as does anything else that leaves the checkouts where they are.

Excluded, the checkouts stay in a build directory the runner keeps between jobs,
and the next job's placement updates them in place. That is what keeps a build
cache valid: files the new revision does not change keep their mtimes, so a
tool that decides freshness by mtime — `cargo`, for path dependencies — does
not rebuild them. The placement still cleans up after the previous job: under
CI, every checkout it placed then gets [`git scale clean -fdx`](clean.md) — the
same rules, each checkout's own `[clean] exclude` included, and the same
report. The workspace root is not among them: it is the runner's to clean, and
a job that places after restoring a build cache into it must not lose that
cache.

An explicit `gitscale sync` step needs none of the exclude — by the time a job
script runs, the runner's clean is already done.

### Caveats

- A `--global` or `--system` hook makes `git clone` and `git checkout` run
  GitScale against whatever config the branch carries. The
  [allowlist](#the-hook-allowlist) baked into the hook is what keeps that from
  being an invitation.
- A repository that sets its own `core.hooksPath` overrides the global one, so a
  `--global` install does not run there. This is silent; `git scale hook status`
  reports it, and `--local` is the fix.
- `git scale hook run` is invoked by the installed hook, not by you. Run by hand
  it refuses, because the environment the shim provides is missing.
- A file checkout — `git checkout -- <path>`, which git reports to
  `post-checkout` with a third argument of `0` — moves no revision, and the
  shim ignores it. Shims written by an earlier GitScale place on it too, and do
  not place the workspace from a [child](#the-hook-in-a-child);
  `git scale hook install` rewrites them.

## The hook allowlist

`.gitscale.toml` lives *inside* the repository, so its `[hooks]` table is written
by whoever wrote the branch — including someone who has only opened a merge
request. A `--global` or `--system` git hook turns every `git clone` and
`git checkout` on the machine into a trigger for it: cloning a branch to review
it would run their command with your shell, your SSH keys and your tokens.

So `git scale hook install` takes an allowlist and bakes it into the hook it
writes:

```
git scale hook install --global --allow 'github.com/acme/*,git.internal.example/*'
```

Comma-separated glob patterns, where `*` stands for any run of characters and
`?` for exactly one. They are matched case-insensitively against the
`host/owner/repo` of the workspace root's `origin` — also for a hook that fired
in a [child](#the-hook-in-a-child) — so the SSH and HTTPS spellings of one
repository are the same pattern:

| Pattern | Allows |
|---|---|
| `github.com/acme/gitscale` | that one repository |
| `github.com/acme/*` | every repository under that owner, subgroups included |
| `github.com/*` | every repository on that host |
| `*/acme/*` | that owner on any host |
| `*` | everything |
| `/srv/workspaces/*` | matched against the workspace path, for a workspace with no remote |

`*` deliberately crosses `/`, so `gitlab.com/acme/*` covers a nested subgroup.
The flip side is that a pattern must be ended deliberately:
`github.com/acme/*` does not match `github.com/acme-evil/x`, but
`github.com/acme*` does.

**There is no config file.** The patterns live in the hook script itself, in
`~/.config/gitscale/hooks/` or `/etc/gitscale/hooks/`, and the script passes
them to GitScale in `GITSCALE_HOOK_ALLOW`. That is what makes them trustworthy:
no branch can reach that file, so a repository cannot vouch for itself. Quotes
and control characters are refused in a pattern, since they would produce a
broken hook script.

Consequences worth knowing:

- **`--allow` is required for `--global` and `--system`.** There is no default,
  because guessing one is the bug this exists to prevent.
- **Re-installing without `--allow` keeps whatever the previous hook allowed**,
  so upgrading does not silently widen or narrow anything.
- **`--local` allows its own repository** without being asked. Installing into
  one repository's `.git/hooks` is already a decision about that repository, and
  git never transfers `.git/hooks`.
- **A hook installed by a GitScale older than 0.3 passes no allowlist**, and
  GitScale refuses to run rather than assuming the old behaviour. Re-run
  `git scale hook install` to choose.
- **A placement you start yourself is not restricted.** The allowlist belongs
  to the hook that fires without being asked. If you clone an untrusted branch
  and then run `git scale sync` in it by hand, its `post_sync` will run — what still protects you there is the set of
  [values GitScale refuses to pass to git](configuration.md#values-gitscale-refuses-to-pass-to-git).

`git scale hook status` prints the patterns in effect and whether the current
repository is inside them. A refusal prints them too, along with the exact
command to re-install with this repository added.

---

[← 2.6 Topics](topics.md) · [Contents](README.md) · [Next → 2.8 CI authentication](ci-authentication.md)
