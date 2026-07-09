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
