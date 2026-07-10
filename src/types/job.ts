/** Job queue + job execution API types — mirror the backend camelCase DTOs. */

import type {
  Artifact,
  JobConclusion,
  Pipeline,
  PipelineEvent,
  PipelineJob,
  PipelineStatus,
  PipelineTrigger,
} from './pipeline';
import type { Runner } from './runner';

/**
 * Static, server-computed wait/progress category for a queue row. Advisory
 * only — a diagnosis of why a job has not started, never control flow.
 */
export type QueueReason =
  | 'waiting_dependencies'
  | 'no_runner_online'
  | 'no_matching_runner'
  | 'runners_busy'
  | 'waiting_scheduler'
  | 'dispatching'
  | 'starting'
  | 'running';

/** One active (queued or running) job in the workspace queue. Deliberately
 * slim: no plan/env — identity, scheduling state, and its explanation. */
export interface QueueJob {
  id: string;
  pipelineId: string;
  key: string;
  name: string | null;
  runsOn: string[];
  needs: string[];
  status: PipelineStatus;
  conclusion: JobConclusion | null;
  stage: string;
  runnerId: string | null;
  attempt: number;
  errorCategory: string | null;
  queuedAt: string;
  assignedAt: string | null;
  startedAt: string | null;
  pipelineNumber: number;
  repositoryId: string;
  repoFullName: string;
  workflowId: string | null;
  workflowName: string;
  gitRef: string;
  trigger: PipelineTrigger;
  queueReason: QueueReason;
}

export interface QueueListResponse {
  jobs: QueueJob[];
  nextCursor: string | null;
}

/** Aggregate scheduler metrics for the queue page's summary strip. */
export interface QueueSummary {
  queuedTotal: number;
  queuedBlocked: number;
  queuedWaitingRunner: number;
  inProgress: number;
  avgQueueWaitSecs: number | null;
  maxQueueWaitSecs: number | null;
  oldestQueuedAt: string | null;
  runnersIdle: number;
  runnersBusy: number;
  runnersOffline: number;
  runnersDisabled: number;
}

/** The Job Execution page's identity payload. */
export interface JobDetail {
  job: PipelineJob;
  pipeline: Pipeline;
  events: PipelineEvent[];
  artifacts: Artifact[];
  runner: Runner | null;
}
