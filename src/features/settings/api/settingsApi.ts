import { api } from '../../../lib/api';
import type { Me } from '../../../types/user';
import type { UserSession } from '../../../types/session';
import type { Workspace, WorkspaceMember } from '../../../types/workspace';

/** Explicit allow-list patch — mirrors the backend DTO exactly. */
export interface UpdateMeInput {
  displayName?: string;
  email?: string;
}

export async function updateMe(patch: UpdateMeInput): Promise<Me> {
  return api.patch('/api/me', { json: patch }).json<Me>();
}

export async function deleteAccount(confirmUsername: string): Promise<void> {
  await api.delete('/api/me', { json: { confirmUsername } });
}

export async function listSessions(): Promise<UserSession[]> {
  const body = await api.get('/api/me/sessions').json<{ sessions: UserSession[] }>();
  return body.sessions;
}

export async function revokeSession(sessionId: string): Promise<void> {
  await api.delete(`/api/me/sessions/${sessionId}`);
}

export async function revokeAllSessions(): Promise<number> {
  const body = await api
    .post('/api/me/sessions/revoke-all')
    .json<{ revoked: number }>();
  return body.revoked;
}

export async function updateWorkspace(
  workspaceId: string,
  patch: { name: string },
): Promise<Workspace> {
  return api.patch(`/api/workspaces/${workspaceId}`, { json: patch }).json<Workspace>();
}

export async function listMembers(workspaceId: string): Promise<WorkspaceMember[]> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/members`)
    .json<{ members: WorkspaceMember[] }>();
  return body.members;
}

export async function getWorkspaceLogoUrl(workspaceId: string): Promise<string | null> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/logo-url`)
    .json<{ url: string | null }>();
  return body.url;
}

/**
 * Raw image bytes to the backend, which verifies the magic bytes and stores
 * the object in R2 under a server-generated key. The browser's declared
 * content type is irrelevant server-side.
 */
export async function uploadWorkspaceLogo(
  workspaceId: string,
  file: File,
): Promise<string> {
  const body = await api
    .put(`/api/workspaces/${workspaceId}/logo`, {
      body: file,
      headers: { 'content-type': 'application/octet-stream' },
    })
    .json<{ logoUrl: string }>();
  return body.logoUrl;
}

export async function removeWorkspaceLogo(workspaceId: string): Promise<void> {
  await api.delete(`/api/workspaces/${workspaceId}/logo`);
}
