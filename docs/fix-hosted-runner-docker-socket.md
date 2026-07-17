# Fix: hosted runners on Dokploy — "docker unreachable" runbook

This is the verified, recommended fix for the backend log warning below when the
backend runs as a **Dokploy Application** container and `RUNNER_PROVISIONER=docker`
is enabled. Follow it top to bottom; each stage's log output tells you which step
you're on.

```
WARN backend::services::runner_provisioner: hosted-runner provisioner: docker
unreachable — retrying every 30s attempts=...
```

The warning is never fatal: the backend retries every 30 seconds, so the moment the
fix lands, hosted runners come alive **without a restart**.

## Why this happens

The backend runs inside a container as a dedicated non-root user (uid 10001, from
`backend/Dockerfile`). The VPS host's Docker daemon and its socket
(`/var/run/docker.sock`, owned `root:docker`, mode `660`) are invisible from inside
a container until you (1) mount the socket in and (2) give uid 10001 permission to
open it. Those are the first two stages below — the `attempts=` line tells you which
one you still need. Stage 3 covers the follow-up `error from registry: denied`
pre-pull warn (the runner image itself was never published).

| The log says | Stage you're on |
| --- | --- |
| `attempts=` … `socket not present` on every path | Stage 1 — the socket isn't mounted |
| `attempts=` … `connected but ping failed (Error in the hyper legacy client: client error (Connect))` or `permission denied` | Stage 2 — mounted, but uid 10001 can't open it |
| `hosted-runner provisioner connected`, then `image pre-pull failed … error from registry: denied` | Stage 3 — connected, but the runner image isn't pullable |
| *(connected, images pre-pulled, no warns)* | Fixed — see Verification |

## Stage 1 — bind-mount the Docker socket into the backend container

In the Dokploy dashboard:

1. Open the backend **Application**.
2. Go to **Advanced → Mounts** (called *Volumes* in some versions).
3. **Add Mount** → type **Bind Mount** (not Volume, not File Mount).
4. **Host Path:** `/var/run/docker.sock`
5. **Mount Path:** `/var/run/docker.sock`
6. Save, then **Redeploy** — mounts only take effect on redeploy.

Gotcha: Dokploy runs apps under Docker Swarm, and Swarm silently refuses to start a
service with an invalid mount even when the deploy screen looks successful. If the
app won't come up after this step, re-check the mount type is **Bind Mount** and
both paths are exact.

After the redeploy the warn changes from `socket not present` to
`connected but ping failed` — that's progress, not failure. It means the socket is
visible but permission is still missing. Continue to Stage 2.

## Stage 2 — grant uid 10001 access to the socket (host ACL)

