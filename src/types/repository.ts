/** Shapes returned by the repository management API. */

export type SyncStatus = 'pending' | 'syncing' | 'synced' | 'failed';

export interface Installation {
  id: string;
  accountLogin: string;
  accountType: 'User' | 'Organization';
  accountAvatarUrl: string | null;
  suspended: boolean;
  createdAt: string;
}

export interface InstallationsResponse {
  installations: Installation[];
  installUrl: string;
}

export interface Repository {
  id: string;
  owner: string;
  name: string;
  fullName: string;
  private: boolean;
  defaultBranch: string;
  language: string | null;
  description: string | null;
  syncStatus: SyncStatus;
  syncError: string | null;
  lastSyncedAt: string | null;
  workflowCount: number;
  createdAt: string;
}

export interface AvailableRepo {
  githubRepoId: number;
  installationId: string;
  owner: string;
  name: string;
  fullName: string;
  private: boolean;
  defaultBranch: string | null;
  language: string | null;
  description: string | null;
  connected: boolean;
}

export interface Branch {
  name: string;
  commitSha: string;
  isDefault: boolean;
  updatedAt: string;
}

export interface SyncRun {
  id: string;
  trigger: 'import' | 'manual' | 'webhook';
  status: 'running' | 'success' | 'failed';
  error: string | null;
  stats: Record<string, number>;
  startedAt: string;
  finishedAt: string | null;
}
