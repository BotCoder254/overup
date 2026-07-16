# Fix: jobs fail with an empty `/workspace` — the stale-runner-image runbook

This is the verified runbook for a family of job failures that all trace back to one
root cause: **the runner binary executing your jobs is older than the code in the
repository.** It also covers the two look-alike failures that were diagnosed alongside
it (a containerd image-pull error and a missing-toolchain error), because they show up
in the same job logs and are easy to blame on the wrong component.

The failures this runbook resolves, as they appear in a pipeline's job log:

```
▶ step 1/4: Install dependencies
npm error code ENOENT
npm error path /workspace/package.json
npm error enoent Could not read package.json: Error: ENOENT: no such file or directory
step failed (exit code 254)
```

```
image pull failed: Docker stream error: failed to extract layer (…) to overlayfs …
failed to Lchown "…/libdjvulibre.a" for UID 0, GID 0: … no such file or directory
```

```
▶ step 1/2: Build
sh: 1: cargo: not found
step failed (exit code 127)
```

They look related — same pipeline, same image, back-to-back runs — but they are three
independent problems with three independent fixes:

| Symptom | Actual cause | Section |
| --- | --- | --- |
| `ENOENT /workspace/package.json` (workspace empty at step time) | The deployed runner image contains the **old, pre-archive-API binary** | §1–§5 |
| `failed to extract layer … failed to Lchown … no such file or directory` | **Docker-host** containerd snapshotter state (disk pressure / partial-pull leftovers), not Overup | §6 |
| `cargo: not found` (or any missing toolchain) | The **job image** doesn't ship that toolchain — `catthehacker/ubuntu:act-latest` has no Rust | §7 |

## 0. The 10-second diagnostic: read the log ORDER, not the log text

The old and new runner binaries produce almost identical log lines — but in a
**different order**. This ordering is the single reliable fingerprint for which binary
actually ran your job, and it cannot lie:

| Binary | Log order |
| --- | --- |
| **OLD (broken)** | `downloading repository archive` → **`checkout complete`** → `pulling image …` → `container started (…)` |
| **NEW (fixed)** | `downloading repository archive` → `pulling image …` → `container started (…)` → **`checkout complete (N files)`** |

Two tells, either is conclusive:

1. **Position** — the old binary "checks out" to a runner-local temp dir *before* the
   container exists (it then bind-mounts that path, which resolves against the wrong
   filesystem when the runner itself runs in a container — hence the empty
   `/workspace`). The new binary streams the source *into the running container* over
   the Docker archive API, so its checkout necessarily happens after
   `container started`.
2. **The file count** — the new binary always prints `checkout complete (N files)`.
   A bare `checkout complete` with no count is the old binary, full stop.

If a job shows the OLD ordering, do not debug the workflow, the YAML, npm, or the job
image — nothing you change there can work. The runner is stale; continue below.

## 1. Why this happens: the publish gap

The fix for the empty-workspace bug (streaming source over the Docker archive API
instead of bind-mounting) lives in `runner/src/executor.rs` — **in the repository**.
But hosted runners execute whatever is inside the image named by `RUNNER_IMAGE`
(default `ghcr.io/botcoder254/overup-runner:latest`), and historically **nothing
published that image automatically**. Merging a runner fix changed zero bytes of the
image the provisioner runs. Three distinct gaps kept the old binary alive; every one
of them must be closed:

1. **The fix wasn't on the deployed branch.** Runner commits merged to `main` only
   help if the image is built from an up-to-date `main` checkout.
2. **The image was never rebuilt/republished** — or worse, was rebuilt from a stale
   clone that didn't contain the fix (a `git clone` from weeks ago, never pulled).
   The image then has a *fresh timestamp* and an *old binary* — the most deceptive
   failure mode in this whole runbook, because everything "looks updated".
