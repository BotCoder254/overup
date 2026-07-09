import type { WorkspaceSummary } from './workspace';

/** Non-sensitive profile returned by `GET /api/me`. */
export interface Me {
  id: string;
  username: string;
  displayName: string | null;
  email: string | null;
  avatarUrl: string | null;
  onboarded: boolean;
  workspace: WorkspaceSummary | null;
}
