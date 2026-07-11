# overup — self-hosted CI/CD platform

Overup is a self-hosted CI/CD platform: a React + TypeScript control-plane UI backed by a
Rust (axum) API with PostgreSQL. Shipped so far: the production-grade skeleton, the complete
GitHub-OAuth authentication subsystem, the **Repository + Workflow Management modules**
(GitHub App integration, webhook-driven sync, workflow YAML parsing/validation, Monaco
workspace with dependency graph), and the **Pipeline Execution subsystem** (event-driven
scheduler, HMAC-signed runner WebSocket protocol, live log streaming, Cloudflare R2
artifacts, a reference Docker runner in `runner/`, and the full Pipelines UI). Runner
Management UI, secrets, matrix expansion, and PR/cron triggers build on this foundation.

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

**GitHub App integration (repositories + workflows).** All repository access goes through a
GitHub App — never user PATs or the OAuth token (which is still discarded after profile
fetch). The backend signs a short-lived RS256 JWT (`iss` = app client ID, `exp` ≤ 10 min)
and exchanges it for **installation access tokens** (1-hour, scoped to
`contents:read + metadata:read`), cached in-memory in `AppState.github_app` and never
logged/persisted. Installation discovery uses the App's **Setup URL**
(`GET /auth/github/app/setup?installation_id=…`): the handler authenticates the browser
session, then verifies the untrusted id server-side (`GET /app/installations/{id}` with the
app JWT must succeed AND the account login must match the user — or an
`installation.created` webhook recorded them as installer) before linking it to the
workspace. `POST /webhooks/github` (outside `/api`, no CSRF/CORS) authenticates every
delivery with constant-time HMAC-SHA256 over the raw body, dedupes on `X-GitHub-Delivery`,
and drives incremental sync on push/repository/installation events. Sync fetches repo
metadata, branches, and `.github/workflows` via the Contents API (no cloning, no git2),
diffs blob shas, parses YAML with `services/workflow_parse.rs` (node budget 20k, depth 32,
512 KB cap, ≤100 jobs — parse errors become diagnostics, never 500s; note YAML 1.1 parses
the bare `on:` key as boolean `true`), and upserts normalized metadata in one transaction.
Authorization: every workspace-scoped endpoint runs `services/authz.rs::require_permission`
(joins `workspace_members → workspace_roles → role_permissions`; reads need `content.read`,
mutations `content.write`). The workflow editor is **read-only this phase**: Monaco +
debounced `POST …/workflows/validate` diagnostics; write-back to GitHub (needs
`workflows:write`) is a follow-up.

