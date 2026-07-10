import { AlertTriangle } from 'lucide-react';
import type { QueueSummary } from '../../../types/job';

function formatWaitSecs(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return '—';
  const total = Math.max(Math.round(seconds), 0);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}

interface CellProps {
  label: string;
  value: string;
  hint?: string;
}

function Cell({ label, value, hint }: CellProps) {
  return (
    <div className="min-w-0 bg-canvas p-4">
      <div className="text-xs font-medium uppercase tracking-wider text-steel">{label}</div>
      <div className="mt-1.5 text-2xl font-semibold tracking-tight text-charcoal">{value}</div>
      {hint && <div className="mt-0.5 truncate text-xs text-steel">{hint}</div>}
    </div>
  );
}

interface QueueSummaryStripProps {
  summary: QueueSummary | undefined;
  loading: boolean;
  error: boolean;
}

/** Scheduler summary strip, in the Dashboard KPI strip's grid treatment
 * (1px background gaps — no divide-x, which breaks when the grid wraps). */
export function QueueSummaryStrip({ summary, loading, error }: QueueSummaryStripProps) {
  if (error) {
    return (
      <div className="mb-4 flex items-center gap-2 rounded border border-steel/20 bg-canvas p-4 text-sm text-steel">
        <AlertTriangle size={16} className="shrink-0 text-danger" aria-hidden="true" />
        Couldn&apos;t load queue metrics.
      </div>
    );
  }

  const cells: CellProps[] =
    loading || !summary
      ? [
          { label: 'Queued', value: '—' },
          { label: 'In progress', value: '—' },
          { label: 'Avg wait', value: '—' },
          { label: 'Max wait', value: '—' },
          { label: 'Runners', value: '—' },
        ]
      : [
          {
            label: 'Queued',
            value: String(summary.queuedTotal),
            hint: `${summary.queuedWaitingRunner} ready · ${summary.queuedBlocked} blocked`,
          },
          { label: 'In progress', value: String(summary.inProgress) },
          { label: 'Avg wait', value: formatWaitSecs(summary.avgQueueWaitSecs) },
          { label: 'Max wait', value: formatWaitSecs(summary.maxQueueWaitSecs) },
          {
            label: 'Runners',
            value: `${summary.runnersIdle} idle`,
            hint: `${summary.runnersBusy} busy · ${summary.runnersOffline} offline${
              summary.runnersDisabled > 0 ? ` · ${summary.runnersDisabled} disabled` : ''
            }`,
          },
        ];

  return (
    <div className="mb-4 grid grid-cols-2 gap-px overflow-hidden rounded border border-steel/20 bg-steel/10 sm:grid-cols-3 lg:grid-cols-5">
      {cells.map((cell) => (
        <Cell key={cell.label} {...cell} />
      ))}
    </div>
  );
}
