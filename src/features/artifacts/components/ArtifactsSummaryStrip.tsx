import { AlertTriangle } from 'lucide-react';
import type { ArtifactsSummary } from '../../../types/artifact';
import { formatBytes } from '../../pipelines/lib/format';

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

interface ArtifactsSummaryStripProps {
  summary: ArtifactsSummary | undefined;
  loading: boolean;
  error: boolean;
}

/**
 * Storage summary strip — the KpiStrip pattern: an even grid of cells
 * separated by a 1px background gap (not `divide-x`, which strays once the
 * strip wraps on narrow viewports).
 */
export function ArtifactsSummaryStrip({ summary, loading, error }: ArtifactsSummaryStripProps) {
  if (error) {
    return (
      <div className="mb-6 flex items-center gap-2 rounded border border-steel/20 bg-canvas p-4 text-sm text-steel">
        <AlertTriangle size={16} className="shrink-0 text-danger" aria-hidden="true" />
        Couldn&apos;t load the storage summary.
      </div>
    );
  }

  // Deliberately just the four actionable numbers — inventory, healthy count,
  // what expires soon, and the storage footprint. Upload-state and largest-
  // artifact detail live in the table and its filters instead.
  const cells: CellProps[] = loading || !summary
    ? [
        { label: 'Artifacts', value: '—' },
        { label: 'Available', value: '—' },
        { label: 'Expiring in 7 days', value: '—' },
        { label: 'Total size', value: '—' },
      ]
    : [
        { label: 'Artifacts', value: String(summary.total) },
        { label: 'Available', value: String(summary.uploaded) },
        { label: 'Expiring in 7 days', value: String(summary.expiringSoon) },
        {
          label: 'Total size',
          value: formatBytes(summary.totalBytes),
          hint: summary.byKind
            .slice(0, 2)
            .map((usage) => `${usage.kind} ${formatBytes(usage.bytes)}`)
            .join(' · '),
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
