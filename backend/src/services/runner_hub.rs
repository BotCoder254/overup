//! In-memory registry of connected runners. Each runner WebSocket task
//! registers an outbound channel here; the scheduler and cancel paths push
//! ServerMsg frames through it. A send failure simply means the runner
//! vanished — the disconnect/orphan machinery handles the rest.

use dashmap::DashMap;
use protocol::ServerMsg;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Bounded per-runner queue: a stalled runner exerts backpressure on its own
/// assignments, never on the whole scheduler.
const RUNNER_QUEUE: usize = 64;

#[derive(Default)]
pub struct RunnerHub {
    connections: DashMap<Uuid, mpsc::Sender<ServerMsg>>,
}

impl RunnerHub {
    /// Register a freshly authenticated runner connection; a previous entry
    /// for the same runner (stale reconnect) is replaced. The returned
    /// sender clone identifies this connection for `unregister_conn`.
    pub fn register(&self, runner_id: Uuid) -> (mpsc::Receiver<ServerMsg>, mpsc::Sender<ServerMsg>) {
        let (tx, rx) = mpsc::channel(RUNNER_QUEUE);
        self.connections.insert(runner_id, tx.clone());
        (rx, tx)
    }

    /// Remove the entry only if it still belongs to this connection —
    /// a reconnect that replaced it must not be unregistered by the old
    /// connection's cleanup.
    pub fn unregister_conn(&self, runner_id: Uuid, conn: &mpsc::Sender<ServerMsg>) {
        self.connections
            .remove_if(&runner_id, |_, tx| tx.same_channel(conn));
    }

    /// Unconditional removal (revocation, stale sweep).
    pub fn unregister(&self, runner_id: Uuid) {
        self.connections.remove(&runner_id);
    }

    pub fn is_connected(&self, runner_id: Uuid) -> bool {
        self.connections.contains_key(&runner_id)
    }

    pub fn connected_ids(&self) -> Vec<Uuid> {
        self.connections.iter().map(|entry| *entry.key()).collect()
    }

    /// Best-effort send. False means the runner is gone or its queue is
    /// saturated — callers treat both as "not reachable right now".
    pub fn send(&self, runner_id: Uuid, msg: ServerMsg) -> bool {
        match self.connections.get(&runner_id) {
            Some(tx) => tx.try_send(msg).is_ok(),
            None => false,
        }
    }
}
