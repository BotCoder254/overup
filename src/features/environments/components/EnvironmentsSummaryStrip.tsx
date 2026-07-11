import { AlertTriangle } from 'lucide-react';
import type { EnvironmentsSummary } from '../../../types/environment';

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

interface EnvironmentsSummaryStripProps {
  summary: EnvironmentsSummary | undefined;
  loading: boolean;
  error: boolean;
}

/**
 * Operational summary strip — the KpiStrip pattern: an even grid of cells
 * separated by a 1px background gap.
 */
export function EnvironmentsSummaryStrip({
  summary,
  loading,
  error,
}: EnvironmentsSummaryStripProps) {
  if (error) {
    return (
      <div className="mb-6 flex items-center gap-2 rounded border border-steel/20 bg-canvas p-4 text-sm text-steel">
        <AlertTriangle size={16} className="shrink-0 text-danger" aria-hidden="true" />
        Couldn&apos;t load the environments summary.
      </div>
    );
  }

  const cells: CellProps[] = loading || !summary
    ? [
        { label: 'Environments', value: '—' },
        { label: 'With secrets', value: '—' },
        { label: 'Created (30d)', value: '—' },
        { label: 'Scoped secrets', value: '—' },
      ]
    : [
        {
          label: 'Environments',
          value: String(summary.total),
          hint: 'Referenced from workflow YAML by name',
        },
        { label: 'With secrets', value: String(summary.withSecrets) },
        { label: 'Created (30d)', value: String(summary.createdLast30d) },
        {
          label: 'Scoped secrets',
          value: String(summary.scopedSecrets),
          hint: 'Highest injection precedence',
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
