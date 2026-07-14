import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { getQueueJobs, getQueueSummary, type QueueFilters } from '../api/jobsApi';

export { useWorkspaceId };
export type { QueueFilters };

/** Keys share the ['workspaces', ws, 'jobs'] prefix so one coarse
 * invalidation (workspace stream, job cancel) refreshes both. */
export const jobsQueueKey = (workspaceId: string, filters: QueueFilters = {}) =>
  [
    'workspaces',
    workspaceId,
    'jobs',
    'queue',
    {
      status: filters.status ?? '',
      repositoryId: filters.repositoryId ?? '',
      workflowId: filters.workflowId ?? '',
      runnerId: filters.runnerId ?? '',
      label: filters.label ?? '',
      q: filters.q ?? '',
    },
  ] as const;
export const jobsSummaryKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'jobs', 'summary'] as const;

/** Every row in the queue is active by definition. While the workspace
 * stream is live it already invalidates on every pipeline transition, so a
 * slow keep-fresh poll (wait-time aggregates advance without any event) is
 * enough; while disconnected, polling is the only freshness source — poll
 * briskly when anything is active, occasionally when the queue is empty. */
const pollWhileConnected = 15000;
const pollWhileActive = 5000;
const pollWhileEmpty = 30000;

export function useQueueJobs(filters: QueueFilters = {}, connected = false) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: jobsQueueKey(workspaceId ?? '', filters),
    queryFn: ({ pageParam }) =>
      getQueueJobs(workspaceId!, { ...filters, cursor: pageParam || undefined }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId),
    refetchInterval: (query) =>
      connected
        ? pollWhileConnected
        : query.state.data?.pages.some((page) => page.jobs.length > 0)
          ? pollWhileActive
          : pollWhileEmpty,
  });
}

export function useQueueSummary(connected = false) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: jobsSummaryKey(workspaceId ?? ''),
    queryFn: () => getQueueSummary(workspaceId!),
    enabled: Boolean(workspaceId),
    refetchInterval: connected ? pollWhileConnected : pollWhileActive,
  });
}
