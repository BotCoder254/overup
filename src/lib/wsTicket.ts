import { api } from './api';

/**
 * Mint a one-time WebSocket auth ticket (60 s TTL, single-use). Used when
 * `env.wsOrigin` is set: the session cookie is first-party on the SPA origin
 * and cannot ride a direct cross-origin socket, so this REST call (which DOES
 * carry the cookie, through the SPA's proxy) trades it for a ticket the
 * upgrade GET presents as `?ticket=...`. Tickets are single-use — mint a
 * fresh one for every connection attempt.
 */
export async function mintWsTicket(workspaceId: string): Promise<string> {
  const body = await api
    .post(`/api/workspaces/${workspaceId}/ws-ticket`)
    .json<{ ticket: string }>();
  return body.ticket;
}
