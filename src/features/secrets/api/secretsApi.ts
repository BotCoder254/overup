import { api } from '../../../lib/api';
import type {
  Secret,
  SecretDetailResponse,
  SecretsAuditResponse,
  SecretsListResponse,
  SecretsSummary,
} from '../../../types/secret';

export interface SecretsCatalogFilters {
  q?: string;
  scope?: string;
  repositoryId?: string;
  environmentId?: string;
  cursor?: string;
}

export async function getSecretsCatalog(
  workspaceId: string,
  filters: SecretsCatalogFilters = {},
): Promise<SecretsListResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value) params.set(key, value);
  }
  return api
    .get(`/api/workspaces/${workspaceId}/secrets`, { searchParams: params })
    .json<SecretsListResponse>();
}

export async function getSecretsSummary(workspaceId: string): Promise<SecretsSummary> {
  return api.get(`/api/workspaces/${workspaceId}/secrets/summary`).json<SecretsSummary>();
}

export async function getSecretsAudit(
  workspaceId: string,
  limit = 20,
): Promise<SecretsAuditResponse> {
  return api
    .get(`/api/workspaces/${workspaceId}/secrets/audit`, {
      searchParams: { limit: String(limit) },
    })
    .json<SecretsAuditResponse>();
}

export async function getSecretDetail(
  workspaceId: string,
  secretId: string,
): Promise<SecretDetailResponse> {
  return api
    .get(`/api/workspaces/${workspaceId}/secrets/${secretId}`)
    .json<SecretDetailResponse>();
}

export interface CreateSecretInput {
  name: string;
  /** Sent once over TLS; the server stores only ciphertext. */
  value: string;
  description?: string;
  /** Mutually exclusive with environmentId. */
  repositoryId?: string;
  environmentId?: string;
}

export async function createSecret(
  workspaceId: string,
  input: CreateSecretInput,
): Promise<{ secret: Secret }> {
  return api
    .post(`/api/workspaces/${workspaceId}/secrets`, { json: input })
    .json<{ secret: Secret }>();
}

export async function replaceSecretValue(
  workspaceId: string,
  secretId: string,
  value: string,
): Promise<{ secret: Secret }> {
  return api
    .put(`/api/workspaces/${workspaceId}/secrets/${secretId}/value`, { json: { value } })
    .json<{ secret: Secret }>();
}

export async function updateSecretDescription(
  workspaceId: string,
  secretId: string,
  description: string,
): Promise<{ secret: Secret }> {
  return api
    .patch(`/api/workspaces/${workspaceId}/secrets/${secretId}`, { json: { description } })
    .json<{ secret: Secret }>();
}

export async function deleteSecret(workspaceId: string, secretId: string): Promise<void> {
  await api.delete(`/api/workspaces/${workspaceId}/secrets/${secretId}`);
}
