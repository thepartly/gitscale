# 2.7 Hooks

Two different things share the word "hook", and it is worth separating them up
front:

- **[Config hooks](#config-hooks)** — the `[hooks]` table in `.gitscale.toml`:
  commands GitScale runs after its own operations.
- **[Git hooks](#git-hooks)** — installing GitScale *into git*, so that
  [placement](stores.md#placement) runs whenever a checkout or merge changes
  the working tree, and every other hook a repository relies on — its own,
  git-lfs's, and the ones it commits in `.githooks/` — keeps running.

They meet in the [hook allowlist](#the-hook-allowlist), which decides whose
config hooks and committed hooks an installed git hook is allowed to run.

- [Config hooks](#config-hooks)
  - [post_sync](#post_sync)
  - [on_pull_error](#on_pull_error)
- [Git hooks](#git-hooks)
  - [Scopes](#scopes)
  - [Which hooks, and what they cover](#which-hooks-and-what-they-cover)
  - [What a hook runs](#what-a-hook-runs)
  - [The hook in a child](#the-hook-in-a-child)
  - [The repository's own hooks](#the-repositorys-own-hooks)
  - [Git LFS](#git-lfs)
  - [Committed hooks: .githooks/](#committed-hooks-githooks)
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
whenever a checkout or merge changes the working tree, alongside the
repository's other hooks. A fresh clone, a `git
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

A global or system install points `core.hooksPath` at GitScale's directory,
and git then looks for every hook there and nowhere else — not in a
repository's `.git/hooks`. Only the hooks installed there run, in any
repository: choose them with [`--hooks`](#which-hooks-and-what-they-cover).

`--local` does **not** survive a fresh clone — git never transfers `.git/hooks`
— so it cannot bootstrap a brand-new checkout. Use `--global` or `--system` for
that. If the repository sets its own `core.hooksPath` (husky, lefthook,
pre-commit), `--local` installs into that directory and says so.

Install refuses to replace an existing `core.hooksPath` it did not set, or to
overwrite an already-displaced hook, without `--force` — doing either silently
would disable somebody's hooks.

### Which hooks, and what they cover

Every install writes `post-checkout` and `post-merge`, which place the
workspace. `--hooks` adds more:

```
git scale hook install --global --allow 'github.com/acme/*' --hooks pre-commit,lfs
```

It takes comma-separated hook names, `lfs` for git-lfs's four (`pre-push`,
`post-checkout`, `post-commit`, `post-merge`), and `all`. The names are every
client-side hook git runs: `applypatch-msg`, `pre-applypatch`,
`post-applypatch`, `pre-commit`, `pre-merge-commit`, `prepare-commit-msg`,
`commit-msg`, `post-commit`, `pre-rebase`, `post-checkout`, `post-merge`,
`pre-push`, `post-rewrite`, `pre-auto-gc`, `sendemail-validate`, and the four
`p4-*` hooks of `git p4`. Not `reference-transaction` or `post-index-change`,
which fire on every ref update and index write, where a program started for
each would be felt, and not the server-side hooks.

Re-installing without `--hooks` keeps the hooks installed before. Naming fewer
takes the rest out, putting back any hook one of them had displaced.

Under a `--global` or `--system` install, a hook that is not installed does not
run in any repository — not from its own `.git/hooks` either. The install says
so, and warns when git-lfs is installed but `lfs` is not chosen:

```
  no other git hook runs in any repository, its own .git/hooks included; add them with --hooks
  git-lfs is installed, but its hooks are not: add --hooks lfs, or a push leaves the large files behind
```

Each hook is a short script that hands over to `git scale hook run`, so what a
hook does is the installed GitScale's to decide, and upgrading GitScale changes
it without reinstalling. On a hook with nothing to do, that costs about a
millisecond.

`post-checkout` and `post-merge` between them cover:

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

The placement happens in two kinds of repository: one with a
`.gitscale.toml` at its own top — a workspace root — and a
[child](#the-hook-in-a-child). That is deliberately not GitScale's usual upward
search: an unrelated repository cloned inside a workspace would otherwise
inherit the parent config and trigger a placement of the whole thing.
Repositories that never opted in see only their own hooks run.

There is no placement when it is GitScale that invoked git — every git call
GitScale makes is marked with `GITSCALE_HOOK`, so a placement running `git
checkout` cannot recurse without bound, and a git command run across the
workspace with `git scale` is placed once, afterwards, rather than once per
repository. The repository's own hooks, git-lfs and `.githooks/` still run:
`git scale push` runs each repository's `pre-push`.

### What a hook runs

`git scale hook run` runs these in order, each with git's arguments and
environment, from where git started the hook:

1. **The repository's own hook** — the one git would have run without
   GitScale: see [the repository's own hooks](#the-repositorys-own-hooks).
2. **git-lfs**, for `pre-push`, `post-checkout`, `post-commit` and
   `post-merge`, in a repository that uses it: see [Git LFS](#git-lfs).
3. **`.githooks/<name>`** at the top of the worktree, when the repository is on
   the [allowlist](#the-hook-allowlist): see
   [committed hooks](#committed-hooks-githooks).
4. **The placement**, for `post-checkout` and `post-merge`, as described above.
   Not for a file checkout — `git checkout -- <path>`, which git reports to
   `post-checkout` with a third argument of `0` — which moves no revision: a
   build restoring one file must not have its sub-repositories reset and cleaned
   underneath it.

A hook that can stop what git is doing — every one whose name does not start
with `post-` — stops at the first stage that fails, and exits with its status.
The `post-` hooks run every stage and exit with the first failure's status.

`pre-push` and `post-rewrite` read a list of refs on stdin; each stage gets the
whole of it.

A hook file that itself runs `git scale hook run` — a displaced GitScale hook,
or a repository's copy of one — is skipped wherever it is found, so nothing
runs twice.

### The hook in a child

A **child** is a checkout GitScale made: a worktree of one of the root's
[stores](stores.md), whose git common dir is
`<root common dir>/gitscale/repos/<name>.git`. A child is never a workspace of
its own. A `git switch`, `git pull` or `git merge` run inside one fires the
hook there, and the hook places the whole workspace around it.

`hook run` tells a child by its git common dir, which ends in
`gitscale/repos/<name>.git`; anything else is a root if it has a
`.gitscale.toml` at its top. A child's own `.gitscale.toml` does not make it a
root. For a child, the placement:

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

### The repository's own hooks

The first stage runs:

- `<name>.local` beside GitScale's hook: whatever install found in its way. If
  a hook of the same name already exists where it installs, install moves it
  aside to `<name>.local`; `uninstall` puts it back.
- For a `--global` or `--system` install, the repository's `.git/hooks/<name>`
  (the common git directory's, shared by every worktree) — the hook git would
  have run had `core.hooksPath` not pointed elsewhere.

As git does, it runs only an executable file, and runs one without a `#!` line
with `sh`.

If the gitscale binary is missing, the hook says so on stderr on every hook,
in every repository, and runs the repository's own hook itself, without
git-lfs, `.githooks/` or a placement.

### Git LFS

git-lfs installs `pre-push`, `post-checkout`, `post-commit` and `post-merge`,
each of which runs `git lfs <name>`. `pre-push` is the one that matters: it
uploads the large files a push refers to, and without it a push sends only
pointers to files the server never receives.

git-lfs writes its hooks wherever `core.hooksPath` points, and refuses to
replace GitScale's there. So with `--hooks lfs` GitScale runs git-lfs itself,
as the second stage, in a repository that uses it — one with a `lfs` directory in its common
git directory, which git-lfs creates the first time it stores a file there, or
with git-lfs's own hook among the repository's. A hook git-lfs wrote is
recognised and not run a second time.

On a machine with GitScale's hooks installed, install them with `--hooks
lfs`, and set up git-lfs without its hooks:

```
git lfs install --skip-repo
```

Plain `git lfs install` sets up the same filters, then fails on the hooks it
cannot write. If the repository uses Git LFS and git-lfs is not installed,
the hook fails with exit status 2 and says so, as git-lfs's own hook does.

### Committed hooks: .githooks/

A repository can commit hooks for everyone who clones it in `.githooks/` at its
top, under git's hook names: `.githooks/pre-commit`, `.githooks/commit-msg`.
For the hooks installed with [`--hooks`](#which-hooks-and-what-they-cover),
they run as the third stage, with nothing to set up per clone, when the
repository is on the [allowlist](#the-hook-allowlist) — matched against the
repository's own `origin`, including in a [child](#the-hook-in-a-child). A file
must be executable, which git records when it is committed.

In a repository the allowlist does not name, the hook says so and carries on:

```
gitscale: .githooks/pre-commit not run — github.com/someone/else is not on this machine's hook allowlist; see `gitscale hook status`
```

A repository that sets its own `core.hooksPath` — to `.githooks`, say — keeps
git's behaviour: git runs that directory, and GitScale's hooks do not run
there at all.

### Checking an install

```
git scale hook status
```

reports, in order: the `system` and `global` `core.hooksPath` settings; the
repository and the hooks directory git will actually use for it; whether the
repository's own `core.hooksPath` is shadowing a global install; the hook files,
grouped by state — `gitscale`, `gitscale, written by an older version`,
`gitscale (repository's own copy)`, `other (not gitscale)`, `not installed`;
the allowlist in effect and whether this repository is inside it; and the last
hook-triggered placement failure, if there is one — `Last hook-triggered
placement FAILED:` and what failed.

```
hooks    /home/me/.config/gitscale/hooks
  not installed                          applypatch-msg, pre-applypatch, …
  gitscale                               pre-commit, post-commit, post-checkout, post-merge, pre-push

Hooks not installed here do not run in this repository at all, not even from its own
.git/hooks. Add them with `gitscale hook install --hooks ...`.
```

`written by an older version` means hooks an earlier GitScale installed, which
fire only for `post-checkout` and `post-merge` and run neither the
repository's other hooks nor git-lfs. Re-run `git scale hook install` with the
scope they were installed at.

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

## The hook allowlist

`.gitscale.toml` and `.githooks/` live *inside* the repository, so its
`[hooks]` table and its committed hooks are written by whoever wrote the branch
— including someone who has only opened a merge request. A `--global` or
`--system` git hook turns every `git clone` and `git checkout` on the machine
into a trigger for them: cloning a branch to review it would run their command
with your shell, your SSH keys and your tokens.

So `git scale hook install` takes an allowlist and bakes it into the hook it
writes:

```
git scale hook install --global --allow 'github.com/acme/*,git.internal.example/*'
```

Comma-separated glob patterns, where `*` stands for any run of characters and
`?` for exactly one. They are matched case-insensitively against a
`host/owner/repo` — for `post_sync`, the workspace root's `origin`, also for a
hook that fired in a [child](#the-hook-in-a-child); for `.githooks/`, the
`origin` of the repository the hook fired in — so the SSH and HTTPS spellings
of one repository are the same pattern:

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
