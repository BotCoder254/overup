# Deploying the Overup backend to a VPS with Dokploy

This guide deploys **only the Rust backend** (`backend/`) — nothing else. No frontend,
no runner. It assumes:

- You have a VPS with [Dokploy](https://dokploy.com) already installed and running.
- You already created a **PostgreSQL service** in Dokploy (and any other services you need).
- You already **connected your GitHub account** to Dokploy (Settings → Git → GitHub) and
  created a new **Application** from this repository.
- You control a domain and can create DNS records (we use `api.example.com` as the
  backend domain and `app.example.com` as the future frontend domain — replace with
  your own everywhere).

There are two supported paths, both covered below:

1. **[Dokploy Application with the Dockerfile build type](#4-dokploy-application-setup-recommended)** — recommended.
   Dokploy pulls the repo, builds `backend/Dockerfile`, and runs the container behind
   its built-in Traefik reverse proxy with automatic HTTPS.
2. **[Raw Docker / Docker Compose](#9-alternative-raw-docker--docker-compose)** — the same
   image built and run by hand, or as a Dokploy **Compose** service.

---

## 1. What you are deploying (architecture)

The backend is a single self-contained binary:

- **axum HTTP + WebSocket server** listening on `BIND_ADDR` (container port **8080**).
- **PostgreSQL** is its only required external service. Database **migrations are
  embedded in the binary** (`sqlx::migrate!`) and run automatically on every startup —
  there is no separate migration step, ever.
- **GitHub OAuth App** (user login) and **GitHub App** (repository/workflow access +
  webhooks) — both talk to `api.github.com` over rustls; you only supply credentials.
- **Cloudflare R2** (optional) for artifacts and archived logs. Without the `R2_*`
  variables, pipelines still run; artifact uploads are denied and logs stay in Postgres.
- Background workers (scheduler, janitor, orphan recovery) start inside the same
  process — one container is the whole control plane.

Runners are **not** part of this deployment. They run elsewhere (any machine with
Docker), dial **out** to the backend over WebSocket (`/runner/ws`), and need no inbound
ports — the backend being publicly reachable over HTTPS is enough. Deploy one after
the backend is live: see [deploy-runner.md](./deploy-runner.md).

Traffic flow in production:

```
Browser ──HTTPS──▶ Traefik (Dokploy, ports 80/443) ──▶ overup-backend:8080
GitHub webhooks ──HTTPS──▶ Traefik ──▶ overup-backend:8080  (/webhooks/github)
Runner (your machine) ──WSS──▶ Traefik ──▶ overup-backend:8080  (/runner/ws)
overup-backend ──▶ postgres service (internal dokploy-network, port 5432 NOT published)
```

## 2. Prerequisites checklist

| Item | Notes |
| --- | --- |
| VPS with Dokploy | Ports 80/443 open to the world; the Dokploy panel port (default 3000) ideally firewalled to your IP. |
| Dokploy PostgreSQL service | Already created. Postgres 16 matches local dev (`docker-compose.yml`). |
| GitHub connected to Dokploy | Done via Dokploy Settings → Git → GitHub (installs Dokploy's GitHub App on your account so it can clone + receive push events). |
| DNS record | `A` record: `api.example.com → <VPS public IP>`. Create it **before** enabling HTTPS so Let's Encrypt validation succeeds. **Don't own a domain? Use a free DuckDNS subdomain — see [§6.1](#61-no-domain-yet-use-a-free-duckdns-subdomain).** |
| GitHub **OAuth App** | For user login. You'll update its callback URL to production in [§7](#7-point-the-github-apps-at-production). |
| GitHub **App** | For repositories/workflows/webhooks. You'll update Setup URL + Webhook URL to production in [§7](#7-point-the-github-apps-at-production). |
| `openssl` anywhere | To generate `RUNNER_JOB_SIGNING_KEY`. |

## 3. Database: use your existing Dokploy Postgres service

The backend needs one database. Use the Postgres service you already run in Dokploy.

1. Open the Postgres service in Dokploy and note:
   - the **internal service/host name** (Dokploy shows it as the "Internal Host" /
     connection string on the service page — it's the name resolvable on the shared
     `dokploy-network`, e.g. `overup-db-xxxxxx`),
   - the username, password, and port (5432).
2. Create a dedicated database (via the service's terminal in Dokploy, or any psql):

   ```sql
   CREATE DATABASE overup;
   -- optional but recommended: a dedicated role instead of the superuser
   CREATE USER overup WITH PASSWORD '<strong-random-password>';
   GRANT ALL PRIVILEGES ON DATABASE overup TO overup;
   \c overup
   GRANT ALL ON SCHEMA public TO overup;
   ```

3. Build the **internal** connection string the backend will use:

   ```
   DATABASE_URL=postgres://overup:<password>@<internal-postgres-host>:5432/overup
   ```

   Because both containers sit on Dokploy's internal Docker network, this URL uses the
   service's **internal hostname** — no TLS flags needed and, critically, **do not
   publish port 5432 to the internet** (leave "External Port" empty on the Postgres
   service). If you must use an external/hosted Postgres instead, append
   `?sslmode=require` (the backend supports TLS via rustls).

4. That's it. On first boot the backend applies every migration in
   `backend/migrations/` automatically and logs the result.

## 4. Dokploy Application setup (recommended)

You already created the Application and connected GitHub — now configure it field by
field. Open the application → **General** tab.

### 4.1 Provider / source

| Field | Value | Why |
| --- | --- | --- |
| Provider | **GitHub** | Uses the GitHub connection you set up; enables clone + auto-deploy on push. |
| Repository | `BotCoder254/overup` | This repo. |
| **Branch** | `main` | Deploy from `main` so every merged PR ships. To test before merging, temporarily select the feature branch (e.g. `feat/pipeline-execution-and-dokploy-docs`), deploy, verify, then switch back to `main` and save. |
| Watch Paths (optional) | `backend/**`, `protocol/**` | Only redeploy when backend-relevant files change — a frontend-only push won't trigger an API rebuild. Leave empty to redeploy on every push. |

### 4.2 Build settings — this is the important part

| Field | Value | Why |
| --- | --- | --- |
| **Build Type** | **Dockerfile** | The repo ships a production Dockerfile; don't use Nixpacks/Buildpacks — they'd try to build the Node frontend at the repo root. |
| **Docker File** | `backend/Dockerfile` | Path to the Dockerfile **relative to the repo root**. |
| **Docker Context Path** | `.` | **Must be the repo root.** The backend depends on the sibling crate `protocol/` (`protocol = { path = "../protocol" }` in `backend/Cargo.toml`), so the build context has to contain both `backend/` and `protocol/`. Setting the context to `backend` will fail with "`protocol` not found". |
| Docker Build Stage | *(leave empty)* | Empty builds the final (`runtime`) stage, which is what you want. |

> **Why not "Build Path"?** Some Dokploy build types (Nixpacks/static) expose a
> "Build Path" field for monorepos. With the **Dockerfile** build type the equivalent
> is the pair above: context `.` + Dockerfile `backend/Dockerfile`. That combination
> is what makes "run the backend only, nothing else" work from this monorepo.

### 4.3 Trigger type (when deploys happen)

In the application's **Deployments / General** settings:

| Field | Value | Why |
| --- | --- | --- |
| **Trigger Type** | **On Push** | Dokploy's GitHub App delivers push events; every push to the selected **Branch** (respecting Watch Paths) builds and deploys automatically. |
| Manual deploys | **Deploy** button | Always available on the application page — use it for the very first deploy and for redeploys without a new commit ("Rebuild" forces a no-cache build). |
| Webhook URL (only for non-GitHub providers) | Application → Deployments tab | If you ever switch the provider to a plain Git URL, Dokploy shows a deploy webhook URL there that you'd add to the repo's webhooks yourself. With the GitHub provider you don't need it. |

### 4.4 Advanced / container settings

- **Port**: the container listens on **8080** (`EXPOSE 8080`, `BIND_ADDR=0.0.0.0:8080`
  baked into the image). You will bind the domain to this port in [§6](#6-domain--https);
  do **not** add a public "Port Mapping" — that would bypass Traefik/HTTPS.
- **Replicas**: 1. The scheduler/janitor are in-process singletons; run exactly one
  instance of the control plane.
- **Restart policy**: Dokploy's default (`unless-stopped`/`always`) is correct — the
  binary is safe to restart at any time (boot-time orphan recovery re-adopts any
  in-flight jobs).

## 5. Environment variables

Application → **Environment** tab. Paste and fill the block below. It mirrors
`backend/.env.example` with production values. **Everything marked `<...>` is a secret —
set it only here, never in git.**

```env
# --- Core -------------------------------------------------------------------
DATABASE_URL=postgres://overup:<db-password>@<internal-postgres-host>:5432/overup
BIND_ADDR=0.0.0.0:8080

# Where the React frontend is (will be) served. Used for CORS and post-login
# redirects. Until the frontend is deployed, set it to the value it WILL have.
FRONTEND_URL=https://app.example.com

# Production = HTTPS, so this MUST be true. It turns on the Secure cookie,
# auto-prefixes the session cookie name with __Host- and enables HSTS.
COOKIE_SECURE=true

SESSION_TTL_HOURS=168

# --- GitHub OAuth App (user login) -------------------------------------------
GITHUB_CLIENT_ID=<oauth-app-client-id>
GITHUB_CLIENT_SECRET=<oauth-app-client-secret>
# Exact URL registered on the OAuth App (allow-list of one) — see §7.
OAUTH_REDIRECT_URL=https://api.example.com/auth/github/callback

# --- GitHub App (repositories, workflows, webhooks) ---------------------------
GITHUB_APP_CLIENT_ID=<github-app-client-id>
# Base64 of the downloaded .pem — see below. (Preferred over a file path in
# Dokploy: no volume mounts needed.)
GITHUB_APP_PRIVATE_KEY_B64=<base64-of-private-key-pem>
GITHUB_WEBHOOK_SECRET=<strong-random-webhook-secret>
GITHUB_APP_SLUG=<your-app-slug>

# --- Pipeline execution -------------------------------------------------------
# Shared HMAC key that signs job payloads pushed to runners. >= 32 random
# bytes; every runner must be started with the SAME value (it goes into the
# runner's env too — see docs/deploy-runner.md).
# Generate: openssl rand -hex 32
RUNNER_JOB_SIGNING_KEY=<64-hex-chars>

DEFAULT_JOB_IMAGE=ubuntu:24.04
JOB_TIMEOUT_SECONDS=3600
PIPELINE_TIMEOUT_SECONDS=7200
MAX_LOG_BYTES_PER_JOB=10485760
MAX_ARTIFACT_BYTES=104857600
MAX_ARTIFACTS_PER_JOB=10

# --- Cloudflare R2 (OPTIONAL — all four or none) ------------------------------
# Without these, pipelines run fine; artifact uploads are denied and logs
# stay in Postgres (no archival/pruning).
#R2_ACCOUNT_ID=<account-id>
#R2_ACCESS_KEY_ID=<access-key-id>
#R2_SECRET_ACCESS_KEY=<secret-access-key>
#R2_BUCKET=<bucket-name>

# --- Retention (janitor runs hourly) ------------------------------------------
ARTIFACT_RETENTION_DAYS=30
ARTIFACT_PENDING_TTL_HOURS=24
LOG_HOT_RETENTION_DAYS=7

# --- Logging ------------------------------------------------------------------
RUST_LOG=info,backend=info,tower_http=warn
```

Generating the two generated secrets:

```bash
# Job payload signing key (also given to every runner):
openssl rand -hex 32

# Webhook secret (any strong random string works):
openssl rand -hex 32
```

### 5.1 Converting the GitHub App private key to `GITHUB_APP_PRIVATE_KEY_B64`

The value must be the **base64 encoding of the entire `.pem` file** (including the
`-----BEGIN/END RSA PRIVATE KEY-----` lines), as a single line. Pasting the raw PEM
text itself into the variable is the classic mistake — the PEM's header dashes,
spaces, and line breaks are not valid base64, and the backend refuses to start with:

```
Error: GITHUB_APP_PRIVATE_KEY_B64 is not valid base64
```

Convert the downloaded key file on whatever machine you have:

```bash
# Linux (single line, no wrapping):
base64 -w0 your-app.private-key.pem

# macOS:
base64 -i your-app.private-key.pem | tr -d '\n'

# Windows PowerShell:
[Convert]::ToBase64String([IO.File]::ReadAllBytes("your-app.private-key.pem"))
```

Copy the entire one-line output (it starts with `LS0tLS1CRUdJTi...`, which is
`-----BEGIN` encoded) into the variable — no quotes, no line breaks. Sanity check:
decoding it must reproduce the PEM exactly:

```bash
echo "$GITHUB_APP_PRIVATE_KEY_B64" | base64 -d | head -1
# -----BEGIN RSA PRIVATE KEY-----
```

Notes:

- `GITHUB_APP_PRIVATE_KEY_B64` replaces `GITHUB_APP_PRIVATE_KEY_PATH` from local dev —
  in a container there's no PEM file on disk, and Dokploy env vars are the cleanest
  secure transport. The backend accepts either.
- `FRONTEND_URL` drives strict CORS and the browser-WebSocket `Origin` allow-list. If
  the login redirect or the live pipeline stream misbehaves later, this value being
  wrong is the first thing to check.
- Changing environment variables in Dokploy requires a **redeploy** to take effect.

## 6. Domain & HTTPS

Application → **Domains** tab → **Add Domain**:

| Field | Value |
| --- | --- |
| Host | `api.example.com` |
| Path | `/` |
| **Container Port** | **8080** |
| HTTPS | **Enabled** |
| Certificate | **Let's Encrypt** |

Preconditions: the DNS `A` record must already resolve to the VPS, and ports 80/443
must be reachable (Let's Encrypt HTTP-01 validation). Traefik (bundled with Dokploy)
terminates TLS and proxies to the container — including WebSocket upgrades for
`/runner/ws` and `/ws/...` (no extra configuration needed; Traefik proxies WS natively).

Do **not** additionally publish port 8080 in the application's port mappings — the only
public entry point should be Traefik on 443.

### 6.1 No domain yet? Use a free DuckDNS subdomain

If Dokploy shows **"Domain resolves to `<some other IP>` but should point to
`<your VPS IP>`"**, the hostname you entered does not point at your VPS — Traefik
routes by hostname, so traffic never reaches you. Turning the certificate off does
not work around it; the fix is always at the DNS level. If you don't own a domain,
[DuckDNS](https://www.duckdns.org) hosts a free subdomain for you (this is the
tested, recommended path):

1. Go to **duckdns.org**, sign in (GitHub login works).
2. Create a subdomain, e.g. `overup-api`, and set its IP to your VPS public IP
   (e.g. `213.136.94.217`). DuckDNS publishes the `A` record for you — nothing to
   configure on the VPS.
3. In Dokploy → Domains: Host `overup-api.duckdns.org`, Path `/`, Container Port
   `8080`, **HTTPS enabled**, Certificate **Let's Encrypt**. Ports 80 + 443 must be
   open (Let's Encrypt validates over port 80).

   > **Firewall check — do this before blaming anything else.** From a machine
   > that is *not* the VPS, run `curl -v --max-time 10 http://<your-domain>/`. If
   > it *times out* (rather than getting any HTTP response), ports 80/443 are
   > blocked — usually the VPS provider's edge firewall (open them in the provider
   > panel) and/or `ufw` on the VPS (`ufw allow 80/tcp && ufw allow 443/tcp`).
   > While they're blocked, Let's Encrypt cannot issue, Traefik falls back to its
   > self-signed default certificate, browsers warn, and runners refuse to connect
   > with `invalid peer certificate: UnknownIssuer`. Testing from the VPS itself
   > proves nothing — local traffic often bypasses the edge firewall.
4. Use `https://overup-api.duckdns.org` everywhere this guide says
   `https://api.example.com` — the env vars in §5 and the GitHub App URLs in §7.
   Keep `COOKIE_SECURE=true`; DuckDNS + Let's Encrypt is real HTTPS.

Quick-and-dirty alternative (testing only): IP-embedded domains like
`api.213-136-94-217.sslip.io` or `api-213-136-94-217.traefik.me` resolve to the
embedded IP with zero signup — but Let's Encrypt usually rate-limits these shared
domains, leaving you on plain HTTP, which forces `COOKIE_SECURE=false` and `http://`
callbacks. Fine for a smoke test, not for real use. A paid domain later is a drop-in
replacement: update the Dokploy domain, the §5 env vars, and the §7 GitHub URLs, then
redeploy.

## 7. Point the GitHub Apps at production

Both apps were registered with `localhost` URLs during development. Update them (or
register separate production apps — recommended so local dev keeps working):

All URLs below use your real backend domain — whatever you configured in §6. With
the DuckDNS setup from §6.1 that means replacing `https://api.example.com` with
`https://overup-api.duckdns.org` everywhere (shown in parentheses).

**OAuth App** (GitHub → Settings → Developer settings → OAuth Apps):

- Homepage URL: `https://app.example.com` (or your frontend's future domain; any
  placeholder is fine until it's deployed — GitHub doesn't validate it)
- Authorization callback URL: `https://api.example.com/auth/github/callback`
  (DuckDNS: `https://overup-api.duckdns.org/auth/github/callback`)
  — must match `OAUTH_REDIRECT_URL` **exactly**, scheme and host included (the
  backend enforces an allow-list of one).

**GitHub App** (GitHub → Settings → Developer settings → GitHub Apps):

- Setup URL: `https://api.example.com/auth/github/app/setup`
  (DuckDNS: `https://overup-api.duckdns.org/auth/github/app/setup`) — keep
  "Redirect on update" checked
- **Webhook URL: `https://api.example.com/webhooks/github`**
  (DuckDNS: `https://overup-api.duckdns.org/webhooks/github`) — production is publicly
  reachable, so the smee.io/cloudflared tunnel from local dev is no longer needed.
- Webhook secret: the exact `GITHUB_WEBHOOK_SECRET` value you set in §5.
- Permissions stay least-privilege: Metadata (read) + Contents (read).
- Events: Push, Repository, Installation target, Pull request.

If you registered fresh production apps, remember the env vars in §5 must carry the
**production** app's client id/secret/key/slug — not the dev app's.

## 8. First deploy & verification

1. Press **Deploy** on the application. Watch the build logs:
   - First build compiles all Rust dependencies (several minutes). Later builds reuse
     the cached dependency layer and are much faster.
2. Watch the runtime logs after start. You should see migration output followed by the
   server binding on `0.0.0.0:8080`. A crash loop at this point is almost always a bad
   `DATABASE_URL` or a missing required env var — the error line names the variable.
3. Health check from anywhere:

   ```bash
   curl -i https://api.example.com/healthz
   # HTTP/2 200
   ```

   (The container also self-reports health: Dokploy shows it healthy/unhealthy via the
   image's built-in `HEALTHCHECK` against `/healthz`.)

4. Auth round-trip: open `https://api.example.com/auth/github/login` in a browser — you
   should land on GitHub's authorize screen, and after authorizing be redirected to
   `FRONTEND_URL/auth/callback?...`. (Until the frontend is deployed that page 404s —
   the redirect happening at all proves OAuth, sessions, and cookies work.)
5. Webhook check: GitHub App → Advanced → Recent Deliveries — push to a synced repo and
   confirm the delivery got a `2xx` from `https://api.example.com/webhooks/github`.

## 9. Alternative: raw Docker / Docker Compose

If you prefer to run the container yourself (or as a Dokploy **Compose** service
instead of an Application), the same image works everywhere.

### 9.1 Build the image

From the **repository root** (the context must include both `backend/` and `protocol/`):

```bash
docker build -f backend/Dockerfile -t overup-backend:latest .
```

The Dockerfile is multi-stage: a `rust:1-slim-bookworm` builder with a cached
dependency layer, then a minimal `debian:bookworm-slim` runtime that runs as a
dedicated **non-root** user (`overup`, uid 10001) with only `ca-certificates` (+ `curl`
for the health check) installed.

### 9.2 Run it with docker run

Put the §5 environment block in a file (e.g. `/opt/overup/backend.env`, `chmod 600`,
owned by root) and run:

```bash
docker network create overup-net 2>/dev/null || true

docker run -d \
  --name overup-backend \
  --env-file /opt/overup/backend.env \
  --network overup-net \
  --restart unless-stopped \
  --read-only --tmpfs /tmp \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  -p 127.0.0.1:8080:8080 \
  overup-backend:latest
```

- `-p 127.0.0.1:8080:8080` binds only to localhost — your reverse proxy (Traefik,
  Caddy, nginx) on the same host terminates TLS on 443 and proxies to
  `127.0.0.1:8080`. Never publish `0.0.0.0:8080` directly.
- If Postgres runs as a container on the same host, attach it to `overup-net` and use
  its container name in `DATABASE_URL`.
- `--read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges` are safe
  for this binary (it writes nothing to disk) and shrink the attack surface.

### 9.3 Or Docker Compose (also usable as a Dokploy "Compose" service)

```yaml
# compose.prod.yml — backend only
services:
  overup-backend:
    build:
      context: .                  # repo root — protocol/ must be in context
      dockerfile: backend/Dockerfile
    # or, if you push the image to a registry instead of building in place:
    # image: ghcr.io/botcoder254/overup-backend:latest
    env_file:
      - /opt/overup/backend.env   # NOT committed; see §5 for contents
    restart: unless-stopped
    read_only: true
    tmpfs:
      - /tmp
    cap_drop:
      - ALL
    security_opt:
      - no-new-privileges:true
    ports:
      - "127.0.0.1:8080:8080"     # remove entirely when Traefik shares the network
    networks:
      - dokploy-network            # join Dokploy's network to reach its Postgres

networks:
  dokploy-network:
    external: true
```

Using this as a Dokploy **Compose** service: create a Compose service from the same
GitHub repo, point it at this compose file, remove the `ports:` block, keep
`dokploy-network`, and attach the domain in Dokploy the same way as §6 (Traefik reaches
the container over the shared network). Environment values can then live in Dokploy's
Environment tab instead of an `env_file`.

## 10. Security hardening checklist

Most protections are already enforced **in the code** (see the security checklist in
`CLAUDE.md`); this list is what the *deployment* must get right:

- [ ] **`COOKIE_SECURE=true`** — non-negotiable behind HTTPS. Enables the `Secure`
      flag, the `__Host-` cookie prefix, and HSTS.
- [ ] **HTTPS only**: the sole public entry is Traefik on 80/443 (80 only redirects /
      answers ACME). No public port mapping for 8080, no public 5432.
- [ ] **Postgres stays internal**: no "External Port" on the Dokploy Postgres service.
      Strong unique DB password; a dedicated `overup` role, not the superuser.
- [ ] **Secrets live only in Dokploy's Environment tab** (or a root-owned `chmod 600`
      env file for raw Docker). Never commit `.env`, the GitHub App `.pem`, client
      secrets, `RUNNER_JOB_SIGNING_KEY`, or `GITHUB_WEBHOOK_SECRET`. This repo's
      `.gitignore` and `.dockerignore` both exclude `.env*` and `*.pem` as a backstop.
- [ ] **Distinct secrets**: `GITHUB_WEBHOOK_SECRET` ≠ `RUNNER_JOB_SIGNING_KEY` ≠
      anything else; each ≥ 32 random bytes.
- [ ] **Firewall (ufw or provider firewall)**: allow 22 (ideally IP-restricted), 80,
      443. Restrict the Dokploy panel port (default 3000) to your IP or put it behind
      its own authenticated domain. Everything else closed.
- [ ] **Non-root container**: the image already runs as uid 10001 with a
      `no-new-privileges`-friendly, read-only-rootfs-compatible binary — keep the
      `--read-only --cap-drop ALL` options if you run it by hand.
- [ ] **One replica** of the backend (in-process scheduler/janitor singletons).
- [ ] **Backups**: enable scheduled backups on the Dokploy Postgres service (it
      supports S3-compatible destinations — R2 works). The database is the only state;
      artifacts/log archives already live in R2.
- [ ] **Key rotation**: rotating the GitHub App private key = generate a new key in
      GitHub, update `GITHUB_APP_PRIVATE_KEY_B64`, redeploy, then revoke the old key.
      Rotating `RUNNER_JOB_SIGNING_KEY` requires restarting every runner with the new
      value at the same time.
- [ ] **Logs**: `RUST_LOG=info,backend=info` is safe — the backend never logs tokens,
      cookies, or webhook bodies by design. Avoid `trace` levels for third-party crates
      in production.
- [ ] **Updates**: keep Dokploy and the VPS packages updated; rebuilds pick up patched
      base images (`Rebuild` = no-cache build).

## 11. Troubleshooting

| Symptom | Likely cause / fix |
| --- | --- |
| Dokploy: "Domain resolves to `X` but should point to `<VPS IP>`" | The hostname doesn't point at your VPS — a DNS problem, not a certificate one. Own domain: fix its `A` record. No domain: use a free DuckDNS subdomain, [§6.1](#61-no-domain-yet-use-a-free-duckdns-subdomain). |
| The domain **times out** from outside (browser spins, `curl` hits `--max-time`) | Ports 80/443 are blocked by the VPS provider's firewall and/or `ufw` — see the firewall check in [§6.1](#61-no-domain-yet-use-a-free-duckdns-subdomain). DNS resolving correctly while connections time out is the signature of a firewall, not a DNS or app problem. |
| Browser warns about the certificate / runners report `UnknownIssuer` | Traefik is serving its **default self-signed cert**: the domain's Certificate is "none", or Let's Encrypt issuance failed (port 80 closed, DNS not propagated). Set Certificate = Let's Encrypt and clear the firewall first. |
| `Error: GITHUB_APP_PRIVATE_KEY_B64 is not valid base64` at startup | The raw PEM text was pasted into the variable. It must be the **base64 of the .pem file** as one line — conversion commands in [§5.1](#51-converting-the-github-app-private-key-to-github_app_private_key_b64). |
| Build fails: `failed to load manifest for dependency 'protocol'` or `../protocol not found` | **Docker Context Path is wrong.** It must be `.` (repo root) with Docker File `backend/Dockerfile` — the build context has to contain both crates. |
| Build fails on `sqlx::migrate!` | `backend/migrations/` missing from the context — check `.dockerignore` wasn't edited to exclude it. |
| Container starts then exits immediately | Read the runtime log's first error line: usually a missing/invalid env var (the config loader names it) or an unreachable `DATABASE_URL`. |
| `502 Bad Gateway` on the domain | Domain's **Container Port** isn't 8080, the app is still starting (first migration run), or the container is crash-looping. |
| Health check keeps failing but logs look fine | Confirm `BIND_ADDR=0.0.0.0:8080` (binding `127.0.0.1` inside the container breaks both Traefik and the healthcheck). |
| Login loop / cookie never set | `COOKIE_SECURE=true` while testing over plain HTTP (the `__Host-` cookie requires HTTPS), or `FRONTEND_URL` doesn't match the real frontend origin. |
| GitHub redirects to an error after authorize | `OAUTH_REDIRECT_URL` ≠ the callback URL registered on the OAuth App (must match exactly, scheme included). |
| Webhook deliveries show `401` | `GITHUB_WEBHOOK_SECRET` doesn't match the secret configured on the GitHub App. |
| Webhook deliveries show `404` | Wrong Webhook URL — it's `/webhooks/github`, not under `/api`. |
| `database connection` errors at boot | Wrong internal hostname (use the Dokploy service's internal host, not `localhost`), or the app and Postgres aren't on the same Docker network. |
| Pipelines stay **Queued** forever | Expected until a runner is connected — runners are a separate deployment: see [deploy-runner.md](./deploy-runner.md). Once one is online, also check its labels cover the workflow's `runs-on`. |
| Frontend can't call the API (CORS errors) | `FRONTEND_URL` must be the exact origin serving the React app (scheme + host, no trailing slash). |
