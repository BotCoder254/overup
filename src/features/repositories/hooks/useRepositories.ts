import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { HTTPError } from 'ky';
import { toast } from 'sonner';
import { useMe } from '../../auth/hooks/useAuth';
import {
  getAvailableRepositories,
  getInstallations,
  getRepositories,
  getRepositoryDetail,
  getRepositoryEvents,
  importRepository,
  removeRepository,
  syncRepository,
} from '../api/repositoriesApi';

/** The workspace id every repository query is scoped to (guards guarantee it). */
export function useWorkspaceId(): string | undefined {
  const { data: me } = useMe();
  return me?.workspace?.id;
}

export const repositoriesKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'repositories'] as const;
export const availableKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'repositories', 'available'] as const;
export const repositoryKey = (workspaceId: string, repositoryId: string) =>
  ['workspaces', workspaceId, 'repositories', 'detail', repositoryId] as const;
export const installationsKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'installations'] as const;
export const repositoryEventsKey = (workspaceId: string, repositoryId: string) =>
  ['workspaces', workspaceId, 'repositories', 'detail', repositoryId, 'events'] as const;

/** Poll every few seconds while any repository is mid-sync. */
const pollWhileSyncing = 3000;

export function useInstallations() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: installationsKey(workspaceId ?? ''),
    queryFn: () => getInstallations(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useRepositories() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: repositoriesKey(workspaceId ?? ''),
    queryFn: () => getRepositories(workspaceId!),
    enabled: Boolean(workspaceId),
    refetchInterval: (query) =>
      query.state.data?.some((repo) => repo.syncStatus === 'syncing' || repo.syncStatus === 'pending')
        ? pollWhileSyncing
        : false,
  });
}

export function useAvailableRepositories(enabled: boolean) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: availableKey(workspaceId ?? ''),
    queryFn: () => getAvailableRepositories(workspaceId!),
    enabled: Boolean(workspaceId) && enabled,
    // Live proxy over GitHub — noticeably slower than our own queries.
    staleTime: 60 * 1000,
  });
}

export function useRepositoryDetail(repositoryId: string | undefined) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: repositoryKey(workspaceId ?? '', repositoryId ?? ''),
    queryFn: () => getRepositoryDetail(workspaceId!, repositoryId!),
    enabled: Boolean(workspaceId && repositoryId),
    refetchInterval: (query) => {
      const status = query.state.data?.repository.syncStatus;
      return status === 'syncing' || status === 'pending' ? pollWhileSyncing : false;
    },
  });
}

/** Keyset-paginated repository event timeline (the Events tab). Refreshes
 * alongside the detail poll while a sync is running via `enabled` timing —
 * a modest refetchInterval keeps the timeline live the rest of the time. */
export function useRepositoryEvents(repositoryId: string | undefined, enabled = true) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: repositoryEventsKey(workspaceId ?? '', repositoryId ?? ''),
    queryFn: ({ pageParam }) =>
      getRepositoryEvents(workspaceId!, repositoryId!, pageParam || undefined),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId && repositoryId) && enabled,
    refetchInterval: 15000,
  });
}

function describeError(error: unknown, fallback: string): string {
  if (error instanceof HTTPError) {
    if (error.response.status === 409) return 'That action conflicts with the current state.';
    if (error.response.status === 429) return 'Too many attempts — please wait a moment.';
  }
  return fallback;
}

export function useImportRepository() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: { installationId: string; githubRepoId: number; fullName: string }) =>
      importRepository(workspaceId!, {
        installationId: input.installationId,
        githubRepoId: input.githubRepoId,
      }),
    onSuccess: (_repo, input) => {
      toast.success(`Importing ${input.fullName} — the first sync is running.`);
      if (workspaceId) {
        void queryClient.invalidateQueries({ queryKey: repositoriesKey(workspaceId) });
        void queryClient.invalidateQueries({ queryKey: availableKey(workspaceId) });
      }
    },
    onError: (error) => {
      toast.error(describeError(error, 'Import failed. Please try again.'));
    },
  });
}

export function useSyncRepository() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (repositoryId: string) => syncRepository(workspaceId!, repositoryId),
    onSuccess: (_data, repositoryId) => {
      toast.success('Sync started.');
      if (workspaceId) {
        void queryClient.invalidateQueries({ queryKey: repositoriesKey(workspaceId) });
        void queryClient.invalidateQueries({ queryKey: repositoryKey(workspaceId, repositoryId) });
      }
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not start the sync.'));
    },
  });
}

export function useRemoveRepository() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (repositoryId: string) => removeRepository(workspaceId!, repositoryId),
    onSuccess: () => {
      toast.success('Repository removed from the workspace.');
      if (workspaceId) {
        void queryClient.invalidateQueries({ queryKey: repositoriesKey(workspaceId) });
        void queryClient.invalidateQueries({ queryKey: availableKey(workspaceId) });
      }
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not remove the repository.'));
    },
  });
}