**Pipeline Execution (pipelines + runners + artifacts).** Event-driven orchestration around
immutable execution records. Triggers (push webhooks for any branch with a real head SHA;
manual dispatch/rerun from the UI) call `services/pipeline_run::create_pipeline`, which
translates the stored workflow via `services/pipeline_plan.rs` (re-uses `workflow_parse`
budgets; snapshots `{image, env, steps, notices}` per job into `pipeline_jobs.plan` JSONB —
`uses:` steps and matrix expansion are recorded as notices, not executed) and persists
pipeline + jobs + `pipeline.created` ledger entry + audit row in one transaction with a
race-free per-repo number from `pipeline_counters`. Status model is GitHub-style: coarse
`status (queued|in_progress|completed)` + `conclusion` (pipeline: success|failure|cancelled|
timed_out|partial; job: + skipped) enforced by CHECK constraints, plus a fine-grained job
`stage` for telemetry, plus the append-only `pipeline_events` ledger for every transition.
The `Notify`-driven scheduler (`services/scheduler.rs`, spawned in main) matches eligible
jobs (queued, all `needs` concluded success — failed deps eagerly skip dependents
transitively) to connected idle runners by label containment (`runs_on ⊆ labels`), claims
job+runner atomically in one transaction (guarded conditional UPDATEs — the repo_sync
pattern), and pushes an **HMAC-SHA256-signed job payload** (signature over the exact JSON
string; embeds runner_id + 5-min expiry; verified constant-time runner-side before parsing)
through `services/runner_hub.rs`. Payloads carry a 1-hour `contents:read` tarball token
(registered as a log mask before dispatch). Sweeps make every non-terminal state
self-healing: 15 s ack-timeout reverts, job/pipeline timeout kills, stale-runner orphaning
(attempt < 2 requeues, else `runner_lost`), and boot-time orphan recovery; a checkout-token
mint failure fails the job with static `checkout_unavailable` rather than running without
source. Runners (`runner/` crate) hold one outbound WS (`/runner/ws`, Bearer token =
SHA-256-hashed registration token shown once; heartbeat cadence comes from `hello_ack`),
connect to Docker via `connect_with_defaults` (local socket/pipe OR remote daemon via
`DOCKER_HOST` + `DOCKER_TLS_VERIFY` + `DOCKER_CERT_PATH`, rustls), execute each job in a
hardened container via bollard (temp workspace bind-mounted at /workspace, tarball checkout
with traversal-safe extraction that also drops symlink/hardlink entries, one keep-alive
container with `no-new-privileges` always plus cap-drop ALL by default, memory/CPU/pids
limits, bridge|none|isolated per-job network, optional non-root user + read-only rootfs,
steps as `exec` with `<shell> -c`, SIGKILL on cancel/timeout, 5 s Docker stats sampling for
CPU/memory/net/blkio metrics reported with `job_result`), and stream seq-numbered log
chunks. `services/log_hub.rs` is the single log write path: mask FIRST (checkout tokens +
confidential-looking env values; a per-job carry buffer catches secrets split across chunk
boundaries; masks clear on a 120 s generation-guarded grace delay) → cap (8 KB/line,
64 KB/chunk, 10 MiB/job with overflow marker) → persist (`pipeline_log_chunks`, idempotent
on (job, seq)) → broadcast to browser subscribers on `/ws/workspaces/{ws}/pipelines/{p}`
(strict Origin check + session cookie + `content.read` BEFORE upgrade; snapshot then live
events with `createdAt`; server pings every 30 s and answers client `{"type":"ping"}` with
a pong; `log_gap` on lag → client backfills over REST, paging until exhausted). Artifacts
AND completed-job log archives live in **Cloudflare R2** (S3 API; artifact presigned
PUT/GET minted server-side, HeadObject verification before rows flip to `uploaded`; logs
gzip'd server-side to `logs/{ws}/{pipeline}/{job}-{attempt}.log.gz` on job completion —
feature-gated on R2 env, clean denial/Postgres-only without it; runner convention: files in
`.overup/artifacts/`). `services/janitor.rs` (hourly) expires uploaded artifacts after
`ARTIFACT_RETENTION_DAYS` (deleting the R2 object), removes stale pending artifact rows,
prunes archived log chunks past `LOG_HOT_RETENTION_DAYS` (the raw-log download then
307-redirects to a presigned R2 GET), and purges sessions/oauth states. The ledger list API
filters on repository, workflow, status, conclusion, trigger, branch, actor, runner, date
range, and escaped free-text ILIKE search, all composing with keyset pagination; validated
metrics land in `pipeline_jobs.metrics` JSONB for the Performance tab.
Frontend: `features/pipelines/` — the ledger page (URL-synced filter bar: free-text search,
combined status/conclusion state, repository, workflow, trigger, branch, date range;
IntersectionObserver-driven infinite scroll with a Load-more fallback; columns prune below
`md`), and a detail workspace (live SVG graph + timeline lanes beside synchronized
Logs/Environment/Artifacts/Metadata/Container/Performance tabs — a resizable horizontal
split at `lg`+ via `lib/useMediaQuery.ts`, a natural vertical stack below it) fed by
`usePipelineStream.ts` (patches the react-query cache, zustand log store, exponential
backoff, 25 s app-level ping + 60 s stale watchdog, paged REST backfill, polling fallback
while disconnected). The xterm `LogViewer` adds regex + case-sensitive search toggles,
per-chunk timestamp display, and copy-to-clipboard; line numbers are deliberately out of
scope (xterm has no gutter).

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
│   │   ├── router.tsx          # route table (/w/:slug is an AppShell layout route)
│   │   ├── navigation.ts       # nav single source of truth (groups → items → icons)
│   │   └── guards/             # ProtectedRoute, PublicOnlyRoute, WorkspaceRoute
│   ├── components/
│   │   ├── brand/              # Logo (Squirrel + wordmark), GitHubMark
│   │   ├── ui/                 # Button (cva), Spinner, Popover (+MenuItem), inputs,
│   │   │                       #   Badge, Card, Table, Tabs, Dialog
│   │   └── layout/             # app shell: AppShell (layout route), Sidebar, SidebarNav,
│   │                           #   WorkspaceSwitcher, UserFooter, MobileTopBar,
│   │                           #   PageHeader (breadcrumb + optional `parent` for detail
│   │                           #   pages), PlaceholderPage, CommandPalette
│   ├── features/               # feature-sliced modules
│   │   ├── auth/               # api/, hooks/, components/AuthSplitLayout, pages/
│   │   ├── workspace/          # create-workspace api/hooks/schema (onboarding)
│   │   ├── repositories/       # install callout, connected/available lists, repo detail
│   │   ├── workflows/          # catalog, Monaco workspace, SVG dependency graph, panels
│   │   ├── pipelines/          # execution ledger + live detail workspace: PipelineGraph,
│   │   │                       #   ExecutionTimeline, LogViewer (xterm), tab panels,
│   │   │                       #   usePipelineStream (WS), stores/logStore (zustand)
│   │   ├── artifacts/          # workspace artifact catalog: summary strip, URL-synced
│   │   │                       #   filters, keyset infinite scroll, provenance detail page
│   │   └── dashboard/          # dashboard page; runners/etc. slot in here
│   ├── lib/                    # api (ky), cn, env, queryClient, slug
│   └── types/                  # shared API types (Me, Workspace, Repository, Workflow,
│                               #   Pipeline + stream events)
├── protocol/                   # shared Rust crate: runner↔server WS messages, JobPayload,
│                               #   HMAC sign/verify (constant-time)
├── runner/                     # reference runner binary: outbound WS, signed-payload
│                               #   verification, Docker execution via bollard, log
│                               #   streaming, R2 artifact upload (src/{main,ws,executor,
│                               #   artifacts}.rs)
└── backend/                    # Rust control plane (axum + sqlx + PostgreSQL)
    ├── migrations/             # users, sessions, oauth_states, workspaces,
    │                           #   github_installations, repositories(+branches/sync_runs/
    │                           #   webhook_deliveries), workflows(+workflow_jobs),
    │                           #   pipelines(+runners/pipeline_jobs/pipeline_events/
    │                           #   pipeline_log_chunks/artifacts/pipeline_counters)
    └── src/
        ├── main.rs             # bootstrap: env, tracing, pool, migrate, orphan recovery,
        │                       #   scheduler spawn, janitor, serve
        ├── config.rs           # all env-driven configuration (GitHub App, signing key,
        │                       #   execution budgets, optional R2 group)
        ├── state.rs            # AppState: pool, config, oauth, http, github_app,
        │                       #   log_hub, runner_hub, scheduler, r2
        ├── error.rs            # AppError → sanitized JSON responses
        ├── db/                 # parameterized sqlx queries only, one module per table
        ├── models/             # FromRow rows + camelCase response DTOs, per resource
        ├── routes/             # router assembly + per-scope middleware (CSRF, limits);
        │                       #   WS nests (/runner, /ws) live OUTSIDE the CSRF layer
        ├── handlers/           # health, auth, me, workspaces, github_installations,
        │                       #   repositories, workflows, github_webhooks, pipelines,
        │                       #   runners, runner_ws, browser_ws
        ├── middleware/         # security_headers, csrf, auth (CurrentUser extractor)
        └── services/           # session, github, github_app (JWT + token cache),
                                #   auth_flow, workspace, authz (RBAC), repo_sync,
                                #   workflow_parse, pipeline_plan, pipeline_run (state
                                #   machine), scheduler, log_hub (mask+cap+broadcast),
                                #   runner_hub, r2 (presign + HeadObject)
