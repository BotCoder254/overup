/**
 * Secrets Management types — mirror the backend camelCase DTOs.
 *
 * Deliberately: NO type in this file (or anywhere) carries a secret value.
 * Values are write-only — they leave the browser once at creation or
 * replacement and can never be read back.
 */

export type SecretScope = 'workspace' | 'repository';

export interface Secret {
  id: string;
  name: string;
  scope: SecretScope;
  repositoryId?: string;
  repositoryName?: string;
  description?: string;
  creatorLogin?: string;
  updaterLogin?: string;
  createdAt: string;
  updatedAt: string;
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
  usedLast30d: number;
  neverUsed: number;
  createdLast30d: number;
  distinctRepositories: number;
  totalInjections: number;
  /** Whether SECRETS_MASTER_KEY is configured on the deployment. */
  encryptionConfigured: boolean;
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
