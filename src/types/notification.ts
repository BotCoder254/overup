// Notification Center types — mirror the backend camelCase DTOs
// (backend/src/models/notification.rs). Titles/bodies are rendered
// server-side from static templates; link targets are allow-listed kind
// objects, never URLs; secret values never appear anywhere by construction.

export type NotificationCategory =
  | 'pipeline'
  | 'runner'
  | 'repository'
  | 'workflow'
  | 'artifact'
  | 'security'
  | 'environment'
  | 'system';

export type NotificationSeverity = 'info' | 'success' | 'warning' | 'error' | 'critical';

/** Server-built navigation target; `kind` is a backend allow-list. */
export interface NotificationLink {
  kind?: string;
  pipelineId?: string;
  repositoryId?: string;
  workflowId?: string;
  artifactId?: string;
  secretId?: string;
  environmentId?: string;
}

export interface Notification {
  id: string;
  /** Source audit action (or janitor scan kind, e.g. `secret.stale`). */
  action: string;
  category: NotificationCategory;
  severity: NotificationSeverity;
  title: string;
  body: string;
  subjectType: string | null;
  subjectId: string | null;
  link: NotificationLink;
  /** Dedup merges bump this instead of creating twin unread rows. */
  occurrenceCount: number;
  readAt: string | null;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface NotificationListResponse {
  notifications: Notification[];
  nextCursor: string | null;
}

export interface NotificationPreferences {
  mutedUntil: string | null;
  disabledCategories: NotificationCategory[];
  minSeverity: NotificationSeverity;
  updatedAt: string | null;
}

/** Frames pushed over /ws/workspaces/{ws}/notifications. */
export type NotificationStreamEvent =
  | { type: 'snapshot'; unreadCount: number }
  | { type: 'notification'; notification: Notification; inserted: boolean }
  | { type: 'unread_count'; count: number }
  | { type: 'pong' };
