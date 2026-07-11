import { api } from '../../../lib/api';
import type {
  ArtifactCatalogResponse,
  ArtifactDetail,
  ArtifactsSummary,
  RetentionPoliciesResponse,
  RetentionPolicy,
} from '../../../types/artifact';

export interface ArtifactCatalogFilters {
  repositoryId?: string;
  workflowId?: string;
  pipelineId?: string;
  jobId?: string;
  status?: string;
  q?: string;
  branch?: string;
  kind?: string;
  retention?: string;
  /** Bytes, stringified — the page converts from MB inputs. */
  minSize?: string;
  maxSize?: string;
  job?: string;
  createdAfter?: string;
  createdBefore?: string;
  cursor?: string;
}

export async function getArtifactCatalog(
  workspaceId: string,
  filters: ArtifactCatalogFilters = {},
): Promise<ArtifactCatalogResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value) params.set(key, value);
  }
  return api
    .get(`/api/workspaces/${workspaceId}/artifacts`, { searchParams: params })
    .json<ArtifactCatalogResponse>();
}

export async function getArtifactsSummary(workspaceId: string): Promise<ArtifactsSummary> {
  return api.get(`/api/workspaces/${workspaceId}/artifacts/summary`).json<ArtifactsSummary>();
}

export async function getArtifactDetail(
  workspaceId: string,
  artifactId: string,
): Promise<ArtifactDetail> {
  return api
    .get(`/api/workspaces/${workspaceId}/artifacts/${artifactId}`)
    .json<ArtifactDetail>();
}

export async function getRetentionPolicies(
  workspaceId: string,
): Promise<RetentionPoliciesResponse> {
  return api
    .get(`/api/workspaces/${workspaceId}/artifacts/retention`)
    .json<RetentionPoliciesResponse>();
}

export async function putRetentionPolicies(
  workspaceId: string,
  policies: RetentionPolicy[],
): Promise<RetentionPoliciesResponse> {
  return api
    .put(`/api/workspaces/${workspaceId}/artifacts/retention`, { json: { policies } })
    .json<RetentionPoliciesResponse>();
}

/** Same presigned-GET endpoint the pipeline panel uses. */
export async function getArtifactDownloadUrl(
  workspaceId: string,
  artifactId: string,
): Promise<string> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/artifacts/${artifactId}/download`)
    .json<{ url: string }>();
  return body.url;
}

export async function deleteArtifact(workspaceId: string, artifactId: string): Promise<void> {
  await api.delete(`/api/workspaces/${workspaceId}/artifacts/${artifactId}`);
}
