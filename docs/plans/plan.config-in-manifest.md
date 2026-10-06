# Plan: an artefact's config in its manifest

Status: proposed, not started.

- [What it gives](#what-it-gives)
- [Publishing](#publishing)
- [Reading](#reading)
- [The placed artefact](#the-placed-artefact)
- [Code](#code)
- [Tests](#tests)
- [Docs](#docs)

## What it gives

An artefact's image holds one layer per group, and nothing else. The
repository's `.gitscale.toml` travels in the image's manifest, as the
annotation `dev.gitscale.config`, beside `dev.gitscale.hash` and
`dev.gitscale.tree`.

- **One group, one layer.** Such an image is what Argo CD takes as an OCI
  source: a single `application/vnd.oci.image.layer.v1.tar+gzip` layer. A root
  publishing its rendered manifests as its artefact deploys them by release
  tag or by source hash.
- **A dependency's config costs no download.** Resolution already fetches the
  manifest of the image it reads, and keeps it in the image store.
- **The placed artefact holds `.gitscale.toml`**, at its top, as a source
  checkout does.

## Publishing

`artefact publish` reads the repository's `.gitscale.toml` and puts its bytes,
unchanged, in the manifest:

```json
"annotations": {
  "org.opencontainers.image.revision": "4f2a9c1…",
  "org.opencontainers.image.source": "https://github.com/org/app",
  "dev.gitscale.tree": "8be104d2…",
  "dev.gitscale.hash": "3c9f2a71…",
  "dev.gitscale.config": "[[artefact.layer]]\nname = \"app\"\n…"
}
```

- No group ships `.gitscale.toml`: a pattern that matches it leaves it out, as
  it does today. The name `gitscale` is no longer reserved for a group.
- The config stays exempt from the [artefact policy](../artefacts.md#the-artefact-policy):
  it is what the consumer resolves with, whatever state it is in.
- The manifest stays reproducible: the same config, layers and commit give
  the same digest.
- Output:

  ```
  Publishing ghcr.io/org/app/gitscale:3c9f2a71…
    tags: source hash 3c9f2a7144e0, v1-2026.10.06-153012
    config: .gitscale.toml, 312 B
    layer vendor: 412 files, 3.1 MiB, sha256:4c1f…
    layer app: 12 files, 84.0 KiB, sha256:a90e…
  ```

## Reading

Wherever a checkout's dependencies come from its image — a checkout taken as
an artefact, a repository whose sources cannot be read — the config is the
manifest's `dev.gitscale.config`. Offline, it comes from the manifest kept in
the image store.

An image without the annotation is not taken:
`ghcr.io/org/app/gitscale:v2.1.0 carries no GitScale config; publish it again`.

## The placed artefact

Placement unpacks the layers, then writes `.gitscale.toml` from the manifest
at the top of the checkout, read-only like every file it placed. It is the
file of the commit the image names, byte for byte, so the checkout reads like
a source checkout: its dependencies are linked inside it, and tools reading
its config find it where they would.

`artefact show` lists the config before the layers:

```
  layers     config  .gitscale.toml  312 B
             vendor  3.1 MiB  sha256:4c1f…
             app     84.0 KiB  sha256:a90e…
```

## Code

- `src/artefact.rs`: `pack_config` and `CONFIG_LAYER` go; the manifest takes
  the config annotation; reading a config takes it from the manifest, with no
  blob; `install` writes `.gitscale.toml` after the layers and before making
  the checkout read-only; publish's policy check no longer filters a config
  layer; the publish and dry-run output print the config line.
- `src/config.rs`: the reserved layer name goes.
- `src/commands/artefact.rs`: `show` prints the config row.

## Tests

- A published manifest carries `dev.gitscale.config`, equal to the
  repository's `.gitscale.toml`; its layers are the groups, in order, and no
  other.
- One group publishes an image with one layer (`registry`, through
  `skopeo inspect`, which counts three layers today).
- A checkout taken as an artefact resolves its dependencies from the manifest,
  online and offline.
- The placed artefact holds `.gitscale.toml`, identical to the commit's and
  read-only; its dependencies are linked inside it.
- A pattern matching `.gitscale.toml` does not ship it; a group named
  `gitscale` publishes.
- An image without the annotation fails with the message above.
- The same commit published twice gives the same manifest digest.

Snapshots of publish, dry-run and `artefact show` output change.

## Docs

- `artefacts.md`: *How it works*, *What gets published* (the paragraph on the
  config), *Groups and layers*, *The artefact policy*, the publish and dry-run
  examples, *artefact show*, *No access to the sources* (where its
  dependencies come from), *The checkout*.
- `recursive-dependencies.md`, *When resolution asks the remotes*: the config
  from each image's manifest.
- `status.md`: what `ls --fetch` records for artefacts — the manifests, with
  the config in them.
