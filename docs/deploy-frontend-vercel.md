# Deploying the Overup frontend to Vercel

This guide deploys the React frontend (`src/`) to **Vercel**, wired to a backend
already running per [deploy-backend-dokploy.md](./deploy-backend-dokploy.md) (examples
use `https://overup-api.duckdns.org` — substitute your API domain). It is the Vercel
equivalent of [deploy-frontend-netlify.md](./deploy-frontend-netlify.md); the auth
architecture is identical, only the platform config differs.

---

## 1. The architecture: why everything is proxied

Overup's auth is a **first-party HttpOnly session cookie** (`__Host-` prefixed,
`SameSite=Lax`). Modern browsers block or strip cross-site cookies (Safari ITP,
Firefox Total Cookie Protection, Chrome's third-party-cookie controls), so the SPA
on `overup.vercel.app` calling `overup-api.duckdns.org` **directly** would never
carry the session — login would silently fail in most browsers.

The fix is [`vercel.json`](../vercel.json): Vercel **proxy rewrites** forward
`/api/*` and the auth flow server-side to the backend. From the browser's point of
view everything is same-origin on the Vercel site, so the cookie is first-party and
works in every browser:

```
Browser ── same-origin ──▶ overup.vercel.app
                             ├── /api/*          ──proxy──▶ overup-api.duckdns.org/api/*
                             ├── /auth/github/*  ──proxy──▶ overup-api.duckdns.org/auth/github/*
                             ├── /auth/logout    ──proxy──▶ overup-api.duckdns.org/auth/logout
                             └── /* (everything else) ──▶ index.html (the SPA)
```

Notes on the rewrite rules (order matters — most specific first; Vercel checks the
filesystem before `rewrites`, so hashed static assets serve directly and only real
routes fall through to the SPA fallback):

- `/auth/github/*` covers login, the OAuth callback, and the GitHub App setup
  redirect. It deliberately does **not** match `/auth/callback` — that is a React
  route and falls through to the SPA fallback.
- `REACT_APP_API_ORIGIN` is **empty** for this deployment: XHR stays relative
  (`/api/me`), the full-page login navigation goes to `/auth/github/login` on the
  Vercel origin, and both flow through the proxy.
- There is deliberately **no custom `headers` block** in `vercel.json` (parity with
  `netlify.toml`): the backend sets CSP / `X-Frame-Options` / `Referrer-Policy` etc.
  on every API response, and a Vercel `headers` glob would also stamp the proxied
  `/api` responses (duplicate/conflicting headers) while a static CSP would break
  Monaco/xterm.

**WebSockets — direct with one-time tickets.** Vercel's proxy cannot forward
WebSocket upgrades, so the live streams (`/ws/...`) don't go through the proxy at
all. Instead, `REACT_APP_WS_ORIGIN` points the sockets **directly at the API
origin**, and because the session cookie is first-party on the Vercel origin (it
can't ride a cross-origin upgrade), the app first mints a **one-time, 60-second WS
ticket** over the proxied REST API (`POST /api/workspaces/{id}/ws-ticket` — cookie,
CSRF header, and RBAC all apply) and presents it as `?ticket=` on the upgrade. The
backend still enforces its strict `Origin == FRONTEND_URL` allow-list on the
handshake, so `FRONTEND_URL` (§4) must be exactly this site's origin. If a socket
can't connect, the app falls back to REST polling and goes dormant after a few
attempts rather than retrying forever. Runners are unaffected (they connect straight
to the API domain with their own bearer token).

## 2. One-time Vercel setup (CLI)

Requires `vercel login` done once (`vercel whoami` confirms it). Then, from the repo
root:

```bash
# Create/link the project. The project name defaults to the directory name (overup);
# pass --project <name> to choose another. This writes .vercel/ (gitignored).
vercel link --yes
```

Build-time env lives in [`vercel.json`](../vercel.json) under `build.env`
(`REACT_APP_API_ORIGIN` empty, `REACT_APP_WS_ORIGIN` = the API origin), committed with
the repo — so a Git-connected project builds correctly without any dashboard env. If
you prefer them as Project Environment Variables instead, set:

```bash
vercel env add REACT_APP_API_ORIGIN production   # enter an empty value
vercel env add REACT_APP_WS_ORIGIN production     # https://overup-api.duckdns.org
```

`vercel.json` is committed — build command, output dir, `build.env`, and the proxy
rules all live there.

## 3. Deploy

```bash
vercel deploy --prod --yes    # builds on Vercel (reads build.env from vercel.json)
```

The command prints the production URL. Redeploying after changes is the same command.
(Alternatively connect the GitHub repo in the Vercel UI for deploys on push —
`vercel.json` already carries the build settings.)

## 4. Backend + GitHub configuration for the Vercel origin

The session cookie now lives on the **Vercel** origin, so the browser-facing GitHub
URLs must go through it. Server-to-server URLs stay on the API domain. Substitute your
real production URL for `overup.vercel.app`.

**Dokploy → backend application → Environment** (then Redeploy):

```env
FRONTEND_URL=https://overup.vercel.app
OAUTH_REDIRECT_URL=https://overup.vercel.app/auth/github/callback
```

**GitHub OAuth App** (Settings → Developer settings → OAuth Apps):

- Homepage URL: `https://overup.vercel.app`
- Authorization callback URL: `https://overup.vercel.app/auth/github/callback`
  (must equal `OAUTH_REDIRECT_URL` exactly)

**GitHub App** (Settings → Developer settings → GitHub Apps):

- Setup URL: `https://overup.vercel.app/auth/github/app/setup`
  (the setup redirect authenticates with the session cookie — it must ride the proxy)
- **Webhook URL: unchanged** — `https://overup-api.duckdns.org/webhooks/github`
  (server→server; no cookie involved; never proxy webhooks through Vercel)

> Because `FRONTEND_URL` is a single origin (the WS Origin allow-list), pointing it at
> the Vercel URL makes Vercel the primary auth origin and supersedes any prior
> Netlify origin for login/WebSockets.

## 5. Verify

1. `https://overup.vercel.app` loads the sign-in page with the Squirrel favicon in
   the tab.
2. `curl -s https://overup.vercel.app/api/me` returns the backend's JSON 401
   (`{"error":...}`) — proves the proxy reaches the API. If it returns HTML, the
   rewrite rules aren't active (see troubleshooting).
3. "Continue with GitHub" → authorize → you land back in the app, logged in.
4. Open the dashboard or a running pipeline: statuses and logs update live — the
   Network tab shows `POST .../ws-ticket` followed by a 101-switching WebSocket to
   `wss://overup-api.duckdns.org/ws/...` (see §1). If the socket can't connect, pages
   still update via polling.

## 6. Troubleshooting

| Symptom | Likely cause / fix |
| --- | --- |
| `/api/me` returns HTML instead of JSON | The rewrite rules aren't applied — `vercel.json` missing from the deployed commit, or the SPA fallback (`/:path*` → `/index.html`) is ordered before the API rules. |
| `/api/*` returns 502 from Vercel | Vercel can't reach the backend — the API domain's ports 80/443 are closed or the cert/domain isn't up yet (see the backend guide §6.1 and its firewall note). |
| Login round-trip works but you land signed out | `FRONTEND_URL` or `OAUTH_REDIRECT_URL` still points at the old origin — both must use the Vercel URL, then redeploy the backend. Also confirm the GitHub OAuth App callback matches exactly. |
| GitHub authorize page errors immediately | OAuth App callback URL ≠ `OAUTH_REDIRECT_URL`. |
| Repo install redirect fails after choosing repos | GitHub App Setup URL still points at the API domain — it must be the Vercel URL (the cookie is on the Vercel origin). |
| Live logs feel delayed / WS won't connect | The socket goes direct with a ticket (§1). Check: `REACT_APP_WS_ORIGIN` was set at build time, backend `FRONTEND_URL` equals the Vercel origin **exactly** (it is the WS Origin allow-list), and `wss://` on the API domain isn't blocked by the reverse proxy. While the socket is down the app polls, so data still updates. |
| Old React favicon in the tab | Hard-refresh / clear the tab's favicon cache; favicons are cached aggressively. |
