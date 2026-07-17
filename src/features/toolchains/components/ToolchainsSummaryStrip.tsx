import { AlertTriangle } from 'lucide-react';
import type { ToolchainsResponse } from '../../../types/toolchain';

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

interface ToolchainsSummaryStripProps {
  data: ToolchainsResponse | undefined;
  loading: boolean;
  error: boolean;
}

/**
 * Operational summary strip — the KpiStrip pattern: an even grid of cells
 * separated by a 1px background gap.
 */
export function ToolchainsSummaryStrip({ data, loading, error }: ToolchainsSummaryStripProps) {
  if (error) {
    return (
      <div className="mb-6 flex items-center gap-2 rounded border border-steel/20 bg-canvas p-4 text-sm text-steel">
        <AlertTriangle size={16} className="shrink-0 text-danger" aria-hidden="true" />
        Couldn&apos;t load the toolchain catalog.
      </div>
    );
  }

  const installed = data?.toolchains.filter((t) => t.installStatus === 'installed').length ?? 0;
  const cells: CellProps[] =
    loading || !data
      ? [
          { label: 'Toolchains', value: '—' },
          { label: 'Installed', value: '—' },
          { label: 'Default image', value: '—' },
          { label: 'Allow-list', value: '—' },
        ]
      : [
          {
            label: 'Toolchains',
            value: String(data.toolchains.length),
            hint: 'Pre-built language images',
          },
          {
            label: 'Installed',
            value: String(installed),
            hint: data.installSupported ? 'Pulled + warmed on runners' : 'Hosted runners required',
          },
          {
            label: 'Default image',
            value: data.defaultImage.split(':').pop() ?? data.defaultImage,
            hint: data.defaultImage,
          },
          {
            label: 'Allow-list',
            value: data.allowlistEnabled ? 'On' : 'Off',
            hint: data.allowlistEnabled ? 'Images are restricted' : 'Any valid image allowed',
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
