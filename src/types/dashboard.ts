/** Dashboard/workspace-wide live-feed API types — mirror the backend camelCase DTOs. */

import type { ArtifactKind, PipelineConclusion, PipelineStatus } from './pipeline';
import type { Runner, RunnerHealth } from './runner';

export interface DashboardSummary {
  pipelinesTotal: number;
  pipelinesSucceeded: number;
  pipelinesFailed: number;
  pipelinesCancelled: number;
  pipelinesInProgress: number;
  pipelinesQueued: number;
  successRate: number;
  avgDurationSecs: number | null;
  runnersTotal: number;
  runnersIdle: number;
  runnersBusy: number;
  runnersOffline: number;
  runnersDisabled: number;
}

export interface ActivityBucket {
  bucket: string;
  succeeded: number;
  failed: number;
  cancelled: number;
  queued: number;
}

export type DashboardRange = '24h' | '7d' | '30d';

/** Frames pushed over the workspace-wide dashboard WebSocket. */
export type WorkspaceStreamEvent =
  | { type: 'snapshot' }
  | {
      type: 'pipeline_update';
      id: string;
      status: PipelineStatus;
      conclusion: PipelineConclusion | null;
      startedAt: string | null;
      finishedAt: string | null;
    }
  | { type: 'runner_update'; runner: Runner }
  | {
      type: 'runner_health';
      runnerId: string;
      health: RunnerHealth;
      lastSeenAt: string;
    }
  | {
      type: 'artifact_update';
      id: string;
      pipelineId: string;
      name: string;
      status: string;
      kind: ArtifactKind;
    }
  | { type: 'pong' };
