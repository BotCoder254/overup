# Overup documentation

This folder holds the operational documentation for Overup — the self-hosted CI/CD
platform (React + TypeScript control-plane UI, Rust/axum API, PostgreSQL, reference
Docker runner).

## Guides

| Document | What it covers |
| --- | --- |
| [deploy-backend-dokploy.md](./deploy-backend-dokploy.md) | Deploying **only the Rust backend** to a VPS running [Dokploy](https://dokploy.com): application setup (provider, branch, build path, trigger type), Dockerfile build, environment variables, domain + HTTPS, GitHub App/OAuth production configuration, a raw Docker / Docker Compose alternative, and a security hardening checklist. |

## Planned documents

These will be added as the corresponding features/phases ship:

- **Frontend deployment** — building the React app and serving it behind the same
  reverse proxy as the API (single origin, `REACT_APP_API_ORIGIN` empty).
- **Runner deployment** — running the reference Docker runner (`runner/`) on one or
  more machines, registration tokens, labels, and hardening knobs.
- **Backup & restore** — PostgreSQL backup strategy and R2 bucket lifecycle.
- **Operations** — monitoring the scheduler/janitor, log retention, artifact
  retention, and troubleshooting stuck pipelines.

## Conventions

- Guides assume the repository lives at `github.com/BotCoder254/overup` and use
  `api.example.com` (backend) / `app.example.com` (frontend) as placeholder domains —
  substitute your own everywhere they appear.
- Anything secret (client secrets, webhook secrets, signing keys, private keys,
  database passwords) is shown as `<placeholder>` and must never be committed.
