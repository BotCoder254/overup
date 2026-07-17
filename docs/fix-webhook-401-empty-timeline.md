# Fix: every GitHub delivery is a red 401 — the empty-event-timeline runbook

This is the runbook for the failure where **the repository Events timeline shows
"No repository events processed yet…" even though commits are being pushed**, and
the GitHub App's Recent Deliveries page shows every delivery red with a `401`
response.

Root cause: **the `GITHUB_WEBHOOK_SECRET` the backend loaded does not byte-match
the webhook secret configured on the GitHub App.** A delivery that fails HMAC
verification is rejected **before anything is persisted** — that is the security
boundary (unauthenticated input must never reach Postgres) — so no queue row, no
processing, no timeline entry. From inside the product the repository just looks
quiet.

## 0. The 5-second diagnostics

Three independent signals confirm this failure, use whichever is closest:

1. **The "Webhook auth" KPI** on the repository page's sync panel is red with a
   rejection count and a cause hint (e.g. `secret mismatch — check
   GITHUB_WEBHOOK_SECRET`). Healthy deployments show `OK`. This gauge is
   deployment-global and in-memory (resets on restart — deliberately: it answers
   "is the CURRENT deployment's secret wrong?").
2. **The Events tab** replaces its quiet empty state with the "Webhook deliveries
   are being rejected" remediation card whenever rejections are being seen.
3. **The backend log** carries one static-cause line per rejected delivery:

   ```
   WARN webhook delivery failed signature verification cause=mismatch event=push delivery_id=…
   ```

## 1. Decode the `cause=` label

The cause is a static category — the signature, digest, and secret are never
logged:

| `cause=` | Meaning | Operator action |
| --- | --- | --- |
| `mismatch` | The configured secret's bytes differ from the App's | §2 — re-set the secret on both sides |
| `missing_header` | GitHub sent no `X-Hub-Signature-256` at all | The App has **no webhook secret configured** — set one in the App settings, then §2 |
| `bad_prefix` | Signature isn't `sha256=…` | Sender isn't GitHub, or a legacy sha1-only configuration — verify the webhook URL is only registered on your App |
| `invalid_hex` | Digest after `sha256=` isn't hex | Malformed sender; same check as `bad_prefix` |
| `malformed_header` | Header unreadable as ASCII | Same check as `bad_prefix` |

## 2. The fix (mismatch — the overwhelmingly common case)

1. Open the GitHub App: GitHub → Settings → Developer settings → GitHub Apps →
   your app. If you don't know the current secret, set a fresh one (Webhook
   secret → change) — GitHub never displays an existing secret again.
2. In the backend deployment environment (Dokploy: Application → Environment),
   set `GITHUB_WEBHOOK_SECRET` to that value **exactly**:
   - **no surrounding quotes** — the loader warns about quote-wrapped values but
     deliberately never strips them (a secret may legitimately contain a quote);
   - whitespace/newlines are trimmed at load (`config.rs::shared_secret`, the
     fix for the original 401 storm), but don't rely on it — paste the bare
     value;
   - it must be ≥ 16 bytes or the backend refuses to boot.
3. **Redeploy** the backend so the new value loads. Confirm the deployment built
   from the branch you expect (Dokploy tracks `main`).
4. GitHub App → Advanced → Recent Deliveries → open a failed delivery →
   **Redeliver**.

## 3. Verify

- The redelivered delivery flips green with a **`202`** response.
- A row appears on the repository's **Events** tab within a couple of seconds
  (the processor drains on a poke; 2 s tick as backstop).
- The **Webhook auth** KPI reads `OK` — the gauge resets to zero on the
  redeploy, so a NEW red count after redeploying means the secret is *still*
  wrong.
- Pushes to the default branch now schedule syncs, and matching workflows create
  pipelines (linked from the event row).

## 4. Look-alikes that are NOT this failure

| Recent Deliveries shows | Actual cause |
| --- | --- |
| `404` / connection errors | Webhook URL wrong (must be `https://<api-domain>/webhooks/github`) or backend down/unreachable |
| `413` | Delivery body over the webhook body cap (25 MiB — GitHub's documented max is 25 MB; if you see this, the payload is anomalous) |
| `429` | Rate limiter — sized as a generous DoS floor (100 rps/500 burst per IP); sustained 429s mean something is flooding the endpoint |
| Green `202`, but timeline row says **Ignored** | Delivery processed fine; the static reason on the row explains why nothing ran (no matching workflows, filters not matched, fork PR skipped, …) |
| Green `202`, timeline empty | The repository isn't connected in the workspace you're viewing — connect it on the Repositories page |
