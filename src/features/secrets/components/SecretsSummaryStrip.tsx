import { AlertTriangle } from 'lucide-react';
import type { SecretsSummary } from '../../../types/secret';

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

interface SecretsSummaryStripProps {
  summary: SecretsSummary | undefined;
  loading: boolean;
  error: boolean;
}

/**
 * Operational summary strip — the KpiStrip pattern: an even grid of cells
 * separated by a 1px background gap.
 */
export function SecretsSummaryStrip({ summary, loading, error }: SecretsSummaryStripProps) {
  if (error) {
    return (
      <div className="mb-6 flex items-center gap-2 rounded border border-steel/20 bg-canvas p-4 text-sm text-steel">
        <AlertTriangle size={16} className="shrink-0 text-danger" aria-hidden="true" />
        Couldn&apos;t load the secrets summary.
      </div>
    );
  }

  const cells: CellProps[] = loading || !summary
    ? [
        { label: 'Secrets', value: '—' },
        { label: 'Workspace-wide', value: '—' },
        { label: 'Repository-scoped', value: '—' },
        { label: 'Environment-scoped', value: '—' },
        { label: 'Used (30d)', value: '—' },
        { label: 'Never used', value: '—' },
        { label: 'Stale', value: '—' },
        { label: 'Injections', value: '—' },
      ]
    : [
        {
          label: 'Secrets',
          value: String(summary.total),
          hint: summary.encryptionConfigured ? 'AES-256-GCM at rest' : 'Encryption key not set',
        },
        { label: 'Workspace-wide', value: String(summary.workspaceScoped) },
        { label: 'Repository-scoped', value: String(summary.repositoryScoped) },
        { label: 'Environment-scoped', value: String(summary.environmentScoped) },
        { label: 'Used (30d)', value: String(summary.usedLast30d) },
        {
          label: 'Never used',
          value: String(summary.neverUsed),
          hint: summary.neverUsed > 0 ? 'Candidates for cleanup' : undefined,
        },
        {
          label: `Stale (${summary.staleAfterDays}d)`,
          value: String(summary.stale),
          hint: summary.stale > 0 ? 'Rotate by replacing the value' : undefined,
        },
        {
          label: 'Injections',
          value: String(summary.totalInjections),
          hint: 'Total job dispatches',
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
