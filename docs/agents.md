# 2.11 The agent skill

- [What it is](#what-it-is)
- [Installing it](#installing-it)
- [How it stays current](#how-it-stays-current)
- [The hint](#the-hint)
- [Removing it](#removing-it)

## What it is

A short [Agent Skill](https://agentskills.io) that teaches coding agents to make
a change across several repositories: `git topic start` a topic, `git topic
join` the children the change touches, `git scale commit` and `git scale push`,
merge in the order `git topic status` prints, promote each layer with
`git upgrade --commit`, and `git topic finish`. It carries a table of
the commands with links to these docs at the installed version, and the rules
an agent keeps to: never edit a pin to test a change, never commit in a
detached child, ask before merging or deleting a remote branch.

Agents load a skill only when a task matches its description, so it costs
nothing in a session that never touches a `.gitscale.toml`.

## Installing it

```
git scale skill install
```

| Path | Read by | Written |
|---|---|---|
| `~/.agents/skills/gitscale/SKILL.md` | Agents that follow the shared convention: Codex, Gemini CLI, Cursor, Copilot, OpenCode, Goose, Amp and others | Always |
| `~/.claude/skills/gitscale/SKILL.md` | Claude Code, which reads only its own directory | When `~/.claude` exists |

Both are copies, not symlinks: not every agent follows links.

Only `skill install` creates the skill. A skill is instructions to coding
agents, and GitScale never adds those to other tools' directories unasked —
not even from a `git clone` that fires the [hook](hooks.md#git-hooks).

`git scale skill status` shows each location and what is there.

## How it stays current

The skill's text is built into the binary, so it always matches the version
that wrote it. A line after its frontmatter records that version and a hash of
the text below it:

```
<!-- gitscale-skill 0.7.0 sha256:… -->
```

After any interactive GitScale command, outside CI, an installed copy older
than the binary is rewritten, and the command says so on stderr. Nothing else
is touched:

- **Not installed:** nothing is created.
- **Newer than the binary:** left alone, so an older gitscale never downgrades
  it.
- **Edited by hand** (the hash no longer matches): left alone; `skill status`
  says so, and `skill install --force` replaces it.
- **A file gitscale did not write** at that path: never touched without
  `--force`.

## The hint

While no skill is installed at either path, `git scale sync` and the
`git scale ls` table print one line to stderr:

```
hint: git scale skill install teaches coding agents this workflow
```

It shows once per root: a `skill-hint` file in the root's common git dir
(`.git/gitscale/`, shared by every worktree of the root) records that it was
shown. Never in CI, never with `--format json`, and never when the output is
not a terminal.

## Removing it

```
git scale skill remove
```

Removes the copies gitscale wrote, and the directories it made for them. An
edited copy goes only with `--force`; a file gitscale did not write never does.

---

[← 2.10 Artefacts](artefacts.md) · [Contents](README.md) · [Next → 3. Configuration file reference](configuration.md)
