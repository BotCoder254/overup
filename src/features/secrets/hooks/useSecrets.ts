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
  createSecret,
  deleteSecret,
  getSecretDetail,
  getSecretsAudit,
  getSecretsCatalog,
  getSecretsSummary,
  replaceSecretValue,
  updateSecretDescription,
  type CreateSecretInput,
  type SecretsCatalogFilters,
} from '../api/secretsApi';

export { useWorkspaceId };

export const secretsCatalogKey = (workspaceId: string, filters: SecretsCatalogFilters = {}) =>
  [
    'workspaces',
    workspaceId,
    'secrets',
    {
      q: filters.q ?? '',
      scope: filters.scope ?? '',
      repositoryId: filters.repositoryId ?? '',
      environmentId: filters.environmentId ?? '',
    },
  ] as const;
export const secretsSummaryKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'secrets', 'summary'] as const;
export const secretsAuditKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'secrets', 'audit'] as const;
export const secretDetailKey = (workspaceId: string, secretId: string) =>
  ['workspaces', workspaceId, 'secrets', 'detail', secretId] as const;

/**
 * 422 bodies carry the server's specific validation message (e.g. which
 * name rule failed); everything else maps to stable copy.
 */
async function describeError(error: unknown, fallback: string): Promise<string> {
  if (error instanceof HTTPError) {
    if (error.response.status === 409) {
      return 'A secret with this name already exists in this scope.';
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
    if (error.response.status === 403) return 'You do not have permission to manage secrets.';
  }
  return fallback;
}

export function useSecretsCatalog(filters: SecretsCatalogFilters = {}) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: secretsCatalogKey(workspaceId ?? '', filters),
    queryFn: ({ pageParam }) =>
      getSecretsCatalog(workspaceId!, { ...filters, cursor: pageParam || undefined }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId),
  });
}

export function useSecretsSummary() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: secretsSummaryKey(workspaceId ?? ''),
    queryFn: () => getSecretsSummary(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useSecretsAudit() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: secretsAuditKey(workspaceId ?? ''),
    queryFn: () => getSecretsAudit(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useSecretDetail(secretId: string | undefined) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: secretDetailKey(workspaceId ?? '', secretId ?? ''),
    queryFn: () => getSecretDetail(workspaceId!, secretId!),
    enabled: Boolean(workspaceId && secretId),
  });
}

function invalidateSecrets(
  queryClient: ReturnType<typeof useQueryClient>,
  workspaceId: string | undefined,
) {
  if (!workspaceId) return;
  void queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'secrets'] });
}

export function useCreateSecret() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateSecretInput) => createSecret(workspaceId!, input),
    onSuccess: (data) => {
      toast.success(`Secret ${data.secret.name} created — its value is now encrypted.`);
      invalidateSecrets(queryClient, workspaceId);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not create the secret.'));
    },
  });
}

export function useReplaceSecretValue() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ secretId, value }: { secretId: string; value: string }) =>
      replaceSecretValue(workspaceId!, secretId, value),
    onSuccess: (data) => {
      toast.success(`Value replaced for ${data.secret.name}.`);
      invalidateSecrets(queryClient, workspaceId);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not replace the value.'));
    },
  });
}

export function useUpdateSecretDescription() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ secretId, description }: { secretId: string; description: string }) =>
      updateSecretDescription(workspaceId!, secretId, description),
    onSuccess: () => {
      toast.success('Description updated.');
      invalidateSecrets(queryClient, workspaceId);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not update the description.'));
    },
  });
}

export function useDeleteSecret() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (secretId: string) => deleteSecret(workspaceId!, secretId),
    onSuccess: () => {
      toast.success('Secret deleted. Pipelines that relied on it will no longer receive it.');
      invalidateSecrets(queryClient, workspaceId);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not delete the secret.'));
    },
  });
}
