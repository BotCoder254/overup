/**
 * Secrets Management types — mirror the backend camelCase DTOs.
 *
 * Deliberately: NO type in this file (or anywhere) carries a secret value.
 * Values are write-only — they leave the browser once at creation or
 * replacement and can never be read back.
 */

import type { DetectedRequirement } from './requirements';

export type SecretScope = 'workspace' | 'repository' | 'environment';

export interface Secret {
  id: string;
  name: string;
  scope: SecretScope;
  repositoryId?: string;
  repositoryName?: string;
  environmentId?: string;
  environmentName?: string;
  description?: string;
  creatorLogin?: string;
  updaterLogin?: string;
  createdAt: string;
  updatedAt: string;
  /** When the value was last set (created or replaced) — the rotation clock. */
  valueSetAt: string;
  lastUsedAt?: string;
  usageCount: number;
}

export interface SecretsListResponse {
  secrets: Secret[];
  nextCursor: string | null;
}

export interface SecretsSummary {
  total: number;
  workspaceScoped: number;
  repositoryScoped: number;
  environmentScoped: number;
  usedLast30d: number;
  neverUsed: number;
  createdLast30d: number;
  distinctRepositories: number;
  totalInjections: number;
  /** Values not rotated within the stale window. */
  stale: number;
  /** The server's stale window in days (currently 90). */
  staleAfterDays: number;
  /** Whether SECRETS_MASTER_KEY is configured on the deployment. */
  encryptionConfigured: boolean;
}

export interface SecretsRequirements {
  /** Workflow-referenced secret names with no configured secret. */
  secrets: DetectedRequirement[];
  /** `${{ vars.NAME }}` references — informational, not managed by overup. */
  vars: DetectedRequirement[];
}

export interface SecretAuditEvent {
  action: string;
  actorLogin: string | null;
  subjectId: string | null;
  metadata: Record<string, unknown>;
  createdAt: string;
}

export interface SecretsAuditResponse {
  events: SecretAuditEvent[];
}

export interface SecretDetailResponse {
  secret: Secret;
  audit: SecretAuditEvent[];
}