```

**Authenticated app shell.** Everything under `/w/:slug` renders inside one persistent
`AppShell` layout route: a fixed light sidebar (workspace switcher popover on top, grouped
nav from `app/navigation.ts` in an independently scrollable middle, user footer with profile
menu + one-click logout at the bottom) beside a floating content canvas (`rounded border
border-steel/20 bg-canvas`, the only scrolling region). Pages render into the canvas via
`<Outlet/>` and start with `components/layout/PageHeader` (breadcrumb + title + actions).
Below `lg` the sidebar becomes an overlay drawer behind a hamburger in `MobileTopBar`.
Ctrl/Cmd+K opens the cmdk `CommandPalette` (navigation). New nav destinations are added in
`app/navigation.ts`; a real feature page replaces its `PlaceholderPage` route element.

New CI/CD features follow the same pattern: `src/features/<name>/{api,hooks,components,pages}`
on the frontend; new `db/`, `handlers/`, `services/` modules plus a migration on the backend.

## Design system (strict rules)

- **Colors — solid only, no gradients, ever.** Use only the Tailwind palette:
  `white`/`canvas` `#ffffff`, `surface` `#f6f5f4`, `primary`/`purple` `#5645d4`,
  `navy` `#0a1530`, `link` `#0075de`, `charcoal` `#37352f`, `steel` `#787671`.
  Opacity variants (e.g. `border-steel/20`, `text-white/60`) are allowed.