3. **Existing runner containers keep their image.** `docker restart` (and the
   janitor's auto-restart of stopped runners) reuses the image the container was
   *created* from. Pulling a new `:latest` changes nothing for a running runner —
   only **revoking and re-creating** the hosted runner produces a container on the
   new image.

A CI job now exists (`.github/workflows/ci.yml`, job `runner-image`) that builds
`runner/Dockerfile` and pushes `:latest` + `:{sha}` to GHCR on every push to `main` —
that is the durable fix for gap 2. Note it only helps when GitHub Actions actually
runs: on a **private** repository, an exhausted spending limit or a failed payment
makes every workflow die instantly as `startup_failure` (even Dependabot's), and the
publish silently never happens. Check github.com → Settings → Billing if runs show
`startup_failure` at 0s. Until Actions is healthy, publish manually (§2–§3).

## 2. Rebuild from the RIGHT source

On whatever machine builds the image (the VPS is fine — see
[publish-runner-image.md](./publish-runner-image.md) for the full build/publish
reference):

```bash
cd <repo-root>
git checkout main
git fetch origin
git pull --ff-only origin main

# CRITICAL — prove the fix commits are in the tree you are about to build:
git log --oneline -5
# must include the runner-fix commits (e.g. "fix(runner): verify checkout delivers
# files, tolerate transient pull failures, publish image from CI"). If the log shows
# only old commits, STOP — you are about to rebuild the broken image with a fresh
# timestamp.

# Build context MUST be the repo root (runner/ needs the sibling protocol/ crate):
docker build -f runner/Dockerfile -t ghcr.io/<your-gh-username>/overup-runner:latest .
```

## 3. Verify the binary INSIDE the image before shipping it

Never trust the tag or the build timestamp — verify the artifact. The fixed binary
contains log strings the old one does not, so one `grep` inside the image is a
conclusive test:

```bash
docker run --rm --entrypoint sh ghcr.io/<your-gh-username>/overup-runner:latest \
  -c 'grep -c "checkout complete (" /app/runner'
```

- **Prints `1` (or more)** → the image contains the fixed binary. Ship it.
- **Prints `0`** → the source tree was stale (§2). Do **not** push; fix the checkout
  and rebuild.

Other new-binary marker strings, if you want a stronger check:
`" files)"`, `"using locally cached image"`, `"contained no usable files"`.

Then push (PAT with `write:packages`; package must be **public** — see
[publish-runner-image.md](./publish-runner-image.md) §3):

```bash
docker login ghcr.io -u <your-gh-username>     # paste the PAT
docker push ghcr.io/<your-gh-username>/overup-runner:latest
```

### Verifying what a REGISTRY holds, without Docker

You can prove what is actually published on GHCR from any machine with `curl` — this
is how the stale image was caught in the wild (it had a post-merge timestamp and a
pre-merge binary):

```bash
# 1. Anonymous pull token (works because the package is public)
TOKEN=$(curl -s "https://ghcr.io/token?service=ghcr.io&scope=repository:<user>/overup-runner:pull" \
  | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')

# 2. Resolve :latest → per-arch manifest → list layers (the last one is the binary COPY layer)
curl -s -H "Authorization: Bearer $TOKEN" \
  -H "Accept: application/vnd.oci.image.index.v1+json" \
  https://ghcr.io/v2/<user>/overup-runner/manifests/latest

# 3. Download the last layer blob, gunzip it, and grep the tar for the marker string
curl -s -L -H "Authorization: Bearer $TOKEN" \
  https://ghcr.io/v2/<user>/overup-runner/blobs/<last-layer-digest> | gunzip -c \
  | grep -c "checkout complete ("        # 1+ = fixed binary is published
```

## 4. Roll it out — a restart is NOT enough

On the VPS (skip the pull if you built directly on the server's daemon):

```bash
docker pull ghcr.io/<your-gh-username>/overup-runner:latest
```

Then, in the Overup UI: **Runners page → revoke each hosted runner → create new
ones** (same labels, e.g. the defaults `self-hosted,linux,x64,ubuntu-latest`). The
provisioner pulls the registry before every provision (falling back to the local
image only if the pull is denied), so newly created runners get the new image.

Why revoke + recreate: a Docker container is bound to the image ID it was created
from. `docker restart`, a backend redeploy, and the janitor's stopped-container
restarts all keep the **old** image. Only container re-creation rebinds `:latest`.

Sanity check on the VPS that the running runner really moved:

```bash
docker images --format '{{.ID}}' ghcr.io/<your-gh-username>/overup-runner:latest
docker ps --format '{{.Names}}\t{{.Image}}'   # find the overup-runner-… container
docker inspect <runner-container> --format '{{.Image}}'   # must match the image ID above
```

## 5. Verify end to end

Dispatch any pipeline and read the job log. Success looks exactly like this, in this
order:

```
downloading repository archive
pulling image catthehacker/ubuntu:act-latest
container started (ab12cd34ef56)
checkout complete (137 files)
▶ step 1/4: Install dependencies
```

…and the step finds its files (`/workspace/package.json` exists). If the log still
shows `checkout complete` (no count) before `pulling image`, one of §2–§4 was skipped
or done against the wrong machine/daemon — walk them again in order.

The hardened runner also closes the *silent* variant of this failure permanently:

- The repackaged source tar is **counted** — a tarball that filters down to zero
  files fails the job (`checkout_failed`, log: `repository archive contained no
  usable files`) instead of uploading an empty archive and logging success.
- Tarballs whose entries carry a leading `./` are handled (the wrapper directory is
  stripped, not the no-op dot), so files can't land at `/workspace/<repo-sha>/…`.
- `checkout complete (N files)` makes delivery observable in every job log.

## 6. The containerd `Lchown … no such file or directory` pull error

```
image pull failed: Docker stream error: failed to extract layer
(application/vnd.docker.image.rootfs.diff.tar.gzip sha256:…) to overlayfs as "extract-…":
failed to Lchown "/var/lib/containerd/io.containerd.snapshotter.v1.overlayfs/…"
for UID 0, GID 0: … no such file or directory
```

This is the **Docker host** failing to unpack an image layer under the containerd
snapshotter — not an Overup bug, and typically transient (the same pull often
succeeds on retry). Known causes, in likelihood order for a VPS:

1. **Disk pressure** — `catthehacker/ubuntu:act-latest` needs several GB to extract;
   an ENOSPC mid-extraction surfaces as bizarre `Lchown`/`no such file` errors, not
   always as "no space left on device".
2. **Leftover partial-pull state** — a cancelled/failed pull leaves snapshotter and
   content-store data that `docker system prune` cannot always clear
   (containerd/containerd#10548).
3. **Nested/loopback filesystems** — running the daemon on overlay-on-overlay
   (LXC/rootless setups) breaks layer extraction (containerd/containerd#2402,
   moby/moby#43576). `/var/lib/containerd` needs a real ext4, or xfs with `ftype=1`.

Host healing sequence:

```bash
df -h /var/lib/docker /var/lib/containerd    # several GB free?
docker system df
docker system prune -af                      # then clear partial-extract state:
sudo systemctl restart docker
docker pull catthehacker/ubuntu:act-latest   # must extract cleanly now
```

> **Careful:** on a Dokploy-style host the daemon restart re-creates
> `/var/run/docker.sock` and drops the uid-10001 ACL unless the systemd drop-in from
> [fix-hosted-runner-docker-socket.md](./fix-hosted-runner-docker-socket.md) §2 is in
> place — that is the `connected but ping failed` regression. Verify
> `getfacl /var/run/docker.sock` still lists `user:10001:rw-` after the restart.

Two platform mitigations make this class of failure non-fatal going forward:

- **Pre-pull warm-up** — `RUNNER_PREPULL_IMAGES` (default: the default job image)
  pulls job images into the daemon on every provisioner (re)connect, so the common
  `pulling_image` stage is a cache hit instead of a multi-GB network+extract.
- **Local-image fallback in the runner** — when a job's image pull errors but the
  daemon already holds the image, the job logs
  `image pull failed (…); using locally cached image` and proceeds. Only a pull
  failure with **no** local copy fails the job (`image_pull_failed`).

## 7. `cargo: not found` — pick the right JOB image per toolchain

`catthehacker/ubuntu:act-latest` (the default job image, and what `runs-on:
ubuntu-latest` maps to) is deliberately a **medium** image: Node.js, Python, git,
build-essential — enough for most actions at a fraction of GitHub's runner size. It
does **not** include Rust, Go, Java, or .NET. The
[catthehacker/docker_images](https://github.com/catthehacker/docker_images) catalog
publishes per-toolchain variants: `rust-latest` (rustfmt, clippy, cbindgen),
`js-latest`, `go-latest`, `java-tools-latest`, `pwsh-latest`, and `full-latest`
(GitHub's full hosted-runner toolcache — ~60 GB extracted; not practical on a VPS).

How Overup chooses a job's image (`backend/src/services/pipeline_plan.rs`), highest
precedence first:

1. the job's **`container:`** key (string or `{image: …}` map), when it is a valid
   image reference;
2. the **`runs-on` label map** (`ubuntu-latest`/`ubuntu-24.04`/… →
   `catthehacker/ubuntu:act-*`);
3. the **`DEFAULT_JOB_IMAGE`** env (itself defaulting to
   `catthehacker/ubuntu:act-latest`).

So a Rust job needs exactly one line in its workflow YAML:

```yaml
jobs:
  build:
    runs-on: ubuntu-latest
    container: ghcr.io/catthehacker/ubuntu:rust-latest   # or rust:1-bookworm (official, smaller)
    steps:
      - run: cargo build --release
```

The only no-YAML alternative is installing the toolchain as the job's first step —
correct but slow, since it re-downloads every run:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
```

Whatever image a job pins, add it to `RUNNER_PREPULL_IMAGES` (comma-separated) so it
is warmed into the daemon instead of pulled mid-pipeline.

## 8. Troubleshooting quick reference

| Symptom | Diagnosis | Fix |
| --- | --- | --- |
| `checkout complete` (no file count) **before** `pulling image` | Old runner binary is executing | §2–§4, in order |
| Rebuilt + pushed, jobs still old | Image built from a stale clone (fresh timestamp, old binary) | §2 `git log` check, §3 grep check |
| Pulled new image on VPS, jobs still old | Runner containers were restarted, not re-created | §4 revoke + re-create |
| `checkout failed: repository archive contained no usable files` | New binary working as designed: the tarball filtered to zero files (unexpected layout) | Inspect the repo/tarball; this replaces the old silent-empty behavior |
| `image pull failed (…); using locally cached image` (job continues) | New binary tolerating a transient pull failure | Nothing — informational; heal the host per §6 when frequent |
| `failed to extract layer … Lchown … no such file or directory` | Docker-host snapshotter state | §6 |
| `sh: 1: <tool>: not found` in a step | Job image lacks the toolchain | §7 |
| GitHub Actions runs all end `startup_failure` at 0s (even Dependabot) | Private-repo Actions billing block | github.com → Settings → Billing: valid payment method + non-zero Actions spending limit |
| `connected but ping failed` provisioner warn after `systemctl restart docker` | Socket ACL dropped by the daemon restart | [fix-hosted-runner-docker-socket.md](./fix-hosted-runner-docker-socket.md) §2 |
