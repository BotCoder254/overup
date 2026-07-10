import { formatBytes } from '../../pipelines/lib/format';
import type { RunnerHealth } from '../../../types/runner';

interface MeterProps {
  label: string;
  used: number | undefined;
  total: number | undefined;
}

function Meter({ label, used, total }: MeterProps) {
  const pct = used !== undefined && total ? Math.min(100, Math.round((used / total) * 100)) : null;
  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline justify-between text-xs">
        <span className="font-medium text-charcoal">{label}</span>
        <span className="text-steel">
          {pct === null
            ? '—'
            : `${formatBytes(used ?? null)} / ${formatBytes(total ?? null)} (${pct}%)`}
        </span>
      </div>
      <div className="h-2 overflow-hidden rounded bg-surface">
        {pct !== null && <div className="h-full bg-primary" style={{ width: `${pct}%` }} />}
      </div>
    </div>
  );
}

function CpuMeter({ permille }: { permille: number | undefined }) {
  const pct = permille !== undefined ? Math.min(100, Math.round(permille / 10)) : null;
  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline justify-between text-xs">
        <span className="font-medium text-charcoal">CPU</span>
        <span className="text-steel">{pct === null ? '—' : `${pct}%`}</span>
      </div>
      <div className="h-2 overflow-hidden rounded bg-surface">
        {pct !== null && <div className="h-full bg-primary" style={{ width: `${pct}%` }} />}
      </div>
    </div>
  );
}

interface HealthPanelProps {
  health: RunnerHealth | null;
}

/** Solid-fill CPU/memory/disk meters sampled on the runner's heartbeat. No gradients. */
export function HealthPanel({ health }: HealthPanelProps) {
  if (!health) {
    return <p className="text-sm text-steel">No health data reported yet.</p>;
  }

  return (
    <div className="space-y-4">
      <CpuMeter permille={health.cpuPermille} />
      <Meter label="Memory" used={health.memUsedBytes} total={health.memTotalBytes} />
      <Meter label="Disk" used={health.diskUsedBytes} total={health.diskTotalBytes} />
    </div>
  );
}