- **Border radius is strictly 6px.** Every `rounded*` alias maps to 6px in
  `tailwind.config.js`; `rounded-full` is reserved for avatars/spinners only.
  Exception: the sidebar footer avatar deliberately uses the 6px square treatment
  (`rounded`, not `rounded-full`) as part of the shell's geometric identity.
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
`/api/me`, repositories, workflows; polling while syncs run), `ky` (HTTP with
`credentials: 'include'` + CSRF header), `clsx` + `tailwind-merge` +
`class-variance-authority` (styling utilities), `lucide-react` (icons), `sonner` (toasts),
`react-error-boundary`, `react-hook-form` (onboarding form), `cmdk` (command palette),
`react-hotkeys-hook` (Ctrl/Cmd+K), `@monaco-editor/react` (workflow YAML editor, custom
`overup` light theme), `react-resizable-panels` (**v4 API: `Group`/`Panel`/`Separator`,
`orientation` prop, percentage sizes** — not the old PanelGroup/PanelResizeHandle),
`date-fns` (relative timestamps), `tailwindcss`. The workflow/pipeline dependency graphs are
hand-rolled SVG DAGs (`WorkflowGraph.tsx` / `PipelineGraph.tsx` share the layered Kahn
layout) — no graph library. Pipeline Execution brought several reserved deps into active
use: `@xterm/xterm` + `addon-fit`/`addon-search`/`addon-web-links` (the streaming
`LogViewer`, its own virtualized scrollback — `@tanstack/react-virtual` was NOT needed),
`recharts` (PerformancePanel queue-vs-exec chart), `zustand`
(`features/pipelines/stores/logStore.ts`, seq-deduped log buffers), and the browser
`WebSocket` API (`usePipelineStream.ts` — reconnect w/ backoff, react-query cache patching,
polling fallback).

Installed and reserved for upcoming features: `@tanstack/react-table`/`react-virtual`
(large run tables), `@tanstack/react-form` + `zod` (workflow/settings forms),
`framer-motion`, `react-markdown`/`remark-gfm`/`rehype-highlight`, `react-dropzone`,
`js-yaml`/`yaml`, `nanoid`, `axios` (unused; `ky` is the standard client), and assorted
hook utilities.

Note: the project is on TypeScript 5.9 (an earlier TS 4.9 pin is gone), so `zod` v4 is
usable when needed. `@tailwindcss/typography` is not installed — install it before adding
a `typography` plugin entry to the Tailwind config. Popovers/menus use the in-house
`components/ui/Popover.tsx` primitive (focus trap, arrow-key roving, Escape/outside-click)
rather than a headless-UI dependency.

## Backend crates

`axum` (HTTP + routing), `tokio` (async runtime), `tower`/`tower-http` (CORS, trace, body
limit, request-id), `tower_governor` (per-IP rate limiting on `/auth`), `axum-extra`
(cookies), `sqlx` (PostgreSQL, migrations, parameterized queries only — never concatenate
SQL), `oauth2` v5 (Authorization Code + PKCE), `reqwest` (rustls, redirects disabled),
`serde`/`serde_json`, `uuid`, `chrono`, `time` (cookie max-age), `tracing` +
`tracing-subscriber` (structured logs — never log secrets), `thiserror`/`anyhow`
(sanitized error responses), `dotenvy`, `rand` (OS RNG session tokens), `sha2` + `hex`
(token/state hashing), `base64`, `validator` (input validation as endpoints grow),
`jsonwebtoken` (RS256 GitHub App JWTs), `hmac` (webhook + job-payload signatures —
`verify_slice` is constant-time), `serde_yaml_ng` (maintained serde_yaml fork; workflow
parsing under strict budgets), `axum` with the **`ws` feature** (runner + browser WebSocket
upgrades), `dashmap` (RunnerHub connection registry + LogHub broadcast/mask maps),
`futures-util` (WS stream splitting), `aws-sdk-s3` (Cloudflare R2 via its S3 API — custom
endpoint, region `auto`, presigned URLs; isolated in `services/r2.rs`), and the local
`protocol` crate (shared WS message types + HMAC helpers). The `runner/` crate adds
`bollard` 0.21 (Docker Engine API: image pull, container lifecycle, exec streams),
`tokio-tungstenite` (rustls), `tar` + `flate2` (traversal-safe tarball extraction),
`tempfile` (per-job workspaces). Deliberately NOT used: `octocrab` (the reqwest helper in
`services/github_app.rs` suffices), `git2` (tarball checkout via the API instead of cloning
— heavy native dep on Windows).

