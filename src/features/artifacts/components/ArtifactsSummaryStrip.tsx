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

  const cells: CellProps[] = loading || !summary
    ? [
        { label: 'Artifacts', value: '—' },
        { label: 'Available', value: '—' },
        { label: 'Uploading / failed', value: '—' },
        { label: 'Uploaded (24h)', value: '—' },
        { label: 'Expiring in 7 days', value: '—' },
        { label: 'Expiring storage', value: '—' },
        { label: 'Largest', value: '—' },
        { label: 'Total size', value: '—' },
      ]
    : [
        { label: 'Artifacts', value: String(summary.total) },
        { label: 'Available', value: String(summary.uploaded) },
        {
          label: 'Uploading / failed',
          value: String(summary.pending + summary.failed),
          hint: `${summary.pending} uploading · ${summary.failed} failed`,
        },
        { label: 'Uploaded (24h)', value: String(summary.recent24h) },
        { label: 'Expiring in 7 days', value: String(summary.expiringSoon) },
        {
          label: 'Expiring storage',
          value: formatBytes(summary.expiringBytes7d),
          hint: `of ${formatBytes(summary.totalBytes)} stored`,
        },
        summary.largest.length > 0
          ? {
              label: 'Largest',
              value: formatBytes(summary.largest[0].sizeBytes),
              hint: summary.largest[0].name,
            }
          : { label: 'Largest', value: '—' },
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
    <div className="mb-6 grid grid-cols-2 gap-px overflow-hidden rounded border border-steel/20 bg-steel/10 sm:grid-cols-4 lg:grid-cols-8">
      {cells.map((cell) => (
        <Cell key={cell.label} {...cell} />
      ))}
    </div>
  );
}
