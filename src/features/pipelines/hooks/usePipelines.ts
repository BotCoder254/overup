import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import { HTTPError } from 'ky';
import { toast } from 'sonner';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import type { PipelineDetail } from '../../../types/pipeline';
import {
  cancelJob,
  cancelPipeline,
  dispatchWorkflow,
  getArtifactDownloadUrl,
  getArtifacts,
  getJobDetail,
  getPipelineDetail,
  getPipelines,
  rerunPipeline,
  type PipelineFilters,
} from '../api/pipelinesApi';

export const pipelinesKey = (workspaceId: string, filters: PipelineFilters = {}) =>
  [
    'workspaces',
    workspaceId,
    'pipelines',
    {
      repositoryId: filters.repositoryId ?? '',
      workflowId: filters.workflowId ?? '',
      status: filters.status ?? '',
      conclusion: filters.conclusion ?? '',
      trigger: filters.trigger ?? '',
      branch: filters.branch ?? '',
      q: filters.q ?? '',
      createdAfter: filters.createdAfter ?? '',
      createdBefore: filters.createdBefore ?? '',
    },
  ] as const;
export const pipelineKey = (workspaceId: string, pipelineId: string) =>
  ['workspaces', workspaceId, 'pipelines', 'detail', pipelineId] as const;
export const artifactsKey = (workspaceId: string, pipelineId: string) =>
  ['workspaces', workspaceId, 'pipelines', 'detail', pipelineId, 'artifacts'] as const;
export const jobKey = (workspaceId: string, pipelineId: string, jobId: string) =>
  ['workspaces', workspaceId, 'pipelines', 'detail', pipelineId, 'jobs', jobId] as const;

/** Poll every few seconds while executions are live. */
const pollWhileActive = 3000;

export function usePipelines(filters: PipelineFilters = {}) {
  const workspaceId = useWorkspaceId();
  return useInfiniteQuery({
    queryKey: pipelinesKey(workspaceId ?? '', filters),
    queryFn: ({ pageParam }) =>
      getPipelines(workspaceId!, { ...filters, cursor: pageParam || undefined }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId),
    refetchInterval: (query) =>
      query.state.data?.pages.some((page) =>
        page.pipelines.some((pipeline) => pipeline.status !== 'completed'),
      )
        ? pollWhileActive
        : false,
  });
}

/**
 * Pipeline detail. The WebSocket stream patches this cache live; polling is
 * the fallback and only runs while the stream is down and the pipeline is
 * not finished.
 */
export function usePipelineDetail(pipelineId: string | undefined, streamConnected: boolean) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: pipelineKey(workspaceId ?? '', pipelineId ?? ''),
    queryFn: () => getPipelineDetail(workspaceId!, pipelineId!),
    enabled: Boolean(workspaceId && pipelineId),
    refetchInterval: (query) => {
      if (streamConnected) return false;
      return query.state.data?.pipeline.status !== 'completed' ? pollWhileActive : false;
    },
  });
}

/**
 * Per-job identity payload (runner summary, job-scoped events/artifacts).
 * The pipeline stream keeps job state itself live; this poll exists for
 * runner health, which is not on the per-pipeline socket.
 */
export function useJobDetail(pipelineId: string | undefined, jobId: string | undefined) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: jobKey(workspaceId ?? '', pipelineId ?? '', jobId ?? ''),
    queryFn: () => getJobDetail(workspaceId!, pipelineId!, jobId!),
    enabled: Boolean(workspaceId && pipelineId && jobId),
    refetchInterval: (query) =>
      query.state.data?.job.status !== 'completed' ? 5000 : false,
  });
}

export function useArtifacts(pipelineId: string | undefined) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: artifactsKey(workspaceId ?? '', pipelineId ?? ''),
    queryFn: () => getArtifacts(workspaceId!, pipelineId!),
    enabled: Boolean(workspaceId && pipelineId),
  });
}

function describeError(error: unknown, fallback: string): string {
  if (error instanceof HTTPError) {
    if (error.response.status === 409) return 'That action conflicts with the current state.';
    if (error.response.status === 422) return 'The request was rejected as invalid.';
    if (error.response.status === 429) return 'Too many attempts — please wait a moment.';
  }
  return fallback;
}

export function useCancelPipeline() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (pipelineId: string) => cancelPipeline(workspaceId!, pipelineId),
    onSuccess: (_data, pipelineId) => {
      toast.success('Cancellation requested.');
      if (workspaceId) {
        void queryClient.invalidateQueries({ queryKey: pipelineKey(workspaceId, pipelineId) });
        void queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'pipelines'] });
      }
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not cancel the pipeline.'));
    },
  });
}

export function useCancelJob() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { pipelineId: string; jobId: string }) =>
      cancelJob(workspaceId!, input.pipelineId, input.jobId),
    onSuccess: (_data, input) => {
      toast.success('Job cancellation requested.');
      if (workspaceId) {
        void queryClient.invalidateQueries({
          queryKey: pipelineKey(workspaceId, input.pipelineId),
        });
        void queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'jobs'] });
      }
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not cancel the job.'));
    },
  });
}

export function useRerunPipeline() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (pipelineId: string) => rerunPipeline(workspaceId!, pipelineId),
    onSuccess: () => {
      toast.success('Re-run created.');
      if (workspaceId) {
        void queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'pipelines'] });
      }
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not re-run the pipeline.'));
    },
  });
}

export function useDispatchWorkflow() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { workflowId: string; branch?: string }) =>
      dispatchWorkflow(workspaceId!, input.workflowId, input.branch),
    onSuccess: () => {
      toast.success('Pipeline dispatched.');
      if (workspaceId) {
        void queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'pipelines'] });
      }
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not dispatch the workflow.'));
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

export type { PipelineDetail };
