import { AlertTriangle } from 'lucide-react';
import type { ActivitySummary } from '../../../types/activity';

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

interface ActivitySummaryStripProps {
  summary: ActivitySummary | undefined;
  loading: boolean;
  error: boolean;
}

/**
 * Operational summary strip — the KpiStrip pattern: an even grid of cells
 * separated by a 1px background gap. Deliberately just the four numbers
 * that matter operationally; the category distribution lives in the rail.
 */
export function ActivitySummaryStrip({ summary, loading, error }: ActivitySummaryStripProps) {
  if (error) {
    return (
      <div className="mb-6 flex items-center gap-2 rounded border border-steel/20 bg-canvas p-4 text-sm text-steel">
        <AlertTriangle size={16} className="shrink-0 text-danger" aria-hidden="true" />
        Couldn&apos;t load the activity summary.
      </div>
    );
  }

  const windowDays = summary?.windowDays ?? 30;
  const cells: CellProps[] =
    loading || !summary
      ? [
          { label: 'Total events', value: '—' },
          { label: 'Last 24 h', value: '—' },
          { label: `Security (${windowDays}d)`, value: '—' },
          { label: `Failures (${windowDays}d)`, value: '—' },
        ]
      : [
          {
            label: 'Total events',
            value: String(summary.total),
            hint: 'Immutable audit ledger',
          },
          { label: 'Last 24 h', value: String(summary.last24h) },
          {
            label: `Security (${windowDays}d)`,
            value: String(summary.security30d),
            hint: 'Secrets, tokens & installs',
          },
          {
            label: `Failures (${windowDays}d)`,
            value: String(summary.failures30d),
            hint: summary.failures30d > 0 ? 'Failed runs & provisioning' : undefined,
          },
        ];

  return (
    <div className="mb-6 grid grid-cols-2 gap-px overflow-hidden rounded border border-steel/20 bg-steel/10 sm:grid-cols-4">
      {cells.map((cell) => (
        <Cell key={cell.label} {...cell} />
      ))}
    </div>
  );
}
