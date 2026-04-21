---
name: cargo-release
description: "Publish gitscale to crates.io. USE FOR: releasing new versions, bumping version numbers, publishing gitscale to crates.io, tagging releases. DO NOT USE FOR: building locally, running tests, CI/CD pipeline setup."
---

# Cargo Release — Publishing to crates.io

## Crate

| Crate | Directory | crates.io name | Role |
|-------|-----------|----------------|------|
| gitscale | `.` | `gitscale` | CLI + library |

Single crate — no publish ordering needed.

## Prerequisites

- `cargo login` must have been run with a valid crates.io API token
- All tests passing (`cargo test`)

## Procedure

### 1. Ask the user for release type

**MANDATORY**: Before doing anything, ask the user whether this is a **patch**, **minor**, or **major** release. Do not assume or proceed without an explicit answer.

- **patch** (X.Y.Z → X.Y.Z+1): Bug fixes, no API changes
- **minor** (X.Y.Z → X.Y+1.0): New features, backward compatible
- **major** (X.Y.Z → X+1.0.0): Breaking changes

Read the current version from `Cargo.toml` and present the three options with the resulting version number so the user can confirm.

### 2. Bump version

Update the version in `Cargo.toml`:

```toml
version = "X.Y.Z"
```

### 3. Run tests

```bash
cargo test
```

All integration tests must pass before proceeding. If tests fail, fix the issues and restart from this step.

### 4. Pre-publish checks

```bash
# Ensure everything compiles
cargo check

# Dry-run package to catch issues before publishing
cargo package --allow-dirty
```

Review any warnings. Fix before proceeding.

### 5. Commit the version bump

Commit all changes **before** publishing so `cargo publish` works without `--allow-dirty`:

```bash
git add -A
git commit -m "release: vX.Y.Z"
```

### 6. Publish

```bash
cargo publish
```

### 7. Tag and push

```bash
git tag vX.Y.Z
git push && git push --tags
```

## Quick Reference (copy-paste)

Replace `X.Y.Z` with the actual version:

```bash
# 1. Run tests
cargo test

# 2. Verify
cargo check

# 3. Dry-run
cargo package --allow-dirty

# 4. Commit
git add -A && git commit -m "release: vX.Y.Z"

# 5. Publish
cargo publish

# 6. Tag & push
git tag vX.Y.Z
git push && git push --tags
```

## Troubleshooting

- **"crate version X.Y.Z is already uploaded"**: The version already exists on crates.io. Bump to a new version.
- **"not logged in"**: Run `cargo login` with your crates.io API token.
- **Packaging warnings about missing files**: Ensure `README.md` exists at the repo root.
