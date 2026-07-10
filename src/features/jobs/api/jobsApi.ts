import { api } from '../../../lib/api';
import type { QueueListResponse, QueueSummary } from '../../../types/job';

export interface QueueFilters {
  /** Empty = both active statuses (queued and in_progress). */
  status?: 'queued' | 'in_progress' | '';
  repositoryId?: string;
  workflowId?: string;
  runnerId?: string;
  /** Single exact runs-on label. */
  label?: string;
  /** Free-text search over job key/name and workflow name. */
  q?: string;
  cursor?: string;
  /** Clamped 1-100 server-side; omitted uses the server's default page size. */
  limit?: number;
}

export async function getQueueJobs(
  workspaceId: string,
  filters: QueueFilters = {},
): Promise<QueueListResponse> {
  const searchParams = new URLSearchParams();
  if (filters.status) searchParams.set('status', filters.status);
  if (filters.repositoryId) searchParams.set('repositoryId', filters.repositoryId);
  if (filters.workflowId) searchParams.set('workflowId', filters.workflowId);
  if (filters.runnerId) searchParams.set('runnerId', filters.runnerId);
  if (filters.label) searchParams.set('label', filters.label);
  if (filters.q) searchParams.set('q', filters.q);
  if (filters.cursor) searchParams.set('cursor', filters.cursor);
  if (filters.limit) searchParams.set('limit', String(filters.limit));
  return api
    .get(`/api/workspaces/${workspaceId}/jobs`, { searchParams })
    .json<QueueListResponse>();
}

export async function getQueueSummary(workspaceId: string): Promise<QueueSummary> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/jobs/summary`)
    .json<{ summary: QueueSummary }>();
  return body.summary;
}
