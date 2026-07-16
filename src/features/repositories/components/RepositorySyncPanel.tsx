import { formatDistanceToNow } from 'date-fns';
import type { ReactNode } from 'react';
import { cn } from '../../../lib/cn';
import type { Repository, RepositoryHealth } from '../../../types/repository';

const OUTCOME_HINTS: Record<string, string> = {
  pipelines_created: 'pipelines created',
  sync_scheduled: 'sync scheduled',
  pipelines_and_sync: 'pipelines + sync',
  ignored: 'no action needed',
  failed: 'processing failed',
};

const SYNC_LABELS: Record<Repository['syncStatus'], string> = {
  pending: 'Queued',
  syncing: 'Syncing…',
  synced: 'Synced',
  failed: 'Failed',
};

interface CellProps {
  label: string;
  value: ReactNode;
  hint?: string;
  valueClassName?: string;
}

function Cell({ label, value, hint, valueClassName }: CellProps) {
  return (
    <div className="min-w-0 bg-canvas p-4">
      <div className="text-xs font-medium uppercase tracking-wider text-steel">{label}</div>
      <div
        className={cn(
          'mt-1.5 text-2xl font-semibold tracking-tight text-charcoal',
          valueClassName,
        )}
      >
        {value}
      </div>
      {hint && <div className="mt-0.5 truncate text-xs text-steel">{hint}</div>}
    </div>
  );
}

interface RepositorySyncPanelProps {
  repository: Repository;
  health: RepositoryHealth;
}

/**
 * Sync status panel — the KpiStrip pattern (1px `gap-px` grid on a
 * `bg-steel/10` sheet). Synchronization is fully automatic once a repository
 * is connected; this strip shows the live health of that automation:
 * sync state, the latest processed webhook event, the delivery queue depth,
 * recent processing failures, and whether results report back to GitHub.
 */
export function RepositorySyncPanel({ repository, health }: RepositorySyncPanelProps) {
  const cells: CellProps[] = [
    {
      label: 'Auto-sync',
      value: SYNC_LABELS[repository.syncStatus],
      valueClassName: repository.syncStatus === 'failed' ? 'text-danger' : undefined,
      hint: repository.lastSyncedAt
        ? `synced ${formatDistanceToNow(new Date(repository.lastSyncedAt), { addSuffix: true })}`
        : 'first sync pending',
    },
    {
      label: 'Last event',
      value: health.lastEventAt
        ? formatDistanceToNow(new Date(health.lastEventAt), { addSuffix: true })
        : '—',
      valueClassName: 'text-lg leading-8',
      hint: health.lastEventOutcome
        ? OUTCOME_HINTS[health.lastEventOutcome] ?? health.lastEventOutcome
        : 'no webhook events yet',
    },
    {
      label: 'Pending events',
      value: String(health.pendingDeliveries),
      hint: 'awaiting processing',
    },
    {
      label: 'Failed (24 h)',
      value: String(health.failedEvents24h),
      valueClassName: health.failedEvents24h > 0 ? 'text-danger' : undefined,
      hint: 'event processing failures',
    },
    {
      label: 'GitHub checks',
      value: health.checksEnabled ? 'On' : 'Off',
      hint: health.checksEnabled
        ? 'results report to commits'
        : 'reporting disabled',
    },
  ];

  return (
    <div className="mb-6 grid grid-cols-2 gap-px overflow-hidden rounded border border-steel/20 bg-steel/10 sm:grid-cols-3 lg:grid-cols-5">
      {cells.map((cell) => (
        <Cell key={cell.label} {...cell} />
      ))}
    </div>
  );
}
