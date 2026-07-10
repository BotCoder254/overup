# Deploying an Overup runner

The backend ([deploy-backend-dokploy.md](./deploy-backend-dokploy.md)) only *schedules*
pipelines — a **runner** executes them. Without at least one connected runner whose
labels match your workflows' `runs-on` values, every pipeline stays **Queued** forever.

This guide covers registering a runner, its full environment reference, and three
deployment options: **Docker (recommended)**, **as a Dokploy Application**, and
**bare binary + systemd**.

---

## 1. How the runner works (what shapes the deployment)

- **Outbound-only.** The runner opens one WebSocket **out** to the backend
  (`OVERUP_URL` → `wss://api.example.com/runner/ws`, authenticated with
  `Authorization: Bearer <RUNNER_TOKEN>`). It needs **no inbound ports, no domain,
  no reverse-proxy entry** — any machine that can reach your API over HTTPS works.
- **It never runs job code itself.** Every job executes in a **Docker container**
  the runner creates via the Docker Engine API (bollard). The runner streams logs,
  enforces timeouts/cancellation, uploads artifacts, and cleans up. So the one hard
  requirement is **access to a Docker daemon**.
- **One job at a time.** Throughput scales by registering more runners (each with its
  own token), on the same machine or many.
- **Self-healing connection.** It reconnects with exponential backoff (1 s → 60 s) if
  the backend restarts or the network drops. It exits permanently only when the
  control plane revokes it.
- **Signed payloads.** Every job assignment is HMAC-SHA256-signed by the backend and
  verified by the runner *before parsing* — which is why `RUNNER_JOB_SIGNING_KEY`
  must be **byte-for-byte identical** on both sides.
- **No R2 credentials needed.** Artifact uploads and source-tarball downloads use
  short-lived presigned/scoped URLs minted by the backend.

**Where to run it:** a **separate small VPS is the recommended place** — CI jobs are
arbitrary code, and whoever controls the Docker daemon effectively controls that
machine. Running it on the same VPS as Dokploy/the backend works (Option B) and is
fine for a personal setup, but understand the blast radius: jobs share the host's
Docker daemon with your control plane.

## 2. Register the runner and get its token

Runners belong to a workspace. Creating one returns the registration token
**exactly once** (only its SHA-256 hash is stored — it cannot be shown again).

Until the Runner Management UI ships, use the API. It's session-authenticated, so
grab your session cookie from the browser (DevTools → Application → Cookies →
`overup_session`, or `__Host-overup_session` in production) after logging in:

```bash
# Workspace id: visible in API responses (e.g. GET /api/me/workspaces) or the DB.
curl -sS -X POST "https://api.example.com/api/workspaces/<workspace-id>/runners" \
  -H "Content-Type: application/json" \
  -H "X-Requested-With: XMLHttpRequest" \
  -H "Cookie: __Host-overup_session=<your-session-cookie-value>" \
  -d '{"name": "vps-runner-1", "labels": ["self-hosted", "linux", "x64", "ubuntu-latest"]}'
```

