---
name: testing
description: "Run and write gitscale's tests. USE FOR: running the test suite, running one feature's tests, the real-registry conformance tests, adding or renaming a test, regenerating the test catalog. DO NOT USE FOR: releasing (see cargo-release)."
---

# Testing gitscale

The rules — layout, naming, ignored tests, the catalog — are in
`docs/testing.md`. Read it before adding or renaming a test.

## Running

```bash
cargo test                                # everything
cargo test --test it pull::               # one feature
cargo test --test it -- --ignored         # known bugs and slow tests
```

## The real-registry conformance tests

`registry::` has tests that talk to a real `registry:2`, and skip themselves
unless `GITSCALE_TEST_REGISTRY` names one. The container runs from
`docker-compose.yml`. Try `localhost:5000` first; when that is refused (as it
is from inside the agent sandbox or the devbox), use the service name:

```bash
GITSCALE_TEST_REGISTRY=localhost:5000 cargo test --test it registry::
GITSCALE_TEST_REGISTRY=registry:5000 cargo test --test it registry::
```

## After adding, renaming or removing a test

```bash
GITSCALE_UPDATE_CATALOG=1 cargo test --test it catalog::
```

and commit `docs/test-catalog.md` with the change.
