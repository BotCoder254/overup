import type { Artifact } from './pipeline';

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
}

export interface ArtifactCatalogResponse {
  artifacts: ArtifactCatalogEntry[];
  nextCursor: string | null;
}

export interface ArtifactsSummary {
  total: number;
  uploaded: number;
  pending: number;
  failed: number;
  expiringSoon: number;
  totalBytes: number;
}
