import { api } from '../../../lib/api';
import type {
  Artifact,
  Pipeline,
  PipelineDetail,
  PipelineListResponse,
  PipelineStatus,
} from '../../../types/pipeline';

export interface PipelineFilters {
  repositoryId?: string;
  workflowId?: string;
  status?: PipelineStatus | '';
  conclusion?: string;
  trigger?: string;
  branch?: string;
  /** Free-text search over commit message, SHA, and workflow name. */
  q?: string;
  /** RFC3339 timestamps. */
  createdAfter?: string;
  createdBefore?: string;
  cursor?: string;
}

export async function getPipelines(
  workspaceId: string,
  filters: PipelineFilters = {},
): Promise<PipelineListResponse> {
  const searchParams = new URLSearchParams();
  if (filters.repositoryId) searchParams.set('repositoryId', filters.repositoryId);
  if (filters.workflowId) searchParams.set('workflowId', filters.workflowId);
  if (filters.status) searchParams.set('status', filters.status);
  if (filters.conclusion) searchParams.set('conclusion', filters.conclusion);
  if (filters.trigger) searchParams.set('trigger', filters.trigger);
  if (filters.branch) searchParams.set('branch', filters.branch);
  if (filters.q) searchParams.set('q', filters.q);
  if (filters.createdAfter) searchParams.set('createdAfter', filters.createdAfter);
  if (filters.createdBefore) searchParams.set('createdBefore', filters.createdBefore);
  if (filters.cursor) searchParams.set('cursor', filters.cursor);
  return api
    .get(`/api/workspaces/${workspaceId}/pipelines`, { searchParams })
    .json<PipelineListResponse>();
}

export async function getPipelineDetail(
  workspaceId: string,
  pipelineId: string,
): Promise<PipelineDetail> {
  return api
    .get(`/api/workspaces/${workspaceId}/pipelines/${pipelineId}`)
    .json<PipelineDetail>();
}

export async function cancelPipeline(workspaceId: string, pipelineId: string): Promise<void> {
  await api.post(`/api/workspaces/${workspaceId}/pipelines/${pipelineId}/cancel`);
}

export async function rerunPipeline(
  workspaceId: string,
  pipelineId: string,
): Promise<Pipeline> {
  const body = await api
    .post(`/api/workspaces/${workspaceId}/pipelines/${pipelineId}/rerun`)
    .json<{ pipeline: Pipeline }>();
  return body.pipeline;
}

export async function dispatchWorkflow(
  workspaceId: string,
  workflowId: string,
  branch?: string,
): Promise<Pipeline> {
  const body = await api
    .post(`/api/workspaces/${workspaceId}/workflows/${workflowId}/dispatch`, {
      json: { branch: branch ?? null },
    })
    .json<{ pipeline: Pipeline }>();
  return body.pipeline;
}

/** One REST page of log chunks; page through until fewer than `LOG_PAGE`. */
export const LOG_PAGE = 2000;

export async function getJobLogs(
  workspaceId: string,
  pipelineId: string,
  jobId: string,
  fromSeq = 0,
): Promise<{
  chunks: { seq: number; stream: string; content: string; createdAt?: string }[];
  jobStatus: string;
}> {
  return api
    .get(
      `/api/workspaces/${workspaceId}/pipelines/${pipelineId}/jobs/${jobId}/logs`,
      { searchParams: { fromSeq: String(fromSeq), limit: String(LOG_PAGE) } },
    )
    .json();
}

export function rawLogUrl(workspaceId: string, pipelineId: string, jobId: string): string {
  return `/api/workspaces/${workspaceId}/pipelines/${pipelineId}/jobs/${jobId}/logs/raw`;
}

export async function getArtifacts(
  workspaceId: string,
  pipelineId: string,
): Promise<Artifact[]> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/pipelines/${pipelineId}/artifacts`)
    .json<{ artifacts: Artifact[] }>();
  return body.artifacts;
}

export async function getArtifactDownloadUrl(
  workspaceId: string,
  artifactId: string,
): Promise<string> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/artifacts/${artifactId}/download`)
    .json<{ url: string }>();
  return body.url;
}
