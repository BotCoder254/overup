# Runner image — build, publish to GHCR, and update

The hosted-runner provisioner runs runner containers from the image named by
`RUNNER_IMAGE` (default `ghcr.io/botcoder254/overup-runner:latest`). **Nothing in
this repository builds or pushes that image automatically** — it must be published
once (this guide), or built directly on the server's daemon (§6). Until one of
those happens, a fresh deployment logs:

```
WARN backend::services::runner_provisioner: image pre-pull failed — jobs needing
it will pull on demand  image=ghcr.io/botcoder254/overup-runner:latest
error=DockerResponseServerError { status_code: 500, message: "error from registry: denied\ndenied" }
```

The image is defined by [`runner/Dockerfile`](../runner/Dockerfile): a multi-stage
build (`rust:1-slim-bookworm` builder → `debian:bookworm-slim` runtime, non-root
`runner` user uid 10001, `ca-certificates` only, `TMPDIR=/opt/overup/work`). The
same image serves hosted runners **and** manually deployed runners
([deploy-runner.md](./deploy-runner.md) §4).

## 1. Prerequisites

- Docker on any machine (your workstation is fine — the image is pushed to a
  registry, it does not have to be built on the VPS).
- The repository checked out (`git clone https://github.com/BotCoder254/overup.git`).
- A GitHub account that will own the GHCR package.

## 2. Build

**The build context must be the repository root** — the runner crate depends on
the sibling `protocol/` crate, so both directories have to be inside the context.
Building from anywhere else fails with `resolve : lstat runner: no such file or
directory`.

```bash
cd <repo-root>        # `ls` must show runner/, protocol/, backend/
docker build -f runner/Dockerfile -t ghcr.io/<your-gh-username>/overup-runner:latest .
```

The first build compiles the Rust dependencies and takes several minutes; the
Dockerfile caches the dependency layer, so source-only rebuilds are much faster.

### Building from WSL against a Windows checkout

The Windows filesystem is mounted under `/mnt/c` — quote the path if it contains
spaces:

```bash
cd "/mnt/c/Users/<you>/Desktop/overup"
docker build -f runner/Dockerfile -t ghcr.io/<your-gh-username>/overup-runner:latest .
```

`/mnt/c` I/O is slow; if the context upload or build crawls, clone natively into
the WSL filesystem instead:

```bash
cd ~ && git clone https://github.com/BotCoder254/overup.git && cd overup
```

## 3. Publish to GHCR

1. Create a **PAT (classic)** with the **`write:packages`** scope:
   github.com → Settings → Developer settings → Personal access tokens →
   Tokens (classic) → Generate new token. (Fine-grained tokens do not support
   GHCR pushes; use classic.)

2. Log in and push:

   ```bash
   echo <PAT> | docker login ghcr.io -u <your-gh-username> --password-stdin
   docker push ghcr.io/<your-gh-username>/overup-runner:latest
   ```

3. **Make the package public — required.** `docker push` creates GHCR packages
   as **private**, and the provisioner (and every VPS) pulls anonymously:
   github.com → your profile → **Packages** tab → `overup-runner` →
   **Package settings** → Danger Zone → **Change visibility → Public**.

4. Verify an anonymous pull works (this is exactly what the VPS does):

   ```bash
   docker logout ghcr.io
   docker pull ghcr.io/<your-gh-username>/overup-runner:latest
   ```

If you published under a different account/name than the default, set
`RUNNER_IMAGE=ghcr.io/<your-gh-username>/overup-runner:latest` in the backend
environment.

## 4. Point the deployment at it

Nothing else to configure when you used the default name. Restart/redeploy the
backend (or just wait for the provisioner's next reconnect) and the log should
show:

```
hosted-runner provisioner connected  via=/var/run/docker.sock ...
pre-pulled image into the docker daemon  image=ghcr.io/.../overup-runner:latest ...
```

Hosted runner creation in the wizard now proceeds past the image-pull stage.

## 5. Updating the image

Rebuild and push the same tag whenever `runner/` or `protocol/` change:

```bash
docker build -f runner/Dockerfile -t ghcr.io/<your-gh-username>/overup-runner:latest .
docker push ghcr.io/<your-gh-username>/overup-runner:latest
```

The backend pre-pulls `:latest` on every provisioner (re)connect and before every
hosted-runner provision, so **new** runner containers get the new image
automatically. **Existing** runner containers keep the image they were created
from — revoke and re-create a hosted runner to move it onto the new image.

## 6. No-registry alternative: build on the VPS

You can skip GHCR entirely and build the image on the server's own daemon under
the same tag:

```bash
# on the VPS, in a checkout of the repo
docker build -f runner/Dockerfile -t ghcr.io/<your-gh-username>/overup-runner:latest .
```

The provisioner still tries the registry first (to keep `:latest` fresh) and that
pull still fails, but a **locally present image satisfies both the pre-pull
warm-up and hosted-runner provisioning** — the log downgrades to
`image already present locally — registry pull failed, skipping`. Provisioning
only fails (`image_pull_failed`) when the pull fails *and* no local copy exists.
The trade-off: you rebuild manually on the VPS for every update.

## 7. Troubleshooting

| Symptom | Fix |
| --- | --- |
| `resolve : lstat runner: no such file or directory` at build | You ran `docker build` outside the repo root — `cd` to the checkout (the `.` context must contain `runner/` **and** `protocol/`). |
| `error from registry: denied` on pull / pre-pull | The package was never pushed, or is still **private** — complete §3 including the visibility flip, then re-test with an anonymous `docker pull`. |
| `denied` / `unauthorized` on **push** | The PAT lacks `write:packages`, is expired, or you're pushing to someone else's namespace — the path segment after `ghcr.io/` must be your username (or an org you can publish in). |
| Build extremely slow under WSL | You're building from `/mnt/c` — clone natively into the WSL filesystem (§2). |
| VPS still warns after publishing | Restart/redeploy the backend (pre-pull runs on provisioner reconnect), and confirm `RUNNER_IMAGE` matches the name you actually pushed. |
| Wizard fails `image_pull_failed` | Registry pull failed **and** the image is absent locally — same fixes as the `denied` row, or build locally per §6. |