## Security checklist (enforced in code)

- Authorization Code + **PKCE**; `state` stored hashed, single-use, 10-min TTL
- Exact redirect-URI allow-list (one registered callback URL)
- Code exchange server-to-server over TLS (rustls), HTTP redirects disabled
- Session tokens: 32 bytes OS RNG; **only SHA-256 hashes** in the database
- Session rotation on every login; absolute expiry; hourly janitor purges expired rows
- Cookie: `HttpOnly`, `Secure` (prod), `SameSite=Lax`, `Path=/`; with `COOKIE_SECURE=true`
  the name is auto-prefixed `__Host-` (binds the cookie to the exact host — no subdomain
  planting/fixation)
- CSRF defense-in-depth: state-changing requests require `X-Requested-With: XMLHttpRequest`
- Security headers on every response: CSP `default-src 'none'; frame-ancestors 'none'`,
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `X-Frame-Options: DENY`, `Cache-Control: no-store`, `Permissions-Policy`,
  `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Resource-Policy: same-origin`,
  `X-Permitted-Cross-Domain-Policies: none`, `X-DNS-Prefetch-Control: off`,
  plus `Strict-Transport-Security` when `COOKIE_SECURE=true` (HTTPS deployments)
- Callback input validation: `code`/`state` parameters are length-capped (512) before any
  processing; oversized values are rejected with the sanitized failure redirect
- Strict CORS: frontend origin only, credentials allowed, minimal methods/headers
  (GET/POST/DELETE)
- Per-IP rate limiting on `/auth/*`, `/api/*`, `/webhooks/*`, plus stricter budgets on
  workspace creation and manual repo sync
- **Per-scope body limits** (not global): 64 KB on `/auth` + `/api`, 1 MiB on
  `/webhooks/github` and `POST …/workflows/validate` — an outer global limit would cap
  webhook payloads, so don't reintroduce one
- Errors are sanitized: clients get stable codes, details stay in tracing logs;
  `sync_error` values are static category strings, never upstream response bodies
- Every `/api/*` endpoint authenticates via the `CurrentUser` extractor (401 without a
  live session) **and** authorizes via `services/authz.rs::require_permission` against the
  RBAC tables (`content.read` for reads, `content.write` for mutations; flat 403 — workspace
  existence never leaks)
- GitHub App: installation tokens are short-lived, permission-scoped
  (`contents:read + metadata:read`), cached in memory only — never logged, never in
  responses, never in Postgres; the private key PEM loads once at startup
- Webhooks: constant-time HMAC-SHA256 over the raw body (`X-Hub-Signature-256`) before any
  parsing; `X-GitHub-Delivery` primary key makes redeliveries no-ops; payloads are parsed
  into minimal typed envelopes and never logged
- Setup redirect: `installation_id` is length-capped, numeric-validated, then verified
  against the GitHub API with an app JWT + account/installer match before linking
- Workflow YAML parsing is deterministic and side-effect free: 512 KB cap, 20k node budget,
  depth 32, ≤100 jobs/≤100 steps caps (alias-bomb defense); owner/repo/path segments are
  allow-list validated before URL interpolation; only `.yml`/`.yaml` files directly inside
  `.github/workflows` are parsed
- Immutable audit trail in `audit_logs`: `installation.linked/unlinked`,
  `repository.imported/synced/removed`, `pipeline.created/completed/cancelled`,
  `runner.created/revoked`, `artifact.uploaded/downloaded/deleted`,
  `artifact.retention_updated` (with request ids where available)
