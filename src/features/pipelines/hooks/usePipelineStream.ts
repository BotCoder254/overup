import { useCallback, useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { env } from '../../../lib/env';
import type {
  PipelineDetail,
  PipelineStreamEvent,
} from '../../../types/pipeline';
import { LOG_PAGE, getJobLogs } from '../api/pipelinesApi';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { useLogStore } from '../stores/logStore';
import { artifactsKey, pipelineKey } from './usePipelines';

/**
 * Live pipeline stream over an authenticated WebSocket.
 *
 * The session cookie rides along on the upgrade GET: in dev,
 * localhost:3000 -> localhost:8080 is same-site (SameSite is scheme+host,
 * not port), and in production the socket is same-origin `wss://`. Events
 * patch the react-query detail cache and push log chunks into the log
 * store; while disconnected, the detail query's polling takes over.
 */
export function usePipelineStream(pipelineId: string | undefined) {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  const [connected, setConnected] = useState(false);
  const socketRef = useRef<WebSocket | null>(null);
  // Jobs whose logs the UI wants; resubscribed after every reconnect.
  const watchedJobsRef = useRef<Set<string>>(new Set());
  const appendLogs = useLogStore((state) => state.append);
  const lastSeq = useLogStore((state) => state.lastSeq);
  const clearLogs = useLogStore((state) => state.clear);

  const sendSubscribe = useCallback(
    (jobId: string) => {
      const socket = socketRef.current;
      if (socket?.readyState === WebSocket.OPEN) {
        socket.send(
          JSON.stringify({ type: 'subscribe_logs', jobId, fromSeq: lastSeq(jobId) + 1 }),
        );
      }
    },
    [lastSeq],
  );

  /** Ask for a job's logs: backfill over the socket, then live chunks. */
  const watchJobLogs = useCallback(
    (jobId: string) => {
      watchedJobsRef.current.add(jobId);
      sendSubscribe(jobId);
    },
    [sendSubscribe],
  );

  /**
   * REST fallback used while the socket is down. Pages until a short page
   * arrives, so gaps larger than one page are fully repaired (bounded by
   * the server's per-job log cap).
   */
  const backfillOverRest = useCallback(
    async (jobId: string) => {
      if (!workspaceId || !pipelineId) return;
      try {
        let from = lastSeq(jobId) + 1;
        for (;;) {
          // The overflow marker sits at i64::MAX; past JS's safe range
          // there is nothing further to fetch.
          if (!Number.isSafeInteger(from)) break;
          const body = await getJobLogs(workspaceId, pipelineId, jobId, from);
          appendLogs(
            jobId,
            body.chunks.map((chunk) => ({
              seq: chunk.seq,
              stream: (chunk.stream as 'stdout' | 'stderr' | 'system') ?? 'stdout',
              content: chunk.content,
              createdAt: chunk.createdAt,
            })),
          );
          if (body.chunks.length < LOG_PAGE) break;
          from = body.chunks[body.chunks.length - 1].seq + 1;
        }
      } catch {
        // Polling keeps state fresh; log backfill retries on next request.
      }
    },
    [workspaceId, pipelineId, lastSeq, appendLogs],
  );

  useEffect(() => {
    if (!workspaceId || !pipelineId) return undefined;

    // A new pipeline means a fresh log buffer and watch list.
    clearLogs();
    watchedJobsRef.current = new Set();

    let disposed = false;
    let attempt = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
    const detailKey = pipelineKey(workspaceId, pipelineId);

    const handleEvent = (event: PipelineStreamEvent) => {
      switch (event.type) {
        case 'snapshot':
          queryClient.setQueryData<PipelineDetail>(detailKey, (old) => ({
            pipeline: event.pipeline,
            jobs: event.jobs,
            events: old?.events ?? [],
          }));
          break;
        case 'pipeline_update':
          queryClient.setQueryData<PipelineDetail>(detailKey, (old) =>
            old
              ? {
                  ...old,
                  pipeline: {
                    ...old.pipeline,
                    status: event.status,
                    conclusion: event.conclusion,
                    startedAt: event.startedAt,
                    finishedAt: event.finishedAt,
                  },
                }
              : old,
          );
          break;
        case 'job_update':
          queryClient.setQueryData<PipelineDetail>(detailKey, (old) =>
            old
              ? {
                  ...old,
                  jobs: old.jobs.map((job) => (job.id === event.job.id ? event.job : job)),
                }
              : old,
          );
          break;
        case 'event':
          queryClient.setQueryData<PipelineDetail>(detailKey, (old) => {
            if (!old) return old;
            if (old.events.some((existing) => existing.id === event.event.id)) return old;
            return { ...old, events: [...old.events, event.event] };
          });
          break;
        case 'log':
          appendLogs(event.jobId, [
            {
              seq: event.seq,
              stream: event.stream,
              content: event.text,
              createdAt: event.createdAt,
            },
          ]);
          break;
        case 'log_gap':
          // We fell behind the broadcast: resync state and logs over REST.
          void queryClient.invalidateQueries({ queryKey: detailKey });
          for (const jobId of Array.from(watchedJobsRef.current)) {
            void backfillOverRest(jobId);
          }
          break;
        case 'artifact':
          void queryClient.invalidateQueries({
            queryKey: artifactsKey(workspaceId, pipelineId),
          });
          break;
        case 'pong':
          // Keepalive reply; receipt alone refreshed the watchdog.
          break;
        default:
          break;
      }
    };

    const connect = () => {
      if (disposed) return;
      const origin = env.apiOrigin || window.location.origin;
      const url = `${origin.replace(/^http/, 'ws')}/ws/workspaces/${workspaceId}/pipelines/${pipelineId}`;
      const socket = new WebSocket(url);
      socketRef.current = socket;

      // Application-level keepalive: browsers surface no native ping, so a
      // half-open socket would otherwise look connected forever. Any inbound
      // frame (including the pong reply) refreshes the watchdog; a silent
      // minute closes the socket and hands over to the backoff reconnect.
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
        // Resume log streams from where the buffer left off.
        for (const jobId of Array.from(watchedJobsRef.current)) {
          socket.send(
            JSON.stringify({
              type: 'subscribe_logs',
              jobId,
              fromSeq: useLogStore.getState().lastSeq(jobId) + 1,
            }),
          );
        }
      };
      socket.onmessage = (message) => {
        lastMessageAt = Date.now();
        try {
          handleEvent(JSON.parse(message.data as string) as PipelineStreamEvent);
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
          // Exponential backoff 1s -> 30s with jitter.
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
  }, [workspaceId, pipelineId]);

  return { connected, watchJobLogs, backfillOverRest };
}
