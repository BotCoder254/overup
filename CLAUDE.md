# overup — self-hosted CI/CD platform

Overup is a self-hosted CI/CD platform: a React + TypeScript control-plane UI backed by a
Rust (axum) API with PostgreSQL. This phase ships the production-grade skeleton and the
complete GitHub-OAuth authentication subsystem; pipelines, runners, and artifacts build on
top of this foundation.

## Architecture

**Backend-for-Frontend (BFF) authentication.** React is a pure presentation layer — it never
sees OAuth tokens, client secrets, or session identifiers. The full flow:

1. "Continue with GitHub" does a full-page navigation to `GET {API}/auth/github/login`.
2. The Rust backend creates a PKCE challenge + random `state`, stores the transaction
   (hashed state → verifier, 10-minute TTL, single-use) in `oauth_states`, and 302s to GitHub.
3. GitHub redirects to `GET /auth/github/callback` (the exact URL registered with the OAuth
   App — an allow-list of one). The backend validates the state, exchanges the code
   server-to-server over rustls, fetches the GitHub profile, and upserts the user.
4. A session is created: 32 random bytes (OS RNG) → base64url cookie value; only the SHA-256
   hash is stored in `sessions`. Any pre-existing session is rotated out. The browser gets an
   `HttpOnly; Secure; SameSite=Lax` cookie and is redirected to `{FRONTEND}/auth/callback`
   (`?new=1` if onboarding is still due, `?error=auth_failed` on any failure — no detail leaks).
5. React boots by calling `GET /api/me` (cookie goes along automatically) and only ever holds
   the non-sensitive profile: id, username, display name, avatar, onboarded flag.

**Dev networking.** CRA's `"proxy": "http://localhost:8080"` forwards XHR (`/api/*`,
`/auth/logout`) to the backend. Full-page navigations are NOT proxied (CRA serves index.html
for `Accept: text/html`), so the login redirect uses the absolute `REACT_APP_API_ORIGIN`.
In production, serve the built frontend and the API behind one reverse proxy (same origin)
and set `REACT_APP_API_ORIGIN` to empty/same origin, `COOKIE_SECURE=true`.

## Repository layout

```
overup/
├── docker-compose.yml          # PostgreSQL 16 for local dev
├── .env.example                # REACT_APP_API_ORIGIN
├── tailwind.config.js          # design tokens (CommonJS — CRA requires it)
├── postcss.config.js
├── src/                        # React + TypeScript frontend (.ts/.tsx only)
│   ├── index.tsx / index.css   # entry + @tailwind directives
│   ├── app/                    # composition root
│   │   ├── App.tsx             # Providers + RouterProvider
│   │   ├── providers.tsx       # react-query, error boundary, sonner Toaster
│   │   ├── router.tsx          # route table
│   │   └── guards/             # ProtectedRoute, PublicOnlyRoute
│   ├── components/
│   │   ├── brand/              # Logo (Squirrel + wordmark), GitHubMark
│   │   ├── ui/                 # Button (cva variants), Spinner
│   │   └── layout/             # AppShell (authenticated chrome)
│   ├── features/               # feature-sliced modules
│   │   ├── auth/               # api/, hooks/, components/AuthSplitLayout, pages/
│   │   └── dashboard/          # placeholder shell; pipelines/runners/etc. slot in here
│   ├── lib/                    # api (ky), cn, env, queryClient
│   └── types/                  # shared API types (Me)
└── backend/                    # Rust control plane (axum + sqlx + PostgreSQL)
    ├── migrations/             # sqlx migrations: users, sessions, oauth_states
    └── src/
        ├── main.rs             # bootstrap: env, tracing, pool, migrate, janitor, serve
        ├── config.rs           # all env-driven configuration
        ├── state.rs            # AppState: pool, config, oauth client, http client
        ├── error.rs            # AppError → sanitized JSON responses
        ├── db/                 # parameterized sqlx queries only (users, sessions, oauth_states)
        ├── models/             # User row + MeResponse DTO
        ├── routes/             # router assembly + middleware stack
        ├── handlers/           # health, auth (login/callback/logout), me
        ├── middleware/         # security_headers, csrf, auth (CurrentUser extractor)
        └── services/           # session, github, auth_flow orchestration
```

New CI/CD features follow the same pattern: `src/features/<name>/{api,hooks,components,pages}`
on the frontend; new `db/`, `handlers/`, `services/` modules plus a migration on the backend.

## Design system (strict rules)

- **Colors — solid only, no gradients, ever.** Use only the Tailwind palette:
  `white`/`canvas` `#ffffff`, `surface` `#f6f5f4`, `primary`/`purple` `#5645d4`,
  `navy` `#0a1530`, `link` `#0075de`, `charcoal` `#37352f`, `steel` `#787671`.
  Opacity variants (e.g. `border-steel/20`, `text-white/60`) are allowed.
- **Border radius is strictly 6px.** Every `rounded*` alias maps to 6px in
  `tailwind.config.js`; `rounded-full` is reserved for avatars/spinners only.
