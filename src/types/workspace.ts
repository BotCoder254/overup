/** Compact workspace shape embedded in `GET /api/me` for routing. */
export interface WorkspaceSummary {
  id: string;
  name: string;
  slug: string;
}

/** Full payload returned by `POST /api/workspaces`. */
export interface Workspace extends WorkspaceSummary {
  description: string | null;
}

/** One row of the read-only members table in Settings → Workspace. */
export interface WorkspaceMember {
  userId: string;
  username: string;
  displayName: string | null;
  email: string | null;
  avatarUrl: string | null;
  roleKey: string;
  roleName: string;
  joinedAt: string;
}
