/** Environments API types — mirror the backend camelCase DTOs. */

export interface Environment {
  id: string;
  name: string;
  description?: string;
  creatorLogin?: string;
  updaterLogin?: string;
  /** Secrets currently scoped to this environment. */
  secretCount: number;
  createdAt: string;
  updatedAt: string;
}

export interface EnvironmentsListResponse {
  environments: Environment[];
  nextCursor: string | null;
}

export interface EnvironmentsSummary {
  total: number;
  withSecrets: number;
  createdLast30d: number;
  /** Total secrets scoped to any environment in the workspace. */
  scopedSecrets: number;
}

export interface EnvironmentAuditEvent {
  action: string;
  actorLogin: string | null;
  subjectId: string | null;
  metadata: Record<string, unknown>;
  createdAt: string;
}

export interface EnvironmentDetailResponse {
  environment: Environment;
  audit: EnvironmentAuditEvent[];
}
