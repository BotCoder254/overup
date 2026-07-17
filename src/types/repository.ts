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
  ownerAvatarUrl: string | null;
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
  ownerAvatarUrl: string | null;
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

/** What processing one repository event caused (static categories). */
export type RepositoryEventOutcome =
  | 'pipelines_created'
  | 'sync_scheduled'
  | 'pipelines_and_sync'
  | 'ignored'
  | 'failed';

/** One entry of the chronological repository event timeline. */
export interface RepositoryEvent {
  id: string;
  event: string;
  action: string | null;
  gitRef: string | null;
  headSha: string | null;
  actorLogin: string | null;
  actorAvatarUrl: string | null;
  outcome: RepositoryEventOutcome;
  /** Static category (e.g. filters_not_matched) when outcome = ignored. */
  ignoredReason: string | null;
  pipelineIds: string[];
  syncRunId: string | null;
  summary: {
    skipped?: { path: string; reason: string }[];
    prNumber?: number;
    merged?: boolean;
    syncCollapsed?: boolean;
    branchDeleted?: boolean;
    tagDeleted?: boolean;
  } & Record<string, unknown>;
  receivedAt: string;
  processedAt: string;
}

export interface RepositoryEventsPage {
  events: RepositoryEvent[];
  nextCursor: string | null;
}

/** Static webhook signature-rejection categories (server vocabulary). */
export type WebhookRejectionCause =
  | 'missing_header'
  | 'malformed_header'
  | 'bad_prefix'
  | 'invalid_hex'
  | 'mismatch';

/**
 * Deployment-global webhook signature-rejection gauge. A wrong
 * GITHUB_WEBHOOK_SECRET rejects every delivery before persistence, so this
 * is not scoped to one repository; it resets when the backend restarts.
 */
export interface WebhookAuthHealth {
  rejections24h: number;
  lastRejectedAt: string | null;
  lastCause: WebhookRejectionCause | null;
}

/** Webhook/sync health figures for the repository sync status panel. */
export interface RepositoryHealth {
  lastEventAt: string | null;
  lastEventOutcome: RepositoryEventOutcome | null;
  failedEvents24h: number;
  pendingDeliveries: number;
  checksEnabled: boolean;
  webhookAuth: WebhookAuthHealth;
}
