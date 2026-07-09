# overup

A modern CI/CD automation platform. Users sign in with GitHub, provision a workspace, and connect repositories, runners, and pipelines inside it. The workspace is the root security and ownership boundary: every future resource (repository, runner, workflow, secret, environment, artifact, pipeline, audit record, permission) belongs to exactly one workspace.

## Architecture

The project follows a Backend-for-Frontend model: the browser is responsible only for presentation and user interaction, while all trust decisions live in the backend.

| Layer    | Technology                                                    | Responsibility                                                        |
| -------- | ------------------------------------------------------------- | --------------------------------------------------------------------- |
| Frontend | React 19, TypeScript, react-router 7, TanStack Query, Tailwind | Presentation, optimistic UX, client-side validation for usability only |
| Backend  | Rust, axum 0.8, SQLx 0.8, tower middleware                     | Authentication, authorization, validation, provisioning, audit         |
| Database | PostgreSQL                                                     | Source of truth; migrations run automatically at backend startup       |

The frontend talks to the backend through relative `/api` and `/auth` paths (a dev proxy forwards to `localhost:8080`). Authentication state lives exclusively in an HttpOnly session cookie; no tokens are ever exposed to JavaScript.

## Security model

- GitHub OAuth 2.0 Authorization Code flow with PKCE. The `state` value is stored hashed and is single-use with a 10-minute TTL; the code exchange happens server-to-server.
- Server-side sessions: 32-byte random tokens generated from the OS RNG, stored only as SHA-256 digests, delivered as HttpOnly, SameSite=Lax cookies (Secure in production). Sessions rotate on login and expired rows are purged by a background janitor.
- Users are identified internally by immutable UUIDs keyed to the immutable GitHub user id, never by mutable usernames or emails.
- CSRF defense in depth: SameSite cookies plus a mandatory `X-Requested-With: XMLHttpRequest` header on every state-changing request, enforced by middleware.
- Security headers on every response: strict Content-Security-Policy, X-Content-Type-Options, Referrer-Policy, X-Frame-Options, COOP/CORP, and HSTS when running over HTTPS.
- Input handling follows OWASP guidance: Unicode normalization before validation, character-category allow-lists (never deny-lists), strict length bounds, and outright rejection of invalid input. All validation is centralized server-side; client-side checks exist only for usability.
- Workspace slugs are generated exclusively on the server (transliteration, reserved-word rejection, deterministic collision suffixes under a unique index). Slugs are routing values only; authorization always uses UUIDs.
- Workspace provisioning is a single atomic SQLx transaction (workspace, owner membership, default roles and permission mappings, settings, audit entry, onboarding completion) using parameterized queries throughout. Partial provisioning cannot exist.
- Per-IP rate limiting on the authentication surface and the API, with a stricter budget on provisioning. Request bodies are capped at 64 KB.
- Error responses never leak internals: clients receive stable machine-readable codes and generic messages; full detail stays in structured server logs correlated by request id.

## Getting started

### Prerequisites

- Node.js 18 or later
- Rust (stable) with cargo
- PostgreSQL 14 or later, or Docker (a `docker-compose.yml` is provided)

### Backend

1. Start PostgreSQL, for example: `docker-compose up -d`
2. Copy the environment template and fill it in:

   ```
   cd backend
   copy .env.example .env
   ```

   Required values: `DATABASE_URL`, `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`. Register a GitHub OAuth App with callback URL `http://localhost:8080/auth/github/callback`. See `backend/.env.example` for the full variable reference.

3. Run the API (migrations apply automatically on startup):

   ```
   cargo run
   ```

   The server listens on `0.0.0.0:8080` by default.

### Frontend

```
npm install
npm start
```

The app is served at `http://localhost:3000` and proxies API calls to the backend.

## Project structure

```
backend/
  migrations/        SQL migrations (run automatically at startup)
  src/
    config.rs        Environment configuration
    error.rs         Central error type; leak-free HTTP mapping
    db/              Data access (users, sessions, workspaces, oauth states)
    handlers/        HTTP handlers (auth, me, workspaces, health)
    middleware/      Session auth, CSRF, security headers
    models/          Row types and response DTOs
    routes/          Router assembly, CORS, rate limiting, layering
    services/        OAuth flow, sessions, workspace validation and slugs
src/
  app/               Router, guards, providers
  components/        Reusable UI (Button, Input, FormField, Logo, ...)
  features/
    auth/            Sign-in, OAuth callback, onboarding, shared layout
    workspace/       Workspace creation API, schema, hooks, components
    dashboard/       Workspace dashboard
  lib/               HTTP client, slug preview, query client, utilities
  types/             Shared TypeScript types
```

## Testing

- Backend: `cd backend && cargo test` (validation and slug-generation unit tests) and `cargo check`.
- Frontend: `npm test` (CRA test runner) and `npx tsc --noEmit` for type checking.
- Production build: `npm run build`.

## Environment variables

All backend configuration is documented inline in `backend/.env.example`. Secrets are never committed; `backend/.env` is gitignored.

---

Copyright overup. All rights reserved.
