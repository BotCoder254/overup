import { api } from '../../../lib/api';
import type { ActivityListResponse, ActivitySummary } from '../../../types/activity';

export interface ActivityFeedFilters {
  q?: string;
  category?: string;
  action?: string;
  /** Narrow to one actor (uuid) — applied by clicking an actor in the feed. */
  actorId?: string;
  /** RFC3339 bounds (already day-bounded by the page). */
  createdAfter?: string;
  createdBefore?: string;
  cursor?: string;
}

export async function getActivityFeed(
  workspaceId: string,
  filters: ActivityFeedFilters = {},
): Promise<ActivityListResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value) params.set(key, value);
  }
  return api
    .get(`/api/workspaces/${workspaceId}/activity`, { searchParams: params })
    .json<ActivityListResponse>();
}

export async function getActivitySummary(workspaceId: string): Promise<ActivitySummary> {
  return api.get(`/api/workspaces/${workspaceId}/activity/summary`).json<ActivitySummary>();
}

/**
 * Compliance export of the (filtered) feed as CSV. The server re-validates
 * every filter, caps the row count, and hardens the CSV against formula
 * injection — the blob here is download-ready.
 */
export async function exportActivityFeed(
  workspaceId: string,
  filters: ActivityFeedFilters = {},
): Promise<Blob> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value) params.set(key, value);
  }
  return api
    .get(`/api/workspaces/${workspaceId}/activity/export`, { searchParams: params })
    .blob();
}
