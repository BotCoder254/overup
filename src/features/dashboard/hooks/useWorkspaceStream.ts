import { useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { env } from '../../../lib/env';
import type { Runner } from '../../../types/runner';
import type { WorkspaceStreamEvent } from '../../../types/dashboard';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { runnerKey, runnersKey } from '../../runners/hooks/useRunners';
import { dashboardRecentPipelinesKey } from './useDashboard';

/**
 * Live workspace-wide stream over an authenticated WebSocket. Feeds the
 * Dashboard and Runner Management pages: runner lifecycle/health changes and
 * meaningful pipeline transitions (started/finished — not every per-job
 * update). Session cookie rides along on the upgrade GET, same as the
 * per-pipeline stream. Consumers gate their own polling on `connected`.
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

    const patchRunner = (runner: Runner) => {
      queryClient.setQueryData<Runner[]>(runnersKey(workspaceId), (old) => {
        if (!old) return old;
        const exists = old.some((existing) => existing.id === runner.id);
        return exists
          ? old.map((existing) => (existing.id === runner.id ? runner : existing))
          : [...old, runner];
      });
      queryClient.setQueryData<Runner>(runnerKey(workspaceId, runner.id), (old) =>
        old ? runner : old,
      );
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
          break;
        case 'runner_update':
          patchRunner(event.runner);
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

    const connect = () => {
      if (disposed) return;
      const origin = env.apiOrigin || window.location.origin;
      const url = `${origin.replace(/^http/, 'ws')}/ws/workspaces/${workspaceId}/dashboard`;
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
        if (!disposed) {
          attempt += 1;
          const delay = Math.min(1000 * 2 ** Math.min(attempt - 1, 5), 30_000);
          reconnectTimer = setTimeout(connect, delay + Math.random() * 500);
        }
      };
      socket.onerror = () => {
        socket.close();
      };
    };

    connect();
    return () => {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      socketRef.current?.close();
      socketRef.current = null;
      setConnected(false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  return { connected };
}
