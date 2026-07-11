import { api } from '../../../lib/api';
import type { Runner } from '../../../types/runner';

export interface RunnersListResponse {
  runners: Runner[];
  /** Whether this deployment can provision hosted runners itself. */
  hostedAvailable: boolean;
}

export async function getRunnersList(workspaceId: string): Promise<RunnersListResponse> {
  return api.get(`/api/workspaces/${workspaceId}/runners`).json<RunnersListResponse>();
}

export async function getRunners(workspaceId: string): Promise<Runner[]> {
  return (await getRunnersList(workspaceId)).runners;
}

export async function getRunnerDetail(workspaceId: string, runnerId: string): Promise<Runner> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/runners/${runnerId}`)
    .json<{ runner: Runner }>();
  return body.runner;
}

export interface BootstrapResult {
  runner: Runner;
  /** One-time bootstrap credential — the runner exchanges it for a permanent token on first connect. */
  token: string;
  expiresAt: string;
}

export async function bootstrapRunner(
  workspaceId: string,
  input: { name: string; labels: string[] },
): Promise<BootstrapResult> {
  return api
    .post(`/api/workspaces/${workspaceId}/runners/bootstrap`, { json: input })
    .json<BootstrapResult>();
}

/**
 * "Create and wait": the server provisions a runner container itself. No
 * token ever reaches the browser — it is injected into the container.
 */
export async function createHostedRunner(
  workspaceId: string,
  input: { name: string; labels: string[] },
): Promise<Runner> {
  const body = await api
    .post(`/api/workspaces/${workspaceId}/runners/hosted`, { json: input })
    .json<{ runner: Runner }>();
  return body.runner;
}

export async function updateRunner(
  workspaceId: string,
  runnerId: string,
  input: { name?: string; labels?: string[] },
): Promise<Runner> {
  const body = await api
    .patch(`/api/workspaces/${workspaceId}/runners/${runnerId}`, { json: input })
    .json<{ runner: Runner }>();
  return body.runner;
}

export async function regenerateRunnerToken(
  workspaceId: string,
  runnerId: string,
): Promise<string> {
  const body = await api
    .post(`/api/workspaces/${workspaceId}/runners/${runnerId}/regenerate-token`)
    .json<{ token: string }>();
  return body.token;
}

export async function revokeRunner(workspaceId: string, runnerId: string): Promise<void> {
  await api.delete(`/api/workspaces/${workspaceId}/runners/${runnerId}`);
}

export async function drainRunner(workspaceId: string, runnerId: string): Promise<void> {
  await api.post(`/api/workspaces/${workspaceId}/runners/${runnerId}/drain`);
}

export async function disableRunner(workspaceId: string, runnerId: string): Promise<void> {
  await api.post(`/api/workspaces/${workspaceId}/runners/${runnerId}/disable`);
}

export async function resumeRunner(workspaceId: string, runnerId: string): Promise<void> {
  await api.post(`/api/workspaces/${workspaceId}/runners/${runnerId}/resume`);
}
