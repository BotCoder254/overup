import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { getDashboardActivity, getDashboardSummary } from '../api/dashboardApi';
import { getActivityFeed } from '../../activity/api/activityApi';
import { getPipelines } from '../../pipelines/api/pipelinesApi';
import type { DashboardRange } from '../../../types/dashboard';

export { useWorkspaceId };

export const dashboardSummaryKey = (workspaceId: string, range: DashboardRange) =>
  ['workspaces', workspaceId, 'dashboard', 'summary', range] as const;
export const dashboardActivityKey = (workspaceId: string, range: DashboardRange) =>
  ['workspaces', workspaceId, 'dashboard', 'activity', range] as const;
export const dashboardRecentPipelinesKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'dashboard', 'recentPipelines'] as const;
export const dashboardActivityFeedKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'dashboard', 'activityFeed'] as const;

const RECENT_PIPELINES_PAGE = 15;
/** The dashboard Activity rail shows five events per page. */
const ACTIVITY_FEED_PAGE = 5;
/** Fallback poll while the live workspace stream is disconnected. */
const pollWhileDisconnected = 10_000;

export function useDashboardSummary(range: DashboardRange, connected: boolean) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: dashboardSummaryKey(workspaceId ?? '', range),
    queryFn: () => getDashboardSummary(workspaceId!, range),
    enabled: Boolean(workspaceId),
    refetchInterval: connected ? false : pollWhileDisconnected,
  });
}

export function useDashboardActivity(range: DashboardRange, connected: boolean) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: dashboardActivityKey(workspaceId ?? '', range),
    queryFn: () => getDashboardActivity(workspaceId!, range),
    enabled: Boolean(workspaceId),
    refetchInterval: connected ? false : pollWhileDisconnected,
  });
}

/**
 * Recent pipelines as a keyset-paginated infinite list (the dashboard's
 * full-width data table). The workspace stream invalidates
 * `dashboardRecentPipelinesKey` on `pipeline_update`; keeping the same key
 * root means invalidation still lands and refetches from the first page —
 * the freshest runs stay on top.
 */
export function useDashboardRecentPipelines(connected: boolean) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: dashboardRecentPipelinesKey(workspaceId ?? ''),
    queryFn: ({ pageParam }) =>
      getPipelines(workspaceId!, {
        limit: RECENT_PIPELINES_PAGE,
        cursor: (pageParam as string) || undefined,
      }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId),
    refetchInterval: connected ? false : pollWhileDisconnected,
  });
}

/**
 * Compact activity feed for the dashboard rail — five events per page, with
 * its own cache key so it never collides with the full Activity page (which
 * requests the server-default page size). Stream-gated polling like the rest.
 */
export function useDashboardActivityFeed(connected: boolean) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: dashboardActivityFeedKey(workspaceId ?? ''),
    queryFn: ({ pageParam }) =>
      getActivityFeed(workspaceId!, {
        limit: ACTIVITY_FEED_PAGE,
        cursor: (pageParam as string) || undefined,
      }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId),
    refetchInterval: connected ? false : pollWhileDisconnected,
  });
}