- **WebSocket surfaces live OUTSIDE the CSRF layer** (native WS can't send
  `X-Requested-With`) and defend themselves before upgrading. Browser WS
  (`/ws/workspaces/{ws}/pipelines/{p}` and `…/{ws}/dashboard`): strict
  `Origin == FRONTEND_URL` allow-list FIRST (CORS does not protect WebSockets), then auth —
  session cookie via the `find_valid_user` path (same-origin) OR a **one-time WS ticket**
  (`?ticket=`, for deployments whose SPA proxy can't forward upgrades, e.g. Netlify: minted
  by the authenticated+RBAC'd+CSRF'd `POST /api/workspaces/{ws}/ws-ticket`, 32 OS-RNG
  bytes, hash-only in-memory storage, 60 s TTL, single-use via atomic remove, bound to
  user+workspace — `services/ws_ticket.rs`), then `content.read` RBAC re-checked on both
  paths — all pre-upgrade; 4 KB inbound frame cap; unparseable input closes
  the socket; broadcast lag emits `log_gap` (client resyncs over REST). Runner WS
  (`/runner/ws`): `Authorization: Bearer` token → SHA-256 hash lookup against non-revoked
  runners (tokens are 32 OS-RNG bytes shown once, only hashes stored); auth failures carry
  a static category (`invalid_token`/`bootstrap_expired`/`missing_token`) the runner logs
  with remediation guidance; 128 KB frame cap; dedicated governors on both endpoints
- **Job payload integrity:** every `job_assign` is HMAC-SHA256-signed over the exact
  transmitted JSON (`RUNNER_JOB_SIGNING_KEY`, ≥32 bytes); payloads embed the target
  `runner_id` and a 5-minute validity window; runners verify constant-time BEFORE parsing —
  tampered, replayed, or misdirected payloads never execute. The verification key is
  delivered to authenticated runners in `hello_ack` over wss (memory-only, re-received per
  connect; a locally pinned `RUNNER_JOB_SIGNING_KEY` takes precedence) — safe because
  runners only VERIFY with it, token auth precedes delivery, and payloads stay
  runner-bound + short-lived
- Runner-reported data is never trusted raw: job-scoped messages are validated against the
  job's actual assignment (`runner_id` match in SQL guards); `stage` values come from a
  fixed vocabulary; `error_category` is coerced onto a static allow-list; artifact names are
  allow-list validated and uploads verified via R2 HeadObject (size cap) before rows flip to
  `uploaded`
