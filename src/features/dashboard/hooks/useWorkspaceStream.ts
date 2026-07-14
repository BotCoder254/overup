import { useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { env } from '../../../lib/env';
import { mintWsTicket } from '../../../lib/wsTicket';
import type { Runner } from '../../../types/runner';
import type { WorkspaceStreamEvent } from '../../../types/dashboard';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { runnerKey, runnersKey } from '../../runners/hooks/useRunners';
import { dashboardRecentPipelinesKey } from './useDashboard';

/**
 * After this many consecutive failures the stream goes dormant and retries
 * rarely — the polling fallback keeps the pages fresh, so there is no point
 * hammering an endpoint the network can't reach.
 */
const DORMANT_AFTER = 5;
const DORMANT_DELAY_MS = 5 * 60_000;

/**
 * Live workspace-wide stream over an authenticated WebSocket. Feeds the
 * Dashboard and Runner Management pages: runner lifecycle/health changes and
 * meaningful pipeline transitions (started/finished — not every per-job
 * update). Auth: the session cookie rides along on the same-origin upgrade
 * GET; when `env.wsOrigin` points the socket at a different origin (proxies
 * that can't forward upgrades, e.g. Netlify), a one-time ticket minted over
 * REST is used instead. Consumers gate their own polling on `connected`.
 */
export function useWorkspaceStream() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  const [connected, setConnected] = useState(false);
  const socketRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!workspaceId) return undefined;

    let disposed = false;
    let attempt = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;

    // Frame ordering makes edits safe without extra bookkeeping: the backend
    // emits the runner_update for a mutation at commit time, before the HTTP
    // response triggers the invalidation refetch, so any stale frame is
    // always followed by one carrying the committed state. Revokes are the
    // exception — the trailing frame arrives with revoked: true AFTER the
    // refetch removed the row, so it must remove (never re-append), or the
    // just-deleted runner resurrects until a manual refresh.
    const patchRunner = (runner: Runner) => {
      queryClient.setQueryData<Runner[]>(runnersKey(workspaceId), (old) => {
        if (!old) return old;
        if (runner.revoked) return old.filter((existing) => existing.id !== runner.id);
        const exists = old.some((existing) => existing.id === runner.id);
        return exists
          ? old.map((existing) => (existing.id === runner.id ? runner : existing))
          : [...old, runner];
      });
      queryClient.setQueryData<Runner>(runnerKey(workspaceId, runner.id), (old) =>
        old ? { ...old, ...runner } : old,
      );
      if (runner.revoked) {
        // Cross-client revokes free hosted-quota headroom too.
        void queryClient.invalidateQueries({
          queryKey: ['workspaces', workspaceId, 'runners', 'hosted-info'],
        });
      }
    };

    // Pipeline transitions arrive in bursts (one frame per pipeline edge,
    // and a multi-job pipeline emits several in quick succession); a
    // trailing debounce collapses each burst into a single jobs refetch so
    // the queue page doesn't multiply its own polling.
    let jobsInvalidateTimer: ReturnType<typeof setTimeout> | undefined;
    const invalidateJobsSoon = () => {
      if (jobsInvalidateTimer) return;
      jobsInvalidateTimer = setTimeout(() => {
        jobsInvalidateTimer = undefined;
        void queryClient.invalidateQueries({
          queryKey: ['workspaces', workspaceId, 'jobs'],
        });
      }, 1000);
    };

    // Every delta frame is written to the audit ledger at (or before) the
    // moment it is emitted, so any frame is a cheap "the feed moved" signal.
    // Coarse prefix invalidation covers the feed AND its summary strip; a
    // no-op when the Activity page isn't mounted.
    const invalidateActivity = () => {
      void queryClient.invalidateQueries({
        queryKey: ['workspaces', workspaceId, 'activity'],
      });
    };

    const handleEvent = (event: WorkspaceStreamEvent) => {
      switch (event.type) {
        case 'snapshot':
          // Deltas only from here — the Dashboard/Runners pages already
          // fetched their initial state over REST on mount.
          break;
        case 'pipeline_update':
          // Cheap aggregate/small-limit queries: a coarse refetch is
          // simpler and just as correct as hand-patching (and handles a
          // brand-new pipeline entering the recent-pipelines top-10 or
          // shifting the KPI counts, which a targeted patch would miss).
          void queryClient.invalidateQueries({ queryKey: dashboardRecentPipelinesKey(workspaceId) });
          void queryClient.invalidateQueries({
            queryKey: ['workspaces', workspaceId, 'dashboard', 'summary'],
          });
          void queryClient.invalidateQueries({
            queryKey: ['workspaces', workspaceId, 'dashboard', 'activity'],
          });
          // Pipeline transitions are exactly when the job queue changes
          // shape; a no-op when the queue page isn't mounted.
          invalidateJobsSoon();
          invalidateActivity();
          break;
        case 'runner_update':
          patchRunner(event.runner);
          invalidateActivity();
          break;
        case 'artifact_update':
          // Coarse prefix invalidation covers the catalog, summary, detail,
          // and retention queries in one shot; a no-op when the Artifacts
          // page isn't mounted.
          void queryClient.invalidateQueries({
            queryKey: ['workspaces', workspaceId, 'artifacts'],
          });
          invalidateActivity();
          break;
        case 'activity_update':
          // Categories without their own frame (secret/environment/
          // repository/integration) still reach live feeds this way.
          invalidateActivity();
          break;
        case 'runner_health':
          queryClient.setQueryData<Runner[]>(runnersKey(workspaceId), (old) =>
            old?.map((runner) =>
              runner.id === event.runnerId
                ? { ...runner, lastHealth: event.health, lastSeenAt: event.lastSeenAt }
                : runner,
            ),
          );
          queryClient.setQueryData<Runner>(runnerKey(workspaceId, event.runnerId), (old) =>
            old ? { ...old, lastHealth: event.health, lastSeenAt: event.lastSeenAt } : old,
          );
          break;
        case 'pong':
          break;
        default:
          break;
      }
    };

    const scheduleReconnect = () => {
      if (disposed) return;
      attempt += 1;
      // Past DORMANT_AFTER straight failures, back way off: polling covers
      // freshness, and endless fast retries only spam the console.
      const delay =
        attempt >= DORMANT_AFTER
          ? DORMANT_DELAY_MS
          : Math.min(1000 * 2 ** Math.min(attempt - 1, 5), 30_000);
      reconnectTimer = setTimeout(() => {
        void connect();
      }, delay + Math.random() * 500);
    };

    const connect = async () => {
      if (disposed) return;
      const origin = env.wsOrigin || env.apiOrigin || window.location.origin;
      let ticketQuery = '';
      if (env.wsOrigin) {
        // Cross-origin socket: the session cookie won't ride along, so trade
        // it for a one-time ticket over REST (fresh per attempt — single-use).
        try {
          const ticket = await mintWsTicket(workspaceId);
          ticketQuery = `?ticket=${encodeURIComponent(ticket)}`;
        } catch {
          scheduleReconnect();
          return;
        }
        if (disposed) return; // torn down while awaiting the ticket
      }
      const url = `${origin.replace(/^http/, 'ws')}/ws/workspaces/${workspaceId}/dashboard${ticketQuery}`;
      const socket = new WebSocket(url);
      socketRef.current = socket;

      let lastMessageAt = Date.now();
      let keepaliveTimer: ReturnType<typeof setInterval> | undefined;

      socket.onopen = () => {
        attempt = 0;
        setConnected(true);
        lastMessageAt = Date.now();
        keepaliveTimer = setInterval(() => {
          if (Date.now() - lastMessageAt > 60_000) {
            socket.close();
            return;
          }
          if (socket.readyState === WebSocket.OPEN) {
            socket.send(JSON.stringify({ type: 'ping' }));
          }
        }, 25_000);
      };
      socket.onmessage = (message) => {
        lastMessageAt = Date.now();
        try {
          handleEvent(JSON.parse(message.data as string) as WorkspaceStreamEvent);
        } catch {
          // Malformed frame: ignore; polling remains the safety net.
        }
      };
      socket.onclose = () => {
        if (keepaliveTimer) clearInterval(keepaliveTimer);
        keepaliveTimer = undefined;
        setConnected(false);
        socketRef.current = null;
        scheduleReconnect();
      };
      socket.onerror = () => {
        socket.close();
      };
    };

    void connect();
    return () => {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      if (jobsInvalidateTimer) clearTimeout(jobsInvalidateTimer);
      socketRef.current?.close();
      socketRef.current = null;
      setConnected(false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  return { connected };
}
