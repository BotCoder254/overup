/** Pipeline execution API types — mirror the backend camelCase DTOs. */

export type PipelineStatus = 'queued' | 'in_progress' | 'completed';
export type PipelineConclusion = 'success' | 'failure' | 'cancelled' | 'timed_out' | 'partial';
export type JobConclusion = 'success' | 'failure' | 'cancelled' | 'timed_out' | 'skipped';
export type PipelineTrigger = 'push' | 'manual';
export type LogStreamName = 'stdout' | 'stderr' | 'system';
/** Execution phase a log chunk is attributed to (protocol::LOG_PHASES). */
export type LogPhase =
  | 'checkout'
  | 'image_pull'
  | 'container'
  | 'steps'
  | 'artifacts'
  | 'cleanup';

export interface Pipeline {
  id: string;
  repositoryId: string;
  repoFullName: string;
  workflowId: string | null;
  workflowName: string;
  workflowPath: string;
  number: number;
  trigger: PipelineTrigger;
  commitSha: string;
  commitMessage: string | null;
  commitAuthor: string | null;
  /** Actor snapshot: webhook sender (push) or the dispatching/rerunning user. */
  actorLogin: string | null;
  actorAvatarUrl: string | null;
  gitRef: string;
  /** Manual dispatch inputs (workflow_dispatch-style); absent otherwise. */
  triggerInputs?: Record<string, string | number | boolean> | null;
  status: PipelineStatus;
  conclusion: PipelineConclusion | null;
  createdAt: string;
  startedAt: string | null;
  finishedAt: string | null;
}

export interface PipelineJobPlanStep {
  name: string;
  run: string;
  shell: string;
}

/** Executable snapshot persisted at pipeline creation (env pre-masked). */
export interface PipelineJobPlan {
  image: string;
  env: Record<string, string>;
  steps: PipelineJobPlanStep[];
  /** YAML `environment:` binding — name only, resolved live at dispatch. */
  environment?: string | null;
  notices: string[];
}

/** Runner-reported resource telemetry, validated server-side. */
export interface PipelineJobMetrics {
  imagePullMs?: number;
  execMs?: number;
  /** Peak CPU in permille of one core (2500 = 2.5 cores). */
  cpuPeakPermille?: number;
  cpuAvgPermille?: number;
  memPeakBytes?: number;
  netRxBytes?: number;
  netTxBytes?: number;
  blkioReadBytes?: number;
  blkioWriteBytes?: number;
  sampleCount?: number;
}

export interface PipelineJob {
  id: string;
  key: string;
  name: string | null;
  runsOn: string[];
  needs: string[];
  plan: PipelineJobPlan;
  status: PipelineStatus;
  conclusion: JobConclusion | null;
  stage: string;
  runnerId: string | null;
  attempt: number;
  exitCode: number | null;
  errorCategory: string | null;
  logBytes: number;
  position: number;
  metrics: PipelineJobMetrics | null;
  queuedAt: string;
  assignedAt: string | null;
  startedAt: string | null;
  finishedAt: string | null;
}

export interface PipelineEvent {
  id: number;
  jobId: string | null;
  eventType: string;
  fromState: string | null;
  toState: string | null;
  runnerId: string | null;
  actorUserId: string | null;
  payload: Record<string, unknown>;
  createdAt: string;
}

export interface LogChunk {
  seq: number;
  stream: LogStreamName;
  content: string;
  /** Server receive time; present on REST and stream paths alike. */
  createdAt?: string;
  /** 0-based plan step this chunk belongs to; null/absent = unsectioned. */
  stepIndex?: number | null;
  /** Execution phase section; null/absent = unsectioned (old runners). */
  phase?: LogPhase | null;
}

export type ArtifactKind =
  | 'package'
  | 'report'
  | 'docs'
  | 'archive'
  | 'binary'
  | 'image'
  | 'log'
  | 'other';

export interface Artifact {
  id: string;
  pipelineId: string;
  jobId: string;
  name: string;
  sizeBytes: number | null;
  contentType: string | null;
  checksumSha256: string | null;
  status: 'pending' | 'uploaded' | 'failed' | 'expired';
  kind: ArtifactKind;
  uncompressedBytes: number | null;
  fileCount: number | null;
  createdAt: string;
  expiresAt: string | null;
}

export interface PipelineListResponse {
  pipelines: Pipeline[];
  nextCursor: string | null;
}

export interface PipelineDetail {
  pipeline: Pipeline;
  jobs: PipelineJob[];
  events: PipelineEvent[];
}

/** Frames pushed over the live pipeline WebSocket. */
export type PipelineStreamEvent =
  | { type: 'snapshot'; pipeline: Pipeline; jobs: PipelineJob[] }
  | {
      type: 'pipeline_update';
      id: string;
      status: PipelineStatus;
      conclusion: PipelineConclusion | null;
      startedAt: string | null;
      finishedAt: string | null;
    }
  | { type: 'job_update'; job: PipelineJob }
  | { type: 'event'; event: PipelineEvent }
  | {
      type: 'log';
      jobId: string;
      seq: number;
      stream: LogStreamName;
      text: string;
      createdAt?: string;
      stepIndex?: number | null;
      phase?: LogPhase | null;
    }
  | { type: 'log_gap'; jobId: string | null }
  | { type: 'artifact'; artifact: Artifact }
  | { type: 'pong' };
