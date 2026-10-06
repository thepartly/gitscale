# Plan: the local stack with the host's Nix toolchain

Status: proposed, not started. Needs the demo's
[local stack](plan.demo.md#local-stack).

- [What it gives](#what-it-gives)
- [`--toolchain nix`](#--toolchain-nix)
- [The flakes](#the-flakes)
- [Working on the host](#working-on-the-host)
- [Checked so far](#checked-so-far)
- [Steps](#steps)

## What it gives

On a NixOS host, a service run from its working tree uses the host's own
tools: the store paths of its repository's dev shell. No toolchain image is
built or pulled. A build in the stack and one in the developer's shell use the
same compiler, so they share `target/`, and rust-analyzer's build is the
stack's. Each repository pins its tools in its own `flake.lock`.

## `--toolchain nix`

`compose --toolchain nix`, or `GITSCALE_DEMO_TOOLCHAIN=nix` for a standing
choice; the default stays `image`. For a service run from its working tree,
the override changes as below. The user, working directory, the workspace and
`$HOME` mounts, and `/etc/passwd` and `/etc/group` stay as with the image.

```yaml
services:
  application-a:
    image: busybox
    entrypoint: []
    command: [nix, develop, /home/me/projects/gitscale-demo/imports/application-a,
              --command, bash, -c, "cargo watch -w src -w imports/shared-libs -x run"]
    environment:
      PATH: /run/current-system/sw/bin
      NIX_CONFIG: experimental-features = nix-command flakes
      SSL_CERT_FILE: /etc/static/ssl/certs/ca-certificates.crt
    volumes:
      - /nix:/nix:ro
      - /run/current-system:/run/current-system:ro
      - /etc/static:/etc/static:ro
```

- `/nix` holds the store and the daemon's socket: `nix develop` asks the
  host's daemon to fetch or build the dev shell, into the host's store.
- `busybox` supplies an empty root filesystem; nothing runs from it.
- `CARGO_TARGET_DIR` is left unset, so `target/` is the one the host uses.
- `nix develop` evaluates the flake on every start; its evaluation cache is in
  the mounted `$HOME`.
- `shell` gets the same, with the current repository's dev shell when it has
  a flake, and the host's system tools otherwise.

## The flakes

Each service repository carries `flake.nix` and `flake.lock`, with a default
dev shell holding what its `x-build.run` needs:

| Repository | Dev shell |
|---|---|
| application-a, application-b | rustc, cargo, cargo-watch |
| frontend | Node, npm |

The flake builds nothing; it only provides tools. The checkouts under
`imports/`, which git does not track and a flake therefore does not see, are
read by cargo and npm, not by Nix. Images stay as CI builds them.

## Working on the host

The developer's shell runs on the host, outside the stack's network. Services
answer at their public ports on localhost, and by name through
`compose run --rm shell`. Nothing in the stack connects to the host, so the
host's firewall needs no change.

The host needs Docker (`virtualisation.docker.enable`, the user in the
`docker` group), `jq` and GitScale.

## Checked so far

On a NixOS 26.05 host, through its Docker daemon, with `busybox`, the mounts
above and the user's own uid:

- the host's tools run: bash, git, curl;
- a container on the same network resolves by name;
- HTTPS works once `SSL_CERT_FILE` is set;
- `nix` needs the user's `/etc/passwd` entry. With it, it reaches the host's
  daemon through the read-only `/nix`, and fetches into the host's store:
  `nix shell nixpkgs#cargo` ran cargo 1.98.

## Steps

1. Flakes in application-a, application-b and frontend; `nix develop` then
   the `x-build.run` command works on the host.
2. `--toolchain nix` and `GITSCALE_DEMO_TOOLCHAIN` in `compose`.
3. The demo's step 9 with `--toolchain nix`: an edit to shared-libs reloads
   application-a, and a build on the host afterwards compiles nothing.
4. npm packages that ship their own executables (Vite's esbuild and rollup)
   run under the dev shell, in the container and on the host.
5. `DEMO.md`: the variant, for a NixOS host.
