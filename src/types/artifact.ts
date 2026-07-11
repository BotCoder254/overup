import type { Artifact, ArtifactKind } from './pipeline';

/**
 * Workspace catalog entry: the artifact plus the provenance of the
 * execution that produced it — mirrors the backend camelCase DTO.
 */
export interface ArtifactCatalogEntry extends Artifact {
  pipelineNumber: number;
  repositoryId: string;
  repositoryFullName: string;
  workflowId: string | null;
  workflowName: string;
  branch: string;
  commitSha: string;
  jobKey: string;
  jobName: string | null;
  runnerName: string | null;
  /** Docker image the producing job ran in (from the job plan snapshot). */
  jobImage: string | null;
}

export interface ArtifactCatalogResponse {
  artifacts: ArtifactCatalogEntry[];
  nextCursor: string | null;
}

/** One file inside an archive artifact (runner-computed manifest). */
export interface ArtifactEntryItem {
  path: string;
  sizeBytes: number;
}

/** Detail response: provenance entry + archive contents when present. */
export interface ArtifactDetail {
  artifact: ArtifactCatalogEntry;
  entries: ArtifactEntryItem[] | null;
}

export interface ArtifactKindUsage {
  kind: ArtifactKind;
  count: number;
  bytes: number;
}

export interface ArtifactsSummary {
  total: number;
  uploaded: number;
  pending: number;
  failed: number;
  expiringSoon: number;
  totalBytes: number;
  recent24h: number;
  expiringBytes7d: number;
  byKind: ArtifactKindUsage[];
  largest: { id: string; name: string; sizeBytes: number }[];
}

/** Per-kind retention policy row ('default' = workspace-wide default). */
export interface RetentionPolicy {
  kind: ArtifactKind | 'default';
  retentionDays: number;
}

export interface RetentionPoliciesResponse {
  policies: RetentionPolicy[];
  globalDefaultDays: number;
}
