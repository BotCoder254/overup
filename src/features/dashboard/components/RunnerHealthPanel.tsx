import { formatDistanceToNow } from 'date-fns';
import { Server } from 'lucide-react';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { EmptyState } from '../../../components/ui/EmptyState';
import { RunnerStatusBadge } from '../../runners/components/RunnerStatusBadge';
import type { Runner } from '../../../types/runner';

interface RunnerHealthPanelProps {
  slug: string;
  runners: Runner[];
  loading: boolean;
}

/** Compact runner roster for the Dashboard's side panel — status, CPU, last seen. */
export function RunnerHealthPanel({ slug, runners, loading }: RunnerHealthPanelProps) {
  if (loading) {
    return <div className="h-40 animate-pulse rounded border border-steel/20 bg-canvas" />;
  }
  if (runners.length === 0) {
    return (
      <EmptyState
        icon={Server}
        title="No runners yet"
        description="Register a self-hosted runner to see its health here."
        className="min-h-0 py-10"
      />
    );
  }

  return (
    <div className="divide-y divide-steel/10 rounded border border-steel/20 bg-canvas">
      {runners.map((runner) => {
        const cpuPermille = runner.lastHealth?.cpuPermille;
        const cpuPct = cpuPermille !== undefined ? Math.min(100, Math.round(cpuPermille / 10)) : null;
        return (
          <Link
            key={runner.id}
            to={workspacePath(slug, `runners/${runner.id}`)}
            className="flex items-center justify-between gap-3 px-4 py-3 transition-colors hover:bg-surface"
          >
            <div className="min-w-0">
              <div className="truncate font-medium text-charcoal">{runner.name}</div>
              <div className="text-xs text-steel">
                {runner.lastSeenAt
                  ? formatDistanceToNow(new Date(runner.lastSeenAt), { addSuffix: true })
                  : 'Never connected'}
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-3">
              {cpuPct !== null && (
                <span className="font-mono text-xs text-steel">{cpuPct}% cpu</span>
              )}
              <RunnerStatusBadge status={runner.status} draining={runner.draining} />
            </div>
          </Link>
        );
      })}
    </div>
  );
}