- **Logo** is `components/brand/Logo.tsx`: the lucide `Squirrel` icon, stroke
  `currentColor`, **no background**, sizes `sm|md|lg|xl`, optional "overup" wordmark.
  It inherits the parent text color (charcoal on light panels, white on navy).
- **Auth pages** all use `features/auth/components/AuthSplitLayout.tsx`: full-height 50/50
  split, no floating card. Left = interaction panel on `canvas` (logo top-left);
  right = solid `navy` branding panel (logo top-right, large Squirrel + vision sentence
  centered, copyright bottom-right), hidden below `lg`.
- Typeface: "Pin Sans" with system fallbacks (`font-sans`/`font-display`).
- Files are `.ts`/`.tsx` only.

## Frontend dependencies

In active use: `react-router-dom` (routing), `@tanstack/react-query` (server state — owns
`/api/me`), `ky` (HTTP with `credentials: 'include'` + CSRF header), `clsx` +
`tailwind-merge` + `class-variance-authority` (styling utilities), `lucide-react` (icons),
`sonner` (toasts), `react-error-boundary`, `tailwindcss`.

Installed and reserved for upcoming features: `@tanstack/react-table`/`react-virtual`
(pipeline/run tables), `@tanstack/react-form` + `react-hook-form` + `zod` (workflow/settings
forms), `zustand` (client-only UI state, e.g. command palette), `monaco-editor` (YAML
editing), `@xterm/xterm` + addons (live log terminal), `recharts` (dashboards), `cmdk`
(command palette), `framer-motion`, `react-markdown`/`remark-gfm`/`rehype-highlight`,
`react-dropzone`, `js-yaml`/`yaml`, `date-fns`, `nanoid`, `axios` (unused; `ky` is the
standard client), and assorted hook utilities.

Note: `zod` v4 wants TypeScript 5+; the project pins TS 4.9.5 (CRA-compatible), so prefer
plain interfaces until TS is upgraded. `@tailwindcss/typography` is not installed — install
it before adding a `typography` plugin entry to the Tailwind config.

## Backend crates

`axum` (HTTP + routing), `tokio` (async runtime), `tower`/`tower-http` (CORS, trace, body
limit, request-id), `tower_governor` (per-IP rate limiting on `/auth`), `axum-extra`
(cookies), `sqlx` (PostgreSQL, migrations, parameterized queries only — never concatenate
SQL), `oauth2` v5 (Authorization Code + PKCE), `reqwest` (rustls, redirects disabled),
`serde`/`serde_json`, `uuid`, `chrono`, `time` (cookie max-age), `tracing` +
`tracing-subscriber` (structured logs — never log secrets), `thiserror`/`anyhow`
(sanitized error responses), `dotenvy`, `rand` (OS RNG session tokens), `sha2` + `hex`
(token/state hashing), `base64`, `validator` (input validation as endpoints grow).

## Security checklist (enforced in code)

- Authorization Code + **PKCE**; `state` stored hashed, single-use, 10-min TTL
- Exact redirect-URI allow-list (one registered callback URL)
- Code exchange server-to-server over TLS (rustls), HTTP redirects disabled
- Session tokens: 32 bytes OS RNG; **only SHA-256 hashes** in the database
- Session rotation on every login; absolute expiry; hourly janitor purges expired rows
- Cookie: `HttpOnly`, `Secure` (prod), `SameSite=Lax`, `Path=/`
- CSRF defense-in-depth: state-changing requests require `X-Requested-With: XMLHttpRequest`
- Security headers on every response: CSP `default-src 'none'; frame-ancestors 'none'`,
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `X-Frame-Options: DENY`, `Cache-Control: no-store`, `Permissions-Policy`,
  `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Resource-Policy: same-origin`,
  plus `Strict-Transport-Security` when `COOKIE_SECURE=true` (HTTPS deployments)
- Callback input validation: `code`/`state` parameters are length-capped (512) before any
  processing; oversized values are rejected with the sanitized failure redirect
- Strict CORS: frontend origin only, credentials allowed, minimal methods/headers
- Per-IP rate limiting on `/auth/*`; 64 KB request body limit
- Errors are sanitized: clients get stable codes, details stay in tracing logs
- Every `/api/*` endpoint authenticates via the `CurrentUser` extractor (401 without a
  live session); authorization checks come with workspaces in the next phase

## Running locally

```bash
# 1. Database — set DATABASE_URL in backend/.env to your own PostgreSQL
#    (hosted providers work; append ?sslmode=require for TLS via rustls).
#    Optional local alternative: docker compose up -d
#    (postgres://overup:overup@localhost:5432/overup)

# 2. Backend — copy backend/.env.example to backend/.env and fill in
#    DATABASE_URL plus GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET (OAuth App
#    with callback http://localhost:8080/auth/github/callback).
#    Migrations run on startup.
cd backend && cargo run

# 3. Frontend (http://localhost:3000)
npm start
```

GitHub OAuth App registration: GitHub → Settings → Developer settings → OAuth Apps →
New OAuth App. Homepage `http://localhost:3000`, callback
`http://localhost:8080/auth/github/callback`. Never commit the client secret.

Verification: `cargo check` and `cargo clippy -- -D warnings` in `backend/`;
`npm run build` at the root. Both are clean as of this phase.
