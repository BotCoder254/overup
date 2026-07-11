# Overup documentation

This folder holds the operational documentation for Overup — the self-hosted CI/CD
platform (React + TypeScript control-plane UI, Rust/axum API, PostgreSQL, reference
Docker runner).

## Guides

| Document | What it covers |
| --- | --- |
| [deploy-backend-dokploy.md](./deploy-backend-dokploy.md) | Deploying **only the Rust backend** to a VPS running [Dokploy](https://dokploy.com): application setup (provider, branch, build path, trigger type), Dockerfile build, environment variables, domain + HTTPS, GitHub App/OAuth production configuration, a raw Docker / Docker Compose alternative, and a security hardening checklist. |
| [deploy-runner.md](./deploy-runner.md) | Deploying the **reference runner** (`runner/`) that executes pipelines: registering a runner + one-time token, the full environment reference (`runner/.env.example`), Docker deployment (socket + shared work-dir mounts), running it as a second Dokploy Application, bare-binary systemd setup, remote TLS Docker daemons, security notes, and troubleshooting. |
| [deploy-frontend-netlify.md](./deploy-frontend-netlify.md) | Deploying the **React frontend** to Netlify: proxy rewrites that keep the session cookie first-party (`netlify.toml`), CLI setup and deploys, backend + GitHub URL changes for the Netlify origin, the WebSocket/polling limitation, and troubleshooting. |
| [fix-hosted-runner-docker-socket.md](./fix-hosted-runner-docker-socket.md) | **Runbook** for the `hosted-runner provisioner: docker unreachable` warning on a Dokploy-deployed backend: reading the `attempts=` line to identify the stage, the socket bind mount, the host ACL for uid 10001 (`setfacl` + systemd persistence), the follow-up `error from registry: denied` pre-pull fix, and end-to-end verification. |
| [publish-runner-image.md](./publish-runner-image.md) | Building and publishing the **runner Docker image** (`runner/Dockerfile`) that `RUNNER_IMAGE` points at: repo-root build context rule, building from WSL, GHCR PAT + push + the required public-visibility flip, anonymous-pull verification, updating the image, the build-on-VPS no-registry alternative, and troubleshooting. |

## Planned documents

These will be added as the corresponding features/phases ship:

- **Same-origin frontend hosting** — serving the built React app behind the same
  reverse proxy as the API on the VPS (single origin; enables live WebSocket
  streaming, which Netlify's proxy cannot forward).
- **Backup & restore** — PostgreSQL backup strategy and R2 bucket lifecycle.
- **Operations** — monitoring the scheduler/janitor, log retention, artifact
  retention, and troubleshooting stuck pipelines.

## Conventions

- Guides assume the repository lives at `github.com/BotCoder254/overup` and use
  `api.example.com` (backend) / `app.example.com` (frontend) as placeholder domains —
  substitute your own everywhere they appear.
- Anything secret (client secrets, webhook secrets, signing keys, private keys,
  database passwords) is shown as `<placeholder>` and must never be committed.
