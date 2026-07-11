import type { RunnerHealth, RunnerResourceProfile } from '../../../types/runner';

/** Mirrors the server-side presets in backend services/runner_profiles.rs. */
const PROFILE_LABELS: Record<RunnerResourceProfile, string> = {
  small: 'Small · 1 CPU / 1 GiB',
  standard: 'Standard · 2 CPU / 2 GiB',
  large: 'Large · 4 CPU / 4 GiB',
};

function formatUptime(seconds: number | undefined): string {
  if (seconds === undefined) return '—';
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

interface RunnerMetaCardProps {
  health: RunnerHealth | null;
  /** Hosted runners only; null for self-hosted rows. */
  resourceProfile?: RunnerResourceProfile | null;
}

export function RunnerMetaCard({ health, resourceProfile }: RunnerMetaCardProps) {
  return (
    <dl className="space-y-3 text-sm">
      {resourceProfile && (
        <div className="flex items-center justify-between gap-4">
          <dt className="text-steel">Size</dt>
          <dd className="text-charcoal">{PROFILE_LABELS[resourceProfile]}</dd>
        </div>
      )}
      <div className="flex items-center justify-between gap-4">
        <dt className="text-steel">Docker version</dt>
        <dd className="text-charcoal">{health?.dockerVersion ?? '—'}</dd>
      </div>
      <div className="flex items-center justify-between gap-4">
        <dt className="text-steel">Operating system</dt>
        <dd className="text-charcoal">{health?.os ?? '—'}</dd>
      </div>
      <div className="flex items-center justify-between gap-4">
        <dt className="text-steel">Host uptime</dt>
        <dd className="text-charcoal">{formatUptime(health?.uptimeSecs)}</dd>
      </div>
    </dl>
  );
}
