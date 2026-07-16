import { formatDistanceToNow } from 'date-fns';
import { Server } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { EmptyState } from '../../../components/ui/EmptyState';
import { RunnerStatusBadge } from '../../runners/components/RunnerStatusBadge';
import type { Runner } from '../../../types/runner';
import { PagerControls } from './PagerControls';

const PAGE_SIZE = 5;

interface RunnerHealthPanelProps {
  slug: string;
  runners: Runner[];
  loading: boolean;
}

/**
 * Compact runner roster for the Dashboard's side panel — status, CPU, last
 * seen; paged five at a time so a large fleet doesn't stretch the panel.
 */
export function RunnerHealthPanel({ slug, runners, loading }: RunnerHealthPanelProps) {
  const [page, setPage] = useState(0);
  const pageCount = Math.max(1, Math.ceil(runners.length / PAGE_SIZE));

  // Clamp when the roster shrinks (revocation, live updates).
  useEffect(() => {
    if (page > pageCount - 1) setPage(pageCount - 1);
  }, [page, pageCount]);

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

  const start = page * PAGE_SIZE;
  const visible = runners.slice(start, start + PAGE_SIZE);

  return (
    <>
    <div className="divide-y divide-steel/10 rounded border border-steel/20 bg-canvas">
      {visible.map((runner) => {
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
    {runners.length > PAGE_SIZE && (
      <PagerControls
        label={`${start + 1}–${Math.min(start + PAGE_SIZE, runners.length)} of ${runners.length}`}
        hasPrev={page > 0}
        hasNext={start + PAGE_SIZE < runners.length}
        onPrev={() => setPage((current) => Math.max(0, current - 1))}
        onNext={() => setPage((current) => current + 1)}
      />
    )}
    </>
  );
}
