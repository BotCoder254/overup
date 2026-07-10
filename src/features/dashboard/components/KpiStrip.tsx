import { AlertTriangle } from 'lucide-react';
import type { DashboardSummary } from '../../../types/dashboard';

function formatDurationSecs(seconds: number | null): string {
  if (seconds === null) return '—';
  const total = Math.max(Math.round(seconds), 0);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}

interface KpiCellProps {
  label: string;
  value: string;
  hint?: string;
}

function KpiCell({ label, value, hint }: KpiCellProps) {
  return (
    <div className="min-w-0 bg-canvas p-4">
      <div className="text-xs font-medium uppercase tracking-wider text-steel">{label}</div>
      <div className="mt-1.5 text-2xl font-semibold tracking-tight text-charcoal">{value}</div>
      {hint && <div className="mt-0.5 truncate text-xs text-steel">{hint}</div>}
    </div>
  );
}

interface KpiStripProps {
  summary: DashboardSummary | undefined;
  loading: boolean;
  error: boolean;
}

/**
 * Unified operational summary strip: an even grid of cells separated by a
 * 1px background gap rather than `divide-x`/`divide-y` — those utilities
 * are DOM-order based and produce stray borders once the strip wraps to
 * multiple rows on narrow viewports.
 */
export function KpiStrip({ summary, loading, error }: KpiStripProps) {
  if (error) {
    return (
      <div className="mb-6 flex items-center gap-2 rounded border border-steel/20 bg-canvas p-4 text-sm text-steel">
        <AlertTriangle size={16} className="shrink-0 text-danger" aria-hidden="true" />
        Couldn&apos;t load workspace metrics.
      </div>
    );
  }

  const cells: KpiCellProps[] = loading || !summary
    ? [
        { label: 'Total runs', value: '—' },
        { label: 'Success rate', value: '—' },
        { label: 'Avg duration', value: '—' },
        { label: 'Active runners', value: '—' },
        { label: 'Queued jobs', value: '—' },
      ]
    : [
        { label: 'Total runs', value: String(summary.pipelinesTotal) },
        {
          label: 'Success rate',
          value: `${Math.round(summary.successRate * 100)}%`,
          hint: `${summary.pipelinesSucceeded} succeeded · ${summary.pipelinesFailed} failed`,
        },
        { label: 'Avg duration', value: formatDurationSecs(summary.avgDurationSecs) },
        {
          label: 'Active runners',
          value: String(summary.runnersIdle + summary.runnersBusy),
          hint: `${summary.runnersTotal} total`,
        },
        { label: 'Queued jobs', value: String(summary.pipelinesQueued) },
      ];

  return (
    <div className="mb-6 grid grid-cols-2 gap-px overflow-hidden rounded border border-steel/20 bg-steel/10 sm:grid-cols-3 lg:grid-cols-5">
      {cells.map((cell) => (
        <KpiCell key={cell.label} {...cell} />
      ))}
    </div>
  );
}
