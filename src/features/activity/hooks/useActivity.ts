import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import {
  getActivityFeed,
  getActivitySummary,
  type ActivityFeedFilters,
} from '../api/activityApi';

export { useWorkspaceId };

export const activityFeedKey = (workspaceId: string, filters: ActivityFeedFilters = {}) =>
  [
    'workspaces',
    workspaceId,
    'activity',
    {
      q: filters.q ?? '',
      category: filters.category ?? '',
      action: filters.action ?? '',
      actorId: filters.actorId ?? '',
      createdAfter: filters.createdAfter ?? '',
      createdBefore: filters.createdBefore ?? '',
    },
  ] as const;
export const activitySummaryKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'activity', 'summary'] as const;

/**
 * Keyset-paginated activity feed. While the workspace stream is connected,
 * frames drive invalidation; disconnected, a slow poll keeps the ledger
 * moving (the stream-gated polling pattern from the dashboard).
 */
export function useActivityFeed(filters: ActivityFeedFilters = {}, connected = false) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: activityFeedKey(workspaceId ?? '', filters),
    queryFn: ({ pageParam }) =>
      getActivityFeed(workspaceId!, { ...filters, cursor: pageParam || undefined }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId),
    refetchInterval: connected ? false : 30_000,
  });
}

export function useActivitySummary(connected = false) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: activitySummaryKey(workspaceId ?? ''),
    queryFn: () => getActivitySummary(workspaceId!),
    enabled: Boolean(workspaceId),
    refetchInterval: connected ? false : 30_000,
  });
}