Response (the only time you'll ever see `token`):

```json
{
  "runner": { "id": "…", "name": "vps-runner-1", "labels": ["self-hosted", "linux", "x64", "ubuntu-latest"], … },
  "token": "…copy this into RUNNER_TOKEN now…"
}
```

Also available: `GET …/runners` (list) and `DELETE …/runners/{runner_id}` (revoke —
a revoked runner disconnects, exits, and its token is dead forever; lost token =
revoke + create a new runner).

**Labels matter.** A job is dispatched to a runner only when the job's `runs-on`
values are a **subset** of the runner's labels. If your workflows say
`runs-on: ubuntu-latest`, the runner must carry the `ubuntu-latest` label.

## 3. Environment reference

Copy [`runner/.env.example`](../runner/.env.example) and fill it in. Everything the
runner reads:

| Variable | Required | Default | Meaning |
| --- | --- | --- | --- |
| `OVERUP_URL` | recommended | `http://localhost:8080` | Backend origin. `http→ws`, `https→wss`. Production: `https://api.example.com`. |
| `RUNNER_TOKEN` | **yes** | — | Registration token from §2 (shown once). |
| `RUNNER_JOB_SIGNING_KEY` | **yes** | — | HMAC key for job-payload verification, ≥ 32 bytes. **Must equal the backend's `RUNNER_JOB_SIGNING_KEY` exactly.** |
| `RUNNER_NAME` | no | `overup-runner` | Display name sent at connect. |
| `RUNNER_LABELS` | recommended | `self-hosted` | Comma-separated. Must cover your workflows' `runs-on` values. |
| `RUNNER_CAP_DROP` | no | `true` | Drop ALL capabilities in job containers (`no-new-privileges` is always on regardless). |
| `RUNNER_JOB_MEMORY_BYTES` | no | `2147483648` (2 GiB) | Per-job memory limit; `0` = unlimited. |
| `RUNNER_JOB_NANO_CPUS` | no | `2000000000` (2 CPUs) | Per-job CPU limit in 10⁻⁹ CPUs; `0` = unlimited. |
| `RUNNER_JOB_PIDS_LIMIT` | no | `512` | Per-job process cap; `0` = unlimited. |
| `RUNNER_JOB_NETWORK` | no | `bridge` | `bridge` \| `none` \| `isolated` (throwaway per-job network). |
| `RUNNER_JOB_USER` | no | *(image user)* | Run job containers as e.g. `1000:1000`. |
| `RUNNER_JOB_READONLY_ROOTFS` | no | `false` | Read-only rootfs + tmpfs `/tmp` for job containers. |
| `DOCKER_HOST` | no | local socket/pipe | Remote daemon, e.g. `tcp://host:2376` (see §7). |
| `DOCKER_TLS_VERIFY` | with remote | — | Set `1` for TLS-verified remote daemons. |
| `DOCKER_CERT_PATH` | with remote | — | Directory with `ca.pem`/`cert.pem`/`key.pem`. |
| `RUST_LOG` | no | `info` | Log filter. |

The runner refuses to start if `RUNNER_TOKEN` or `RUNNER_JOB_SIGNING_KEY` is missing,
or if the signing key is shorter than 32 bytes.

## 4. Option A — Docker (recommended)

### 4.1 Build the image

From the **repository root** (the context must include both `runner/` and `protocol/`
— the runner depends on the sibling `protocol` crate, same rule as the backend image):

```bash
docker build -f runner/Dockerfile -t overup-runner:latest .
```

Multi-stage: `rust:1-slim-bookworm` builder with a cached dependency layer →
`debian:bookworm-slim` runtime, non-root `runner` user (uid 10001), `ca-certificates`
only.

### 4.2 The two mounts you must get right

Running Docker-driven CI **inside** a container means the runner talks to the
**host's** daemon ("Docker-outside-of-Docker"). Two consequences:

1. **The socket.** Mount `/var/run/docker.sock` into the container. The image runs as
   a non-root user, so also grant it the socket's group:
   `--group-add "$(stat -c %g /var/run/docker.sock)"`.

2. **The shared work directory — the identical-path rule.** The runner creates each
   job workspace as a temp dir (under `TMPDIR`, which the image sets to
   `/opt/overup/work`) and asks the daemon to bind-mount it into the job container.
   But the *host* daemon resolves that path **on the host**, not inside the runner
   container. So the same path must exist on both sides of the boundary:

   ```
   -v /opt/overup/work:/opt/overup/work
   ```

   Skip or rename this mount and jobs will start with an **empty `/workspace`**
   (Docker silently creates the missing host dir): checkouts appear to succeed
   runner-side but the job container sees none of it.

### 4.3 Run it

```bash
sudo mkdir -p /opt/overup/work && sudo chown 10001:10001 /opt/overup/work
# env file from runner/.env.example, root-owned:
sudo install -m 600 runner.env /opt/overup/runner.env

docker run -d \
  --name overup-runner \
  --env-file /opt/overup/runner.env \
  -v /var/run/docker.sock:/var/run/docker.sock \
  --group-add "$(stat -c %g /var/run/docker.sock)" \
  -v /opt/overup/work:/opt/overup/work \
  --restart unless-stopped \
  overup-runner:latest
```

In the env file set at minimum: `OVERUP_URL=https://api.example.com`, `RUNNER_TOKEN`,
`RUNNER_JOB_SIGNING_KEY` (same as backend), `RUNNER_LABELS`.

### 4.4 Or Docker Compose

```yaml
# compose.runner.yml — run from the repo root on the runner machine
services:
  overup-runner:
    build:
      context: .                    # repo root — protocol/ must be in context
      dockerfile: runner/Dockerfile
    # or: image: ghcr.io/botcoder254/overup-runner:latest
    env_file:
      - /opt/overup/runner.env
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
      - /opt/overup/work:/opt/overup/work   # identical path on both sides!
    group_add:
      - "988"        # $(stat -c %g /var/run/docker.sock) — docker group gid on YOUR host
    restart: unless-stopped
```

(Compose can't shell out for the gid — look it up once with
`stat -c %g /var/run/docker.sock` and hard-code it.)

## 5. Option B — as a Dokploy Application (same VPS as the backend)

Dokploy can build and run the runner exactly like the backend. Create a second
Application from the same repository:

| Setting | Value |
| --- | --- |
| Provider / Repository | GitHub / `BotCoder254/overup` |
| Branch | `main` |
| Build Path | `/` |
| Build Type | **Dockerfile** |
| Docker File | `runner/Dockerfile` |
| Docker Context Path | `.` |
| Trigger Type | On Push |
| Watch Paths | `runner/**`, `protocol/**` |
| **Domain** | **none — do not add one.** The runner listens on nothing. |

**Environment tab** — same content as the env file in Option A:

```env
OVERUP_URL=https://api.example.com
RUNNER_TOKEN=<token from §2>
RUNNER_JOB_SIGNING_KEY=<same value as the backend app>
RUNNER_NAME=dokploy-runner-1
RUNNER_LABELS=self-hosted,linux,x64,ubuntu-latest
RUST_LOG=info
```

(`OVERUP_URL` can simply be the public API domain — the WSS connection going out
through Traefik and back is fine and keeps the config identical everywhere.)

**Mounts** (application → Advanced → Mounts, both of type *Bind Mount*):

| Host Path | Container Path |
| --- | --- |
| `/var/run/docker.sock` | `/var/run/docker.sock` |
| `/opt/overup/work` | `/opt/overup/work` |

Create the work dir on the VPS first: `mkdir -p /opt/overup/work && chown 10001:10001 /opt/overup/work`.

One caveat Dokploy can't express per-app: the socket's group. If the runner logs
`docker engine unreachable` (permission denied), the quickest fixes are either a
Dokploy "Advanced → Custom" user setting of `0` (run this one container as root —
container-local root, still confined) or widening the socket group on the host.

**Understand the trade-off:** this hands the host's Docker daemon — the same daemon
running Dokploy, Traefik, your backend, and Postgres — to the CI runner. Job
containers themselves are hardened (caps dropped, `no-new-privileges`, resource
limits), but a dedicated runner VPS (Option A there) is the cleaner boundary.

## 6. Option C — bare binary + systemd (no container)

On any Linux box with Docker and Rust installed:

```bash
git clone https://github.com/BotCoder254/overup.git /opt/overup/src
cd /opt/overup/src/runner
cargo build --release
sudo install -m 755 target/release/runner /usr/local/bin/overup-runner
sudo install -m 600 .env /etc/overup/runner.env   # filled-in copy of .env.example
```

`/etc/systemd/system/overup-runner.service`:

```ini
[Unit]
Description=Overup CI runner
After=network-online.target docker.service
Wants=network-online.target

[Service]
User=overup-runner
Group=docker
EnvironmentFile=/etc/overup/runner.env
ExecStart=/usr/local/bin/overup-runner
Restart=always
RestartSec=5
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
```

```bash
sudo useradd --system overup-runner
sudo systemctl daemon-reload
sudo systemctl enable --now overup-runner
journalctl -u overup-runner -f
```

No path-mapping concerns here — the runner and the daemon see the same filesystem.

## 7. Remote TLS Docker daemon

The runner can drive a Docker daemon on a *different* machine — useful for keeping
execution off the runner host entirely:

```env
DOCKER_HOST=tcp://docker-host.example.com:2376
DOCKER_TLS_VERIFY=1
DOCKER_CERT_PATH=/certs    # ca.pem, cert.pem, key.pem (mount into the container)
```

**Never expose an unauthenticated `tcp://…:2375` daemon** — that is unauthenticated
root on that machine for the whole internet. 2376 + mutual TLS only.

Note the identical-path rule from §4.2 does **not** apply to remote daemons the same
way — bind mounts resolve on the *daemon's* host, so runner-created workspaces won't
be visible there. Remote daemons work best with Option C (bare runner on the daemon's
own host) or are best avoided until you're comfortable with the mount semantics.

## 8. Security notes

- **The Docker socket is root-equivalent** on whichever host owns it. Prefer a
  dedicated runner VPS; treat the runner machine as compromised-by-design and keep
  nothing else valuable on it.
- **Job hardening is on by default** and configured runner-side (§3): all capabilities
  dropped, `no-new-privileges` always, 2 GiB/2 CPU/512-pid budgets, per-job disposable
  workspaces, SIGKILL on cancel/timeout, containers force-removed. Consider
  `RUNNER_JOB_NETWORK=isolated` (per-job throwaway network) and `RUNNER_JOB_USER` +
  `RUNNER_JOB_READONLY_ROOTFS=true` for stricter setups; loosen limits only when a
  workload demands it.
- **Secrets:** `RUNNER_TOKEN` and `RUNNER_JOB_SIGNING_KEY` live only in the env file
  (root-owned, `chmod 600`) or Dokploy's Environment tab — never in git
  (`.gitignore`/`.dockerignore` exclude `.env*` as a backstop). The token is
  per-runner and instantly revocable; the signing key is shared — rotating it means
  updating the backend and **every** runner together.
- **Checkout tokens** inside job payloads are short-lived (1 h), scoped to
  `contents:read`, and automatically masked out of all job logs by the backend.
- **Keep the base images fresh:** rebuild the runner image periodically to pick up
  Debian security updates.

## 9. First run & verification

1. Start the runner and read its log. A healthy boot looks like:
   - `docker engine reachable` (endpoint = local socket/pipe or your `DOCKER_HOST`)
   - a successful connect (after `hello_ack` the backend has it registered as idle;
     heartbeats follow at the cadence the server dictates)
2. In the backend's ledger, trigger a pipeline: **manual dispatch** from the UI, or
   push to a synced repository with a workflow whose `runs-on ⊆ RUNNER_LABELS`.
3. Watch it go `Queued → In progress`, logs streaming live in the pipeline detail
   view, then a conclusion. Metrics (CPU/memory/net) appear on the Performance tab.
4. `docker ps` on the runner host during a job shows the job container; after the
   job it's gone (force-removed), and the temp workspace is deleted.

## 10. Troubleshooting

| Symptom | Likely cause / fix |
| --- | --- |
| Runner exits: `missing required environment variable …` | `RUNNER_TOKEN` or `RUNNER_JOB_SIGNING_KEY` unset in the env file / Environment tab. |
| Runner exits: `RUNNER_JOB_SIGNING_KEY must be at least 32 bytes` | Use the full `openssl rand -hex 32` output (64 hex chars). |
| Connection rejected / immediately closed at connect | Wrong or **revoked** `RUNNER_TOKEN` (only the hash is stored — if you lost the token, revoke the runner and create a new one). |
| `this runner was revoked by the control plane; exiting` | Expected after `DELETE …/runners/{id}` — the process exits permanently; deploy a new runner with a new token. |
| Connects fine, but every job fails instantly around payload verification | `RUNNER_JOB_SIGNING_KEY` differs from the backend's — signed payloads fail constant-time verification and are never executed. Also check for stray whitespace/quotes in the env file. |
| `docker engine unreachable; jobs will fail` | Socket not mounted, socket group not granted (`--group-add`), or `DOCKER_HOST` wrong. The runner stays connected but jobs fail cleanly until Docker is reachable. |
| Jobs run but `/workspace` is empty (checkout "succeeded") | **The identical-path rule (§4.2):** `/opt/overup/work` isn't mounted host↔container at the same path, so the host daemon bind-mounted a different (empty) directory. |
| Pipelines stay **Queued** with the runner online | Labels don't cover the job's `runs-on` (subset rule), or the single job slot is busy — check `RUNNER_LABELS` against the workflow, or add runners. |
| `wss` connect fails through the proxy | `OVERUP_URL` must be the public origin (`https://api.example.com`); Traefik/Dokploy proxies WebSockets natively — check the URL and TLS cert before suspecting the proxy. |
| Job containers can't reach the network | `RUNNER_JOB_NETWORK=none` set, or `isolated` combined with a workload expecting the default bridge. |
