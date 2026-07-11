import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import { HTTPError } from 'ky';
import { toast } from 'sonner';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import type { RetentionPolicy } from '../../../types/artifact';
import {
  deleteArtifact,
  getArtifactCatalog,
  getArtifactDetail,
  getArtifactDownloadUrl,
  getArtifactsSummary,
  getRetentionPolicies,
  putRetentionPolicies,
  type ArtifactCatalogFilters,
} from '../api/artifactsApi';

export { useWorkspaceId };

export const artifactCatalogKey = (workspaceId: string, filters: ArtifactCatalogFilters = {}) =>
  [
    'workspaces',
    workspaceId,
    'artifacts',
    {
      repositoryId: filters.repositoryId ?? '',
      workflowId: filters.workflowId ?? '',
      pipelineId: filters.pipelineId ?? '',
      jobId: filters.jobId ?? '',
      status: filters.status ?? '',
      q: filters.q ?? '',
      branch: filters.branch ?? '',
      kind: filters.kind ?? '',
      retention: filters.retention ?? '',
      minSize: filters.minSize ?? '',
      maxSize: filters.maxSize ?? '',
      job: filters.job ?? '',
      createdAfter: filters.createdAfter ?? '',
      createdBefore: filters.createdBefore ?? '',
    },
  ] as const;
export const retentionPoliciesKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'artifacts', 'retention'] as const;
export const artifactsSummaryKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'artifacts', 'summary'] as const;
export const artifactDetailKey = (workspaceId: string, artifactId: string) =>
  ['workspaces', workspaceId, 'artifacts', 'detail', artifactId] as const;

function describeError(error: unknown, fallback: string): string {
  if (error instanceof HTTPError) {
    if (error.response.status === 409) return 'That action conflicts with the current state.';
    if (error.response.status === 422) return 'The request was rejected as invalid.';
    if (error.response.status === 429) return 'Too many attempts — please wait a moment.';
  }
  return fallback;
}

export function useArtifactsCatalog(filters: ArtifactCatalogFilters = {}) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: artifactCatalogKey(workspaceId ?? '', filters),
    queryFn: ({ pageParam }) =>
      getArtifactCatalog(workspaceId!, { ...filters, cursor: pageParam || undefined }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId),
    // Freshly produced artifacts confirm within seconds; poll only while a
    // visible row is still uploading.
    refetchInterval: (query) =>
      query.state.data?.pages.some((page) =>
        page.artifacts.some((artifact) => artifact.status === 'pending'),
      )
        ? 5000
        : false,
  });
}

export function useArtifactsSummary() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: artifactsSummaryKey(workspaceId ?? ''),
    queryFn: () => getArtifactsSummary(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useArtifactDetail(artifactId: string | undefined) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: artifactDetailKey(workspaceId ?? '', artifactId ?? ''),
    queryFn: () => getArtifactDetail(workspaceId!, artifactId!),
    enabled: Boolean(workspaceId && artifactId),
    refetchInterval: (query) =>
      query.state.data?.artifact.status === 'pending' ? 5000 : false,
  });
}

export function useRetentionPolicies() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: retentionPoliciesKey(workspaceId ?? ''),
    queryFn: () => getRetentionPolicies(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useSaveRetentionPolicies() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (policies: RetentionPolicy[]) => putRetentionPolicies(workspaceId!, policies),
    onSuccess: (data) => {
      toast.success('Retention policies saved.');
      if (!workspaceId) return;
      queryClient.setQueryData(retentionPoliciesKey(workspaceId), data);
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not save retention policies.'));
    },
  });
}

export function useDownloadArtifact() {
  const workspaceId = useWorkspaceId();
  return useMutation({
    mutationFn: (artifactId: string) => getArtifactDownloadUrl(workspaceId!, artifactId),
    onSuccess: (url) => {
      // Presigned R2 URL: hand it straight to the browser.
      window.location.assign(url);
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not prepare the download.'));
    },
  });
}

export function useDeleteArtifact() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (artifactId: string) => deleteArtifact(workspaceId!, artifactId),
    onSuccess: () => {
      toast.success('Artifact deleted.');
      if (!workspaceId) return;
      void queryClient.invalidateQueries({
        queryKey: ['workspaces', workspaceId, 'artifacts'],
      });
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not delete the artifact.'));
    },
  });
}
