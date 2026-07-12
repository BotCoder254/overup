// Activity Feed types — mirror the backend camelCase DTOs
// (backend/src/models/activity.rs). Severity/category/security are derived
// server-side from the action string; audit metadata is value-free by
// construction (secret values never appear anywhere in the ledger).

export type ActivityCategory =
  | 'workspace'
  | 'integration'
  | 'repository'
  | 'pipeline'
  | 'runner'
  | 'artifact'
  | 'secret'
  | 'environment';

export type ActivitySeverity = 'info' | 'success' | 'warning' | 'danger';

export interface ActivityEvent {
  id: string;
  action: string;
  category: ActivityCategory;
  severity: ActivitySeverity;
  /** Credential-adjacent surfaces: secrets, runner tokens, installations. */
  security: boolean;
  subjectType: string;
  subjectId: string | null;
  /** null actor = system action (webhook sync, scheduler, provisioner). */
  actorId: string | null;
  actorLogin: string | null;
  actorAvatarUrl: string | null;
  metadata: Record<string, unknown>;
  requestId: string | null;
  createdAt: string;
}

export interface ActivityListResponse {
  events: ActivityEvent[];
  nextCursor: string | null;
}

export interface ActivitySummary {
  /** All-time ledger size; everything below is windowed. */
  total: number;
  last24h: number;
  security30d: number;
  failures30d: number;
  byCategory: Record<ActivityCategory, number>;
  windowDays: number;
}
