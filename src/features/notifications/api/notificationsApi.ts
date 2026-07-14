import { api } from '../../../lib/api';
import type {
  NotificationListResponse,
  NotificationPreferences,
} from '../../../types/notification';

export interface NotificationFilters {
  /** true = unread only, false = read only, omitted = both. */
  unread?: boolean;
  category?: string;
  severity?: string;
  repositoryId?: string;
  q?: string;
  createdAfter?: string;
  createdBefore?: string;
  /** 'exclude' (default) | 'include' | 'only'. */
  archived?: string;
  cursor?: string;
}

export async function listNotifications(
  workspaceId: string,
  filters: NotificationFilters = {},
): Promise<NotificationListResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value !== undefined && value !== null && value !== '') params.set(key, String(value));
  }
  return api
    .get(`/api/workspaces/${workspaceId}/notifications`, { searchParams: params })
    .json<NotificationListResponse>();
}

/**
 * CSV export of the caller's (filtered) notifications. The server
 * re-validates every filter, caps the row count, and hardens the CSV
 * against formula injection — the blob here is download-ready.
 */
export async function exportNotifications(
  workspaceId: string,
  filters: NotificationFilters = {},
): Promise<Blob> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value !== undefined && value !== null && value !== '') params.set(key, String(value));
  }
  return api
    .get(`/api/workspaces/${workspaceId}/notifications/export`, { searchParams: params })
    .blob();
}

export async function getUnreadCount(workspaceId: string): Promise<number> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/notifications/unread-count`)
    .json<{ count: number }>();
  return body.count;
}

export async function markNotificationRead(
  workspaceId: string,
  notificationId: string,
): Promise<void> {
  await api.post(`/api/workspaces/${workspaceId}/notifications/${notificationId}/read`);
}

export async function markAllNotificationsRead(
  workspaceId: string,
  category?: string,
): Promise<number> {
  const body = await api
    .post(`/api/workspaces/${workspaceId}/notifications/read-all`, {
      json: category ? { category } : {},
    })
    .json<{ updated: number }>();
  return body.updated;
}

export type BulkNotificationAction = 'read' | 'unread' | 'archive';

export async function bulkUpdateNotifications(
  workspaceId: string,
  action: BulkNotificationAction,
  ids: string[],
): Promise<number> {
  const body = await api
    .post(`/api/workspaces/${workspaceId}/notifications/bulk`, { json: { action, ids } })
    .json<{ updated: number }>();
  return body.updated;
}

export async function getNotificationPreferences(
  workspaceId: string,
): Promise<NotificationPreferences> {
  return api
    .get(`/api/workspaces/${workspaceId}/notification-preferences`)
    .json<NotificationPreferences>();
}

export async function putNotificationPreferences(
  workspaceId: string,
  preferences: {
    mutedUntil?: string | null;
    disabledCategories: string[];
    minSeverity: string;
  },
): Promise<NotificationPreferences> {
  return api
    .put(`/api/workspaces/${workspaceId}/notification-preferences`, {
      json: {
        mutedUntil: preferences.mutedUntil ?? undefined,
        disabledCategories: preferences.disabledCategories,
        minSeverity: preferences.minSeverity,
      },
    })
    .json<NotificationPreferences>();
}
