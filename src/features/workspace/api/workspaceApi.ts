import { api } from '../../../lib/api';
import type { Workspace } from '../../../types/workspace';

/**
 * Only the name and optional description ever leave the browser. The slug,
 * ownership, roles, and all other privileged fields are derived server-side.
 */
export async function createWorkspace(input: {
  name: string;
  description?: string;
}): Promise<Workspace> {
  return api.post('/api/workspaces', { json: input }).json<Workspace>();
}
