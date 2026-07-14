import { formatDistanceToNow } from 'date-fns';
import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Badge } from '../../../components/ui/Badge';
import type { Pipeline } from '../../../types/pipeline';
import type { WorkflowDetail } from '../../../types/workflow';
import { PipelineStatusBadge } from '../../pipelines/components/PipelineStatusBadge';
import { usePipelines } from '../../pipelines/hooks/usePipelines';
import { SyncStatusBadge } from '../../repositories/components/SyncStatusBadge';
import { useRepositoryDetail } from '../../repositories/hooks/useRepositories';

/** How many recent runs feed the average/frequency figures. */
const RECENT_RUNS = 25;
const FREQUENCY_WINDOW_DAYS = 7;

function Cell({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0 bg-canvas px-4 py-3">
      <div className="text-[11px] font-medium uppercase tracking-wider text-steel">{label}</div>
      <div className="mt-1 flex min-h-[1.5rem] flex-wrap items-center gap-1 text-sm text-charcoal">
        {children}
      </div>
    </div>
  );
}

function averageDuration(runs: Pipeline[]): string | null {
  const durations = runs
    .filter((run) => run.status === 'completed' && run.startedAt && run.finishedAt)
    .map((run) => new Date(run.finishedAt!).getTime() - new Date(run.startedAt!).getTime())
    .filter((ms) => ms >= 0);
  if (durations.length === 0) return null;
  const totalSeconds = Math.round(
    durations.reduce((sum, ms) => sum + ms, 0) / durations.length / 1000,
  );
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

/**
 * Operational meta strip for the workflow detail header: trigger
 * definitions, default branch, repo sync state, and execution telemetry
 * (last run / average duration / frequency) computed client-side from the
 * most recent pipelines — no dedicated backend endpoint. Placeholder dashes
 * while loading so the editor below never shifts.
 */
export function WorkflowMetaStrip({
  workflow,
  slug,
}: {
  workflow: WorkflowDetail;
  slug: string;
}) {
  const repository = useRepositoryDetail(workflow.repositoryId);
  const runs = usePipelines({ workflowId: workflow.id, limit: RECENT_RUNS });

  const recent: Pipeline[] = runs.data?.pages[0]?.pipelines ?? [];
  const lastRun = recent[0];
  const avg = averageDuration(recent);
  const windowStart = Date.now() - FREQUENCY_WINDOW_DAYS * 24 * 60 * 60 * 1000;
  const inWindow = recent.filter(
    (run) => new Date(run.createdAt).getTime() >= windowStart,
  ).length;
  // A full page means there may be more runs inside the window than fetched.
  const frequency =
    recent.length >= RECENT_RUNS && inWindow === recent.length ? `≥${inWindow}` : String(inWindow);

  const dash = <span className="text-steel">—</span>;

  return (
    <div className="mb-4 grid grid-cols-2 gap-px overflow-hidden rounded border border-steel/20 bg-steel/10 sm:grid-cols-3 lg:grid-cols-6">
      <Cell label="Triggers">
        {workflow.triggers.length > 0
          ? workflow.triggers.map((trigger) => (
              <Badge key={trigger} variant="info">
                {trigger}
              </Badge>
            ))
          : dash}
      </Cell>
      <Cell label="Default branch">
        <span className="truncate font-mono text-xs">{workflow.defaultBranch}</span>
      </Cell>
      <Cell label="Repo sync">
        {repository.data ? <SyncStatusBadge status={repository.data.repository.syncStatus} /> : dash}
      </Cell>
      <Cell label="Last run">
        {runs.isLoading ? (
          dash
        ) : lastRun ? (
          <Link
            to={workspacePath(slug, `pipelines/${lastRun.id}`)}
            className="flex min-w-0 items-center gap-1.5 hover:underline"
          >
            <PipelineStatusBadge status={lastRun.status} conclusion={lastRun.conclusion} />
            <span className="truncate text-xs text-steel">
              {formatDistanceToNow(new Date(lastRun.createdAt), { addSuffix: true })}
            </span>
          </Link>
        ) : (
          <span className="text-xs text-steel">Never run</span>
        )}
      </Cell>
      <Cell label="Avg duration">
        {runs.isLoading ? (
          dash
        ) : avg ? (
          <>
            <span className="font-medium">{avg}</span>
            <span className="text-xs text-steel">last {recent.length}</span>
          </>
        ) : (
          dash
        )}
      </Cell>
      <Cell label={`Runs (${FREQUENCY_WINDOW_DAYS}d)`}>
        {runs.isLoading ? dash : <span className="font-medium">{frequency}</span>}
      </Cell>
    </div>
  );
}
