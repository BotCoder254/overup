//! One-time tickets for cross-origin browser WebSocket authentication.
//!
//! Some deployments front the SPA with a proxy that can forward XHR but not
//! WebSocket upgrades (Netlify). There the session cookie is first-party on
//! the SPA origin and can never ride a direct socket to the API origin, so
//! the browser instead mints a ticket over the (proxied, cookie-carrying,
//! CSRF-checked, RBAC'd) REST API and presents it as `?ticket=...` on the
//! upgrade GET.
//!
//! Same credential standard as sessions and runner tokens: 32 OS-RNG bytes,
//! only the SHA-256 hash kept server-side, single-use (atomic remove — a
//! raced duplicate loses), 60-second TTL, bound to `{user, workspace}`.
//! Purely in-memory: tickets are ephemeral by design, so a restart simply
//! invalidates outstanding ones and the client mints a fresh ticket on its
//! next reconnect attempt.
//!
//! The raw ticket travels in the request URI. tower-http's trace layer only
//! records URIs in DEBUG-level spans (the default filter is `info`), the app
//! sets `Referrer-Policy: no-referrer`, and browsers keep WS URLs out of
//! history — and a captured ticket is already-spent, 60s-scoped, and
//! hash-only server-side.

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use uuid::Uuid;

use crate::services::session;

/// Tickets exist only to bridge the REST call and the immediately-following
/// upgrade; one minute absorbs slow networks without widening the window.
const TICKET_TTL_SECS: i64 = 60;
/// Defense-in-depth cap so a misbehaving client can't grow the map unbounded
/// (the `/api` governor already throttles minting well below this).
const MAX_TICKETS: usize = 4096;
/// Raw tickets are 43 chars (base64url of 32 bytes); reject junk pre-hash.
const MAX_TICKET_LEN: usize = 128;

struct TicketEntry {
    user_id: Uuid,
    workspace_id: Uuid,
    expires_at: DateTime<Utc>,
}

/// In-memory store of pending tickets, keyed by SHA-256 hex of the raw value.
#[derive(Default)]
pub struct WsTicketStore {
    tickets: DashMap<String, TicketEntry>,
}

impl WsTicketStore {
    /// Mint a ticket bound to `(user, workspace)`. Returns the raw value —
    /// it is never stored; only the hash is. Lazily purges expired entries,
    /// and returns `None` when the store is at capacity even after purging.
    pub fn mint(&self, user_id: Uuid, workspace_id: Uuid) -> Option<String> {
        let now = Utc::now();
        self.tickets.retain(|_, entry| entry.expires_at > now);
        if self.tickets.len() >= MAX_TICKETS {
            return None;
        }
        let (value, hash) = session::generate_token();
        self.tickets.insert(
            hash,
            TicketEntry {
                user_id,
                workspace_id,
                expires_at: now + Duration::seconds(TICKET_TTL_SECS),
            },
        );
        Some(value)
    }

    /// Single-use redemption: remove-then-verify. Returns the bound user id
    /// only if the ticket existed, is unexpired, and was minted for this
    /// workspace. The entry is burned on ANY outcome — `remove` is the
    /// atomic single-winner step, so a replayed ticket always gets `None`.
    pub fn consume(&self, raw: &str, workspace_id: Uuid) -> Option<Uuid> {
        if raw.is_empty() || raw.len() > MAX_TICKET_LEN {
            return None;
        }
        let (_, entry) = self.tickets.remove(&session::hash_token(raw))?;
        if entry.expires_at <= Utc::now() || entry.workspace_id != workspace_id {
            return None;
        }
        Some(entry.user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_then_consume_returns_bound_user() {
        let store = WsTicketStore::default();
        let user = Uuid::new_v4();
        let workspace = Uuid::new_v4();
        let ticket = store.mint(user, workspace).expect("mint");
        assert_eq!(store.consume(&ticket, workspace), Some(user));
    }

    #[test]
    fn tickets_are_single_use() {
        let store = WsTicketStore::default();
        let user = Uuid::new_v4();
        let workspace = Uuid::new_v4();
        let ticket = store.mint(user, workspace).expect("mint");
        assert_eq!(store.consume(&ticket, workspace), Some(user));
        assert_eq!(store.consume(&ticket, workspace), None);
    }

    #[test]
    fn wrong_workspace_is_rejected_and_burns_the_ticket() {
        let store = WsTicketStore::default();
        let user = Uuid::new_v4();
        let workspace = Uuid::new_v4();
        let ticket = store.mint(user, workspace).expect("mint");
        assert_eq!(store.consume(&ticket, Uuid::new_v4()), None);
        // Burned by the failed attempt — the right workspace no longer works.
        assert_eq!(store.consume(&ticket, workspace), None);
    }

    #[test]
    fn expired_tickets_are_rejected() {
        let store = WsTicketStore::default();
        let user = Uuid::new_v4();
        let workspace = Uuid::new_v4();
        let ticket = store.mint(user, workspace).expect("mint");
        let hash = session::hash_token(&ticket);
        store.tickets.get_mut(&hash).expect("entry").expires_at =
            Utc::now() - Duration::seconds(1);
        assert_eq!(store.consume(&ticket, workspace), None);
    }

    #[test]
    fn garbage_and_overlong_tokens_are_rejected() {
        let store = WsTicketStore::default();
        let workspace = Uuid::new_v4();
        assert_eq!(store.consume("", workspace), None);
        assert_eq!(store.consume("not-a-ticket", workspace), None);
        assert_eq!(store.consume(&"x".repeat(MAX_TICKET_LEN + 1), workspace), None);
    }

    #[test]
    fn mint_purges_expired_entries() {
        let store = WsTicketStore::default();
        let workspace = Uuid::new_v4();
        let ticket = store.mint(Uuid::new_v4(), workspace).expect("mint");
        let hash = session::hash_token(&ticket);
        store.tickets.get_mut(&hash).expect("entry").expires_at =
            Utc::now() - Duration::seconds(1);
        store.mint(Uuid::new_v4(), workspace).expect("mint");
        assert!(!store.tickets.contains_key(&hash));
    }
}
