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

export interface WorkspaceAvailability {
  name: string;
  /** The clean slug that would be derived from `name`. */
  slug: string;
  /** `true` when `slug` is free and unreserved. */
  available: boolean;
  /** When taken/reserved, the slug that would actually be assigned. */
  adjustedSlug: string | null;
}

/**
 * Advisory pre-flight for the create-workspace form: given a name, the
 * backend derives the slug and reports whether it's free. Never blocks
 * submission — the server still resolves collisions authoritatively.
 */
export async function checkWorkspaceAvailability(
  name: string,
): Promise<WorkspaceAvailability> {
  return api
    .get('/api/workspaces/availability', { searchParams: { name } })
    .json<WorkspaceAvailability>();
}
