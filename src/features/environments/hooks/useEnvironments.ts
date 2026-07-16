import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import { HTTPError } from 'ky';
import { toast } from 'sonner';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import {
  createEnvironment,
  deleteEnvironment,
  getEnvironmentDetail,
  getEnvironmentsAudit,
  getEnvironmentsCatalog,
  getEnvironmentsRequirements,
  getEnvironmentsSummary,
  updateEnvironment,
  type CreateEnvironmentInput,
  type EnvironmentsCatalogFilters,
  type UpdateEnvironmentInput,
} from '../api/environmentsApi';

export { useWorkspaceId };

export const environmentsCatalogKey = (
  workspaceId: string,
  filters: EnvironmentsCatalogFilters = {},
) => ['workspaces', workspaceId, 'environments', { q: filters.q ?? '' }] as const;
export const environmentsSummaryKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'environments', 'summary'] as const;
export const environmentsAuditKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'environments', 'audit'] as const;
export const environmentsRequirementsKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'environments', 'requirements'] as const;
export const environmentDetailKey = (workspaceId: string, environmentId: string) =>
  ['workspaces', workspaceId, 'environments', 'detail', environmentId] as const;

/** 422 bodies carry the server's specific validation message. */
async function describeError(error: unknown, fallback: string): Promise<string> {
  if (error instanceof HTTPError) {
    if (error.response.status === 409) {
      return 'An environment with this name already exists.';
    }
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
      return 'You do not have permission to manage environments.';
    }
  }
  return fallback;
}

export function useEnvironmentsCatalog(filters: EnvironmentsCatalogFilters = {}) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: environmentsCatalogKey(workspaceId ?? '', filters),
    queryFn: ({ pageParam }) =>
      getEnvironmentsCatalog(workspaceId!, { ...filters, cursor: pageParam || undefined }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId),
  });
}

export function useEnvironmentsSummary() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: environmentsSummaryKey(workspaceId ?? ''),
    queryFn: () => getEnvironmentsSummary(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

/**
 * YAML-bound environment names with no matching row. Lives under the
 * environments prefix, so `invalidateEnvironments` clears it after every
 * mutation — a detected entry disappears as soon as it's created.
 */
export function useEnvironmentsRequirements() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: environmentsRequirementsKey(workspaceId ?? ''),
    queryFn: () => getEnvironmentsRequirements(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useEnvironmentsAudit() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: environmentsAuditKey(workspaceId ?? ''),
    queryFn: () => getEnvironmentsAudit(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useEnvironmentDetail(environmentId: string | undefined) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: environmentDetailKey(workspaceId ?? '', environmentId ?? ''),
    queryFn: () => getEnvironmentDetail(workspaceId!, environmentId!),
    enabled: Boolean(workspaceId && environmentId),
  });
}

/**
 * Coarse prefix invalidation. Mutations also invalidate the secrets prefix:
 * environment deletes cascade to their secrets, and renames change the
 * environmentName shown on secret rows.
 */
function invalidateEnvironments(
  queryClient: ReturnType<typeof useQueryClient>,
  workspaceId: string | undefined,
) {
  if (!workspaceId) return;
  void queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'environments'] });
  void queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'secrets'] });
}

export function useCreateEnvironment() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateEnvironmentInput) => createEnvironment(workspaceId!, input),
    onSuccess: (data) => {
      toast.success(`Environment ${data.environment.name} created.`);
      invalidateEnvironments(queryClient, workspaceId);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not create the environment.'));
    },
  });
}

export function useUpdateEnvironment() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      environmentId,
      ...input
    }: UpdateEnvironmentInput & { environmentId: string }) =>
      updateEnvironment(workspaceId!, environmentId, input),
    onSuccess: () => {
      toast.success('Environment updated.');
      invalidateEnvironments(queryClient, workspaceId);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not update the environment.'));
    },
  });
}

export function useDeleteEnvironment() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (environmentId: string) => deleteEnvironment(workspaceId!, environmentId),
    onSuccess: (data) => {
      toast.success(
        data.deletedSecrets > 0
          ? `Environment deleted along with ${data.deletedSecrets} scoped secret${
              data.deletedSecrets === 1 ? '' : 's'
            }.`
          : 'Environment deleted.',
      );
      invalidateEnvironments(queryClient, workspaceId);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not delete the environment.'));
    },
  });
}
