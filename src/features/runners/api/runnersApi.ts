import { api } from '../../../lib/api';
import type { Runner } from '../../../types/runner';

export async function getRunners(workspaceId: string): Promise<Runner[]> {
  const body = await api.get(`/api/workspaces/${workspaceId}/runners`).json<{ runners: Runner[] }>();
  return body.runners;
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