Dokploy's Application type has **no group-add field** in its Advanced settings, so
the container can't join the host's `docker` group. The recommended fix is a
host-side ACL that grants exactly uid 10001 — nothing else on the host changes, and
no redeploy is needed (the backend's 30-second retry picks it up by itself).

SSH into the VPS:

```bash
# 1. Install the ACL tool if missing (Ubuntu/Debian)
sudo apt-get install -y acl

# 2. Grant uid 10001 read/write on the Docker socket
sudo setfacl -m u:10001:rw /var/run/docker.sock

# 3. Confirm — the output must include "user:10001:rw-"
getfacl /var/run/docker.sock
```

Within ~30 seconds the backend log should print:

```
hosted-runner provisioner connected  via=/var/run/docker.sock ...
```

### Make it survive Docker restarts and reboots (required)

The daemon recreates the socket file on every start, which drops the ACL — a VPS
reboot or `systemctl restart docker` would silently re-break hosted runners. A
systemd drop-in re-applies the ACL every time Docker starts:

```bash
sudo mkdir -p /etc/systemd/system/docker.service.d
sudo tee /etc/systemd/system/docker.service.d/overup-socket-acl.conf <<'EOF'
[Service]
ExecStartPost=/usr/bin/setfacl -m u:10001:rw /var/run/docker.sock
EOF
sudo systemctl daemon-reload
```

No Docker restart is needed now (the ACL from step 2 is already live). Test the
persistence whenever convenient:

```bash
sudo systemctl restart docker && getfacl /var/run/docker.sock   # user:10001:rw- must still be there
```

## Required environment (recap)

In the Application's **Environment** tab, alongside the rest of §5 of
[deploy-backend-dokploy.md](./deploy-backend-dokploy.md):

```bash
RUNNER_PROVISIONER=docker
# URL runner CONTAINERS reach the API on — never localhost (inside a container
# that is the container itself). Docker's bridge gateway usually works on a
# single host; otherwise use the public API origin.
RUNNER_PROVISIONER_OVERUP_URL=http://172.17.0.1:8080
# Optional: job images pre-pulled on every deploy/restart (default = DEFAULT_JOB_IMAGE)
#RUNNER_PREPULL_IMAGES=catthehacker/ubuntu:act-latest
```

## Stage 3 — publish the runner image (fixes `error from registry: denied`)

Once connected, the next warn many fresh deployments hit is:

```
WARN backend::services::runner_provisioner: image pre-pull failed — jobs needing
it will pull on demand  image=ghcr.io/botcoder254/overup-runner:latest
error=DockerResponseServerError { status_code: 500, message: "error from registry: denied\ndenied" }
```

This is **not** a missing env var: the default `RUNNER_IMAGE`
(`ghcr.io/botcoder254/overup-runner:latest`) is a GHCR package that nothing in the
repo publishes automatically — until you push it once (and make it public), the
registry denies every anonymous pull. Two fixes, pick one
(full step-by-step guide: [publish-runner-image.md](./publish-runner-image.md)):

```bash
# A) Publish once from any machine with the repo (recommended):
docker build --build-arg BUILD_REF=$(git rev-parse --short HEAD) -f runner/Dockerfile -t ghcr.io/<your-gh-username>/overup-runner:latest .
echo <PAT with write:packages> | docker login ghcr.io -u <your-gh-username> --password-stdin
docker push ghcr.io/<your-gh-username>/overup-runner:latest
# then on github.com: Packages → overup-runner → Package settings → Change visibility → Public
# (docker push creates GHCR packages PRIVATE by default — this step is required)

# B) No registry — build on the VPS under the same tag:
docker build --build-arg BUILD_REF=$(git rev-parse --short HEAD) -f runner/Dockerfile -t ghcr.io/<your-gh-username>/overup-runner:latest .
```

With option B the registry pull still fails, but a locally present image satisfies
both the pre-pull warm-up and hosted-runner provisioning (the log downgrades to
`image already present locally — registry pull failed, skipping`). If you publish
under your own username, set `RUNNER_IMAGE` in the Environment tab to match.

## Verification

1. Backend runtime log shows, in order:
   ```
   hosted-runner provisioner connected  via=/var/run/docker.sock ...
   pre-pulled image into the docker daemon  image=ghcr.io/botcoder254/overup-runner:latest ...
   pre-pulled image into the docker daemon  image=ubuntu:24.04 ...
   ```
2. `docker images` on the VPS lists `ubuntu:24.04` and the runner image.
3. The Runners page wizard offers the hosted path; a created runner goes **Online**.
   (Provisions but never connects → `RUNNER_PROVISIONER_OVERUP_URL` isn't reachable
   from inside a container — see the recap above.)
4. Run a pipeline: the job's `pulling_image` stage completes near-instantly because
   the image is already local.

## Security note

Docker-socket access is **root-equivalent on the host**. That is inherent to hosted
runners (the provisioner must control the daemon it provisions on) and acceptable on
a single-operator VPS; the ACL keeps the grant scoped to exactly uid 10001. Never
expose the daemon over unauthenticated `tcp://2375`.

*(Running the backend as a Dokploy **Compose** service instead supports `group_add`
natively — see §9.3 of [deploy-backend-dokploy.md](./deploy-backend-dokploy.md) if
you ever migrate; with the ACL in place it is not needed.)*