- **Log hygiene:** the single ingest path masks FIRST (checkout tokens auto-registered at
  dispatch + confidential-looking env values; a per-job carry buffer catches secrets split
  across chunk boundaries; duplicate seqs are dropped so replays can't corrupt it) and only
  THEN caps (8 KB/line, 64 KB/chunk, 10 MiB/job + overflow marker) BEFORE persisting or
  broadcasting — the cap can never bisect a secret, and the browser and the database never
  see an unmasked byte; masks are memory-only and clear on a 120 s generation-guarded grace
  delay after job end (straggler chunks stay masked; a re-dispatched job keeps fresh masks);
  confidential-looking env values are also masked in API `plan` responses
- Runner-reported metrics are clamped onto sane ceilings server-side (durations ≤ 7 days,
  bytes ≤ 1 TiB, permille ≤ 1024 cores) — over-cap values are dropped, never stored
- List filters are validated before SQL: status/conclusion/trigger allow-lists, length caps
  on branch/search/timestamps, and free-text search runs through a server-built ILIKE
  pattern with `\`, `%`, `_` escaped — user text only ever matches literally
- Execution isolation (reference runner): per-job temp workspaces (deleted after), tarball
  extraction drops non-`Normal` path components AND symlink/hardlink entries (traversal
  defense — a link target can't be validated by path checks), 1 GiB tarball cap, jobs run
  in Docker containers with `no-new-privileges` always set, **capabilities dropped by
  default** (`RUNNER_CAP_DROP=false` to opt out), memory/CPU/pids limits
  (`RUNNER_JOB_MEMORY_BYTES`/`RUNNER_JOB_NANO_CPUS`/`RUNNER_JOB_PIDS_LIMIT`, 0 disables),
  configurable network (`RUNNER_JOB_NETWORK=bridge|none|isolated` — isolated creates a
  throwaway per-job bridge network removed on every exit path), optional non-root
  `RUNNER_JOB_USER` and `RUNNER_JOB_READONLY_ROOTFS` (tmpfs /tmp), SIGKILL on
  cancel/timeout, containers force-removed afterwards
- Remote Docker daemons are TLS-only in practice: the runner honors `DOCKER_HOST` with
  `DOCKER_TLS_VERIFY=1` + `DOCKER_CERT_PATH` (rustls; bollard `ssl` feature) — never expose
  an unauthenticated tcp://2375 daemon
- Scheduler self-healing: guarded conditional UPDATEs everywhere (no lost/duplicated
  transitions), atomic two-row job+runner claims, 15 s ack-timeout reverts, job/pipeline
  timeout sweeps, stale-runner orphaning, boot-time orphan recovery; checkout-token mint
  failures fail the job (`checkout_unavailable`) instead of running without source
- Storage hygiene: artifacts carry an immutable `expires_at` computed at upload from
  per-kind workspace retention policies (`artifact_retention_policies`, 1–400 days,
  kind row → `default` row → `ARTIFACT_RETENTION_DAYS` env); the hourly janitor deletes
  expired/abandoned R2 objects and rows, and prunes archived log chunks only when the R2
  archive exists (`LOG_HOT_RETENTION_DAYS`). Artifact `kind` is classified SERVER-side
  from the validated name (`services/artifact_kind.rs`); runner-reported archive manifests
  (entries/uncompressed size/file count) are capped (1000 entries, 96 KB JSON, 1 TiB/1M
  ceilings) and dropped whole on any violation — the upload itself still succeeds
- Pipeline conclusions and `sync_error`-style fields hold static category strings only
  (`runner_lost`, `timeout`, `step_failed`, `checkout_unavailable`, ...) — never
  upstream/runner text

## Running locally

```bash
# 1. Database — set DATABASE_URL in backend/.env to your own PostgreSQL
#    (hosted providers work; append ?sslmode=require for TLS via rustls).
#    Optional local alternative: docker compose up -d
#    (postgres://overup:overup@localhost:5432/overup)

# 2. Backend — copy backend/.env.example to backend/.env and fill in
#    DATABASE_URL plus GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET (OAuth App
#    with callback http://localhost:8080/auth/github/callback) plus
#    RUNNER_JOB_SIGNING_KEY (openssl rand -hex 32). R2_* vars are optional —
#    without them pipelines run but artifact uploads are denied and logs
#    stay in Postgres (no archival/pruning). Retention knobs:
#    ARTIFACT_RETENTION_DAYS / ARTIFACT_PENDING_TTL_HOURS /
#    LOG_HOT_RETENTION_DAYS — per-kind artifact retention (1–400 days) is
#    also configurable per workspace in the Artifacts UI and takes
#    precedence at upload time. Migrations run on startup.
cd backend && cargo run

# 3. Frontend (http://localhost:3000)
npm start

# 4. Runner (optional — pipelines stay Queued without one). Requires Docker.
#    Register via the Runners page wizard (or POST …/runners/bootstrap) and
#    run the generated command — the signing key is delivered automatically
#    over the authenticated socket (RUNNER_JOB_SIGNING_KEY is optional and
#    only pins it locally). Labels must cover the workflows' runs-on values
#    (e.g. ubuntu-latest).
cd runner
OVERUP_URL=http://localhost:8080 RUNNER_TOKEN=<token> \
RUNNER_LABELS=self-hosted,linux,x64,ubuntu-latest cargo run

# Hosted runners ("create and wait" — no install step): set on the backend
#   RUNNER_PROVISIONER=docker
#   RUNNER_PROVISIONER_OVERUP_URL=<URL runner containers reach the API on>
#   RUNNER_IMAGE=ghcr.io/botcoder254/overup-runner:latest   (default)
#   RUNNER_PROVISIONER_DOCKER_HOST=<daemon for job execution; unset mounts
#                                   /var/run/docker.sock — root-equivalent>
#   RUNNER_AUTO_PROVISION=true       auto-create one hosted runner ("hosted-1",
#                                    labels self-hosted,linux,x64,ubuntu-latest)
#                                    when a workspace is created — warn-only,
#                                    never blocks workspace creation
#   HOSTED_RUNNERS_PER_WORKSPACE=3   quota on managed, non-revoked runners
#   HOSTED_RUNNERS_GLOBAL=20         quota across the whole deployment
#   RUNNER_PROVISIONER_NETWORK=overup-runners   dedicated bridge network for
#                                    runner containers (created at startup;
#                                    "bridge" opts out; create-failure falls
#                                    back to bridge, never disables the feature)
#   RUNNER_PROVISIONER_DEFAULT_PROFILE=standard  small|standard|large
# Hosted is the wizard's DEFAULT path (self-hosted moves behind "Advanced");
# it takes a resource profile (small=1CPU/1GiB/256pids, standard=2/2GiB/512,
# large=4/4GiB/1024 — services/runner_profiles.rs; limits bound the runner
# container AND are forwarded to job containers via RUNNER_JOB_* env) and an
# instance count (N rows named name-1..name-N, one batch tx under the quota
# advisory lock — all-or-nothing on name collision; response is
# {"runners":[...]}). Quota exhaustion shows remediation copy; a racing
# create 409s with static category hosted_runner_quota. Credential hygiene
# (JIT minting): rows are created credential-less; the background task pulls
# the image FIRST, then per instance mints the bootstrap token, arms only its
# hash via a guarded UPDATE (db::runners::arm_bootstrap — a revoked/purged
# row aborts before any container exists), injects it into the container env
# wrapped in zeroize::Zeroizing, and wipes it at end of iteration — plaintext
# never spans the pull and never reaches a spawn that outlives it. Runner
# containers are hardened: no-new-privileges, profile limits, dedicated
# network (no cap-drop — they drive Docker; job containers get cap-drop from
# the runner crate). Provisioning outcomes are audited (runner.provisioned /
# runner.provision_failed, actor NULL). Revoke deprovisions the container.
# The hourly janitor cleans up abandoned bootstraps, purges never-armed
# pending rows older than 2 h, AND reconciles Docker against runner rows:
# orphan overup.managed containers are deprovisioned, stopped ones restarted,
# and offline rows whose container vanished get provision_error=
# container_missing.

# Runner hardening knobs (defaults are least-privilege):
#   RUNNER_CAP_DROP=true            drop ALL capabilities (set false to opt out)
#   RUNNER_JOB_MEMORY_BYTES=2147483648   2 GiB/job (0 = unlimited)
#   RUNNER_JOB_NANO_CPUS=2000000000      2 CPUs/job (0 = unlimited)
#   RUNNER_JOB_PIDS_LIMIT=512            process cap (0 = unlimited)
#   RUNNER_JOB_NETWORK=bridge            bridge | none | isolated (per-job network)
#   RUNNER_JOB_USER=                     e.g. 1000:1000 for non-root execution
#   RUNNER_JOB_READONLY_ROOTFS=false     read-only rootfs + tmpfs /tmp
#
# Remote Docker (server daemon): the runner honors DOCKER_HOST. For a
# TLS-secured daemon on tcp://host:2376 also set DOCKER_TLS_VERIFY=1 and
# DOCKER_CERT_PATH=<dir with ca.pem/cert.pem/key.pem>. Unset = local
# socket/named pipe. Never expose an unauthenticated tcp://2375 daemon.
```

GitHub OAuth App registration: GitHub → Settings → Developer settings → OAuth Apps →
New OAuth App. Homepage `http://localhost:3000`, callback
`http://localhost:8080/auth/github/callback`. Never commit the client secret.

GitHub App registration (repositories/workflows): GitHub → Settings → Developer settings →
GitHub Apps → New GitHub App.
- Setup URL: `http://localhost:8080/auth/github/app/setup` (check "Redirect on update";
  in production use the API origin, e.g. `https://api.example.com/auth/github/app/setup`)
- Leave **"Request user authorization (OAuth) during installation" UNCHECKED** — enabling
  it makes GitHub redirect installs to the OAuth callback with no `state` parameter. The
  callback detects that case and forwards to the setup handler, but the correct
  configuration avoids the detour entirely
- Webhook URL: needs a public tunnel in dev — `smee.io` or `cloudflared tunnel` forwarding
  to `http://localhost:8080/webhooks/github`; set a strong webhook secret
- Repository permissions: **Metadata (read)** + **Contents (read)** — least privilege;
  `Workflows (write)` is only needed when editor write-back ships
- Subscribe to events: Push, Repository, Installation target, Pull request
  (installation/installation_repositories events arrive automatically)
- Generate a private key; set `GITHUB_APP_CLIENT_ID`, `GITHUB_APP_PRIVATE_KEY_PATH` (or
  `GITHUB_APP_PRIVATE_KEY_B64`), `GITHUB_WEBHOOK_SECRET`, `GITHUB_APP_SLUG` in
  `backend/.env`. Never commit the PEM or the secret.
- Without a webhook tunnel everything still works: the Repositories page has manual
  Re-sync, and imports always run an initial sync.

Verification: `cargo check`, `cargo clippy -- -D warnings`, and `cargo test` in `backend/`
(workflow parser, pipeline plan translation, conclusion matrix, log masking tests), in
`protocol/` (HMAC sign/verify round-trip, tamper/expiry rejection), and in `runner/`;
`npm run build` at the root. All clean as of this phase. Note: `backend`, `protocol`, and
`runner` are three sibling crates (no cargo workspace — keeps `backend/Cargo.lock` and the
documented `cd backend && cargo run` flow intact).
