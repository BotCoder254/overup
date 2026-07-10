import { api } from '../../../lib/api';
import type {
  AvailableRepo,
  Branch,
  InstallationsResponse,
  Repository,
  SyncRun,
} from '../../../types/repository';
import type { WorkflowSummary } from '../../../types/workflow';

export interface RepositoryDetail {
  repository: Repository;
  branches: Branch[];
  workflows: WorkflowSummary[];
  syncRuns: SyncRun[];
}

export async function getInstallations(workspaceId: string): Promise<InstallationsResponse> {
  return api.get(`/api/workspaces/${workspaceId}/installations`).json<InstallationsResponse>();
}

export async function unlinkInstallation(workspaceId: string, installationId: string): Promise<void> {
  await api.delete(`/api/workspaces/${workspaceId}/installations/${installationId}`);
}

export async function getRepositories(workspaceId: string): Promise<Repository[]> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/repositories`)
    .json<{ repositories: Repository[] }>();
  return body.repositories;
}

export async function getAvailableRepositories(workspaceId: string): Promise<AvailableRepo[]> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/repositories/available`)
    .json<{ repositories: AvailableRepo[] }>();
  return body.repositories;
}

export async function importRepository(
  workspaceId: string,
  input: { installationId: string; githubRepoId: number },
): Promise<Repository> {
  return api
    .post(`/api/workspaces/${workspaceId}/repositories`, { json: input })
    .json<Repository>();
}

export async function getRepositoryDetail(
  workspaceId: string,
  repositoryId: string,
): Promise<RepositoryDetail> {
  return api
    .get(`/api/workspaces/${workspaceId}/repositories/${repositoryId}`)
    .json<RepositoryDetail>();
}

export async function syncRepository(workspaceId: string, repositoryId: string): Promise<void> {
  await api.post(`/api/workspaces/${workspaceId}/repositories/${repositoryId}/sync`);
}

export async function removeRepository(workspaceId: string, repositoryId: string): Promise<void> {
  await api.delete(`/api/workspaces/${workspaceId}/repositories/${repositoryId}`);
}
