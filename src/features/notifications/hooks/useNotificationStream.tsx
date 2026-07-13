import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { env } from '../../../lib/env';
import { mintWsTicket } from '../../../lib/wsTicket';
import type { NotificationStreamEvent } from '../../../types/notification';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { notificationsKey, unreadCountKey } from './useNotifications';

const DORMANT_AFTER = 5;
const DORMANT_DELAY_MS = 5 * 60_000;

/**
 * Live per-user notification stream over an authenticated WebSocket
 * (`/ws/workspaces/{ws}/notifications`) — the useWorkspaceStream pattern,
 * mounted once in the AppShell so the bell is live on every page. The
 * socket is a latency optimization: the snapshot frame carries the
 * authoritative unread count, notification frames patch the badge and
 * invalidate the list caches, and while disconnected the REST hooks poll.
 */
export function useNotificationStream() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  const [connected, setConnected] = useState(false);
  const socketRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!workspaceId) return undefined;

    let disposed = false;
    let attempt = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;

    const setUnread = (count: number) => {
      queryClient.setQueryData<number>(unreadCountKey(workspaceId), count);
    };

    const handleEvent = (event: NotificationStreamEvent) => {
      switch (event.type) {
        case 'snapshot':
          // Authoritative on every (re)connect; anything missed while
          // disconnected is folded in by the list invalidation below.
          setUnread(event.unreadCount);
          void queryClient.invalidateQueries({
            queryKey: [...notificationsKey(workspaceId), 'list'],
          });
          break;
        case 'notification':
          // Fresh row bumps the badge; a dedup merge keeps it (the unread
          // row already counted). Mounted lists refetch for the row itself.
          if (event.inserted) {
            queryClient.setQueryData<number>(unreadCountKey(workspaceId), (old) =>
              typeof old === 'number' ? old + 1 : old,
            );
          }
          void queryClient.invalidateQueries({
            queryKey: [...notificationsKey(workspaceId), 'list'],
          });
          // Critical operational incidents surface immediately, wherever
          // the user is (Toaster is mounted once in providers.tsx).
          if (event.notification.severity === 'critical') {
            toast.error(event.notification.title, {
              description: event.notification.body || undefined,
            });
          }
          break;
        case 'unread_count':
          // Pushed after read/archive mutations so sibling tabs converge.
          setUnread(event.count);
          void queryClient.invalidateQueries({
            queryKey: [...notificationsKey(workspaceId), 'list'],
          });
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
      const url = `${origin.replace(/^http/, 'ws')}/ws/workspaces/${workspaceId}/notifications${ticketQuery}`;
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
          handleEvent(JSON.parse(message.data as string) as NotificationStreamEvent);
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
      socketRef.current?.close();
      socketRef.current = null;
      setConnected(false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  return { connected };
}

const NotificationStreamContext = createContext<{ connected: boolean }>({ connected: false });

/**
 * Mounted once in the AppShell: owns the socket and shares `connected` so
 * the bell and the history page gate their polling on the same stream.
 */
export function NotificationStreamProvider({ children }: { children: ReactNode }) {
  const stream = useNotificationStream();
  return (
    <NotificationStreamContext.Provider value={stream}>
      {children}
    </NotificationStreamContext.Provider>
  );
}

export function useNotificationStreamContext() {
  return useContext(NotificationStreamContext);
}
