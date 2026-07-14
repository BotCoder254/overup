import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import { HTTPError } from 'ky';
import { toast } from 'sonner';
import type {
  Notification,
  NotificationListResponse,
  NotificationPreferences,
} from '../../../types/notification';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import {
  type BulkNotificationAction,
  type NotificationFilters,
  bulkUpdateNotifications,
  getNotificationPreferences,
  getUnreadCount,
  listNotifications,
  markAllNotificationsRead,
  markNotificationRead,
  putNotificationPreferences,
} from '../api/notificationsApi';

export { useWorkspaceId };

export const notificationsKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'notifications'] as const;
export const notificationsListKey = (workspaceId: string, filters: NotificationFilters = {}) =>
  [
    'workspaces',
    workspaceId,
    'notifications',
    'list',
    {
      unread: filters.unread ?? '',
      category: filters.category ?? '',
      severity: filters.severity ?? '',
      repositoryId: filters.repositoryId ?? '',
      q: filters.q ?? '',
      createdAfter: filters.createdAfter ?? '',
      createdBefore: filters.createdBefore ?? '',
      archived: filters.archived ?? '',
    },
  ] as const;
export const unreadCountKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'notifications', 'unread-count'] as const;
export const notificationPreferencesKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'notifications', 'preferences'] as const;

/** 422 bodies carry the server's specific validation message. */
async function describeError(error: unknown, fallback: string): Promise<string> {
  if (error instanceof HTTPError) {
    if (error.response.status === 422) {
      try {
        const body = (await error.response.clone().json()) as {
          error?: { message?: string };
        };
        if (body.error?.message) return body.error.message;
      } catch {
        // fall through to the generic copy
      }
      return 'The request was rejected as invalid.';
    }
    if (error.response.status === 429) return 'Too many attempts — please wait a moment.';
    if (error.response.status === 403) {
      return 'You do not have permission to view notifications.';
    }
  }
  return fallback;
}

/**
 * Keyset-paginated notification list. While the notification stream is
 * connected, frames drive invalidation; disconnected, a slow poll keeps the
 * feed moving (the stream-gated polling pattern from the dashboard).
 */
export function useNotificationsList(filters: NotificationFilters = {}, connected = false) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: notificationsListKey(workspaceId ?? '', filters),
    queryFn: ({ pageParam }) =>
      listNotifications(workspaceId!, { ...filters, cursor: pageParam || undefined }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId),
    refetchInterval: connected ? false : 30_000,
  });
}

/** The bell badge count; authoritative frames patch it while connected. */
export function useUnreadCount(connected = false) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: unreadCountKey(workspaceId ?? ''),
    queryFn: () => getUnreadCount(workspaceId!),
    enabled: Boolean(workspaceId),
    refetchInterval: connected ? false : 30_000,
  });
}

/** Flip one notification read everywhere it is cached, without refetching. */
function patchRead(
  queryClient: ReturnType<typeof useQueryClient>,
  workspaceId: string,
  notificationId: string,
) {
  const now = new Date().toISOString();
  queryClient.setQueriesData<{ pages: NotificationListResponse[]; pageParams: unknown[] }>(
    { queryKey: [...notificationsKey(workspaceId), 'list'] },
    (old) =>
      old && {
        ...old,
        pages: old.pages.map((page) => ({
          ...page,
          notifications: page.notifications.map((n: Notification) =>
            n.id === notificationId && !n.readAt ? { ...n, readAt: now } : n,
          ),
        })),
      },
  );
  queryClient.setQueryData<number>(unreadCountKey(workspaceId), (old) =>
    typeof old === 'number' ? Math.max(0, old - 1) : old,
  );
}

/**
 * Background mark-read (clicking a card): optimistic — the caches flip
 * immediately, and a failure just leaves the row unread for the next sync.
 * A 404 means "already read or gone", which is success for this purpose.
 */
export function useMarkRead() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (notificationId: string) => markNotificationRead(workspaceId!, notificationId),
    onMutate: (notificationId) => {
      if (workspaceId) patchRead(queryClient, workspaceId, notificationId);
    },
    onError: (error) => {
      if (error instanceof HTTPError && error.response.status === 404) return;
      if (workspaceId) {
        void queryClient.invalidateQueries({ queryKey: notificationsKey(workspaceId) });
      }
    },
  });
}

export function useMarkAllRead() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (category?: string) => markAllNotificationsRead(workspaceId!, category),
    onSuccess: (updated) => {
      if (workspaceId) {
        void queryClient.invalidateQueries({ queryKey: notificationsKey(workspaceId) });
      }
      if (updated > 0) {
        toast.success(
          updated === 1 ? 'Marked 1 notification as read.' : `Marked ${updated} notifications as read.`,
        );
      }
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Failed to mark notifications as read.'));
    },
  });
}

export function useBulkNotifications() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ action, ids }: { action: BulkNotificationAction; ids: string[] }) =>
      bulkUpdateNotifications(workspaceId!, action, ids),
    onSuccess: (updated, { action }) => {
      if (workspaceId) {
        void queryClient.invalidateQueries({ queryKey: notificationsKey(workspaceId) });
      }
      const verb =
        action === 'archive' ? 'Archived' : action === 'unread' ? 'Marked unread' : 'Marked read';
      toast.success(`${verb} ${updated} notification${updated === 1 ? '' : 's'}.`);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Bulk update failed.'));
    },
  });
}

export function useNotificationPreferences() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: notificationPreferencesKey(workspaceId ?? ''),
    queryFn: () => getNotificationPreferences(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useSaveNotificationPreferences() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (preferences: {
      mutedUntil?: string | null;
      disabledCategories: string[];
      minSeverity: string;
    }) => putNotificationPreferences(workspaceId!, preferences),
    onSuccess: (saved: NotificationPreferences) => {
      if (workspaceId) {
        queryClient.setQueryData(notificationPreferencesKey(workspaceId), saved);
      }
      toast.success('Notification preferences saved.');
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Failed to save notification preferences.'));
    },
  });
}
