import { formatDistanceToNow } from 'date-fns';
import { AlertTriangle, Server } from 'lucide-react';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Badge } from '../../../components/ui/Badge';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { Spinner } from '../../../components/ui/Spinner';
import type { QueueSummary } from '../../../types/job';
import type { Runner } from '../../../types/runner';
import { RunnerStatusBadge } from '../../runners/components/RunnerStatusBadge';

interface RunnerAvailabilityPanelProps {
  slug: string;
  runners: Runner[] | undefined;
  loading: boolean;
  summary: QueueSummary | undefined;
}

/** Jobs queued past this long while idle runners exist deserve attention. */
const STALLED_QUEUE_SECS = 300;

/**
 * The infrastructure side of the queue: the runner fleet (live via the
 * workspace stream) plus a scheduler-health note when the oldest wait
 * looks wrong for the available capacity.
 */
export function RunnerAvailabilityPanel({
  slug,
  runners,
  loading,
  summary,
}: RunnerAvailabilityPanelProps) {
  const fleet = (runners ?? []).filter((runner) => !runner.revoked);
  const oldestWaitSecs = summary?.oldestQueuedAt
    ? Math.max((Date.now() - new Date(summary.oldestQueuedAt).getTime()) / 1000, 0)
    : null;
  const stalled =
    oldestWaitSecs !== null &&
    oldestWaitSecs > STALLED_QUEUE_SECS &&
    (summary?.runnersIdle ?? 0) > 0;

  return (
    <Card>
      <CardHeader>
        <h2 className="text-sm font-semibold text-charcoal">Runner availability</h2>
        <p className="text-xs text-steel">
          Jobs are matched to runners by label containment — every requested label must be
          offered by the runner.
        </p>
      </CardHeader>
      <CardBody>
        {loading ? (
          <div className="flex justify-center py-6">
            <Spinner className="h-5 w-5 text-steel" />
          </div>
        ) : fleet.length === 0 ? (
          <div className="flex flex-col items-center gap-2 py-6 text-center">
            <Server size={40} strokeWidth={1.25} className="text-steel" aria-hidden="true" />
            <p className="text-sm text-steel">
              No runners registered — queued jobs cannot start without one.
            </p>
            <Link
              to={workspacePath(slug, 'runners')}
              className="text-sm text-link hover:underline"
            >
              Register a runner
            </Link>
          </div>
        ) : (
          <ul className="divide-y divide-steel/10">
            {fleet.map((runner) => (
              <li key={runner.id} className="flex items-start justify-between gap-2 py-2.5">
                <div className="min-w-0">
                  <Link
                    to={workspacePath(slug, `runners/${runner.id}`)}
                    className="text-sm font-medium text-charcoal hover:text-link hover:underline"
                  >
                    {runner.name}
                  </Link>
                  <div className="mt-1 flex flex-wrap gap-1">
                    {runner.labels.map((label) => (
                      <Badge key={label} variant="outline">
                        {label}
                      </Badge>
                    ))}
                  </div>
                  {runner.lastSeenAt && (
                    <div className="mt-1 text-xs text-steel">
                      seen {formatDistanceToNow(new Date(runner.lastSeenAt), { addSuffix: true })}
                    </div>
                  )}
                </div>
                <RunnerStatusBadge status={runner.status} draining={runner.draining} />
              </li>
            ))}
          </ul>
        )}

        {stalled && (
          <div className="mt-3 flex items-start gap-2 rounded border border-danger/30 bg-danger/5 p-2.5 text-xs text-charcoal">
            <AlertTriangle size={14} className="mt-0.5 shrink-0 text-danger" aria-hidden="true" />
            <span>
              The oldest queued job has been waiting more than{' '}
              {Math.round(STALLED_QUEUE_SECS / 60)} minutes while idle runners exist — check the
              waiting jobs&apos; label requirements against the fleet above.
            </span>
          </div>
        )}
      </CardBody>
    </Card>
  );
}
