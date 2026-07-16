import { api } from '../../../lib/api';
import type {
  Environment,
  EnvironmentAuditEvent,
  EnvironmentDetailResponse,
  EnvironmentsListResponse,
  EnvironmentsRequirements,
  EnvironmentsSummary,
} from '../../../types/environment';

export interface EnvironmentsCatalogFilters {
  q?: string;
  cursor?: string;
}

export async function getEnvironmentsCatalog(
  workspaceId: string,
  filters: EnvironmentsCatalogFilters = {},
): Promise<EnvironmentsListResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value) params.set(key, value);
  }
  return api
    .get(`/api/workspaces/${workspaceId}/environments`, { searchParams: params })
    .json<EnvironmentsListResponse>();
}

export async function getEnvironmentsSummary(workspaceId: string): Promise<EnvironmentsSummary> {
  return api
    .get(`/api/workspaces/${workspaceId}/environments/summary`)
    .json<EnvironmentsSummary>();
}

export async function getEnvironmentsRequirements(
  workspaceId: string,
): Promise<EnvironmentsRequirements> {
  return api
    .get(`/api/workspaces/${workspaceId}/environments/requirements`)
    .json<EnvironmentsRequirements>();
}

export async function getEnvironmentsAudit(
  workspaceId: string,
  limit = 20,
): Promise<{ events: EnvironmentAuditEvent[] }> {
  return api
    .get(`/api/workspaces/${workspaceId}/environments/audit`, {
      searchParams: { limit: String(limit) },
    })
    .json<{ events: EnvironmentAuditEvent[] }>();
}

export async function getEnvironmentDetail(
  workspaceId: string,
  environmentId: string,
): Promise<EnvironmentDetailResponse> {
  return api
    .get(`/api/workspaces/${workspaceId}/environments/${environmentId}`)
    .json<EnvironmentDetailResponse>();
}

export interface CreateEnvironmentInput {
  name: string;
  description?: string;
}

export async function createEnvironment(
  workspaceId: string,
  input: CreateEnvironmentInput,
): Promise<{ environment: Environment }> {
  return api
    .post(`/api/workspaces/${workspaceId}/environments`, { json: input })
    .json<{ environment: Environment }>();
}

export interface UpdateEnvironmentInput {
  name?: string;
  description?: string;
}

export async function updateEnvironment(
  workspaceId: string,
  environmentId: string,
  input: UpdateEnvironmentInput,
): Promise<{ environment: Environment }> {
  return api
    .patch(`/api/workspaces/${workspaceId}/environments/${environmentId}`, { json: input })
    .json<{ environment: Environment }>();
}

export async function deleteEnvironment(
  workspaceId: string,
  environmentId: string,
): Promise<{ deletedSecrets: number }> {
  return api
    .delete(`/api/workspaces/${workspaceId}/environments/${environmentId}`)
    .json<{ deletedSecrets: number }>();
}
