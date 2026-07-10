import { Badge } from '../../../components/ui/Badge';
import { Spinner } from '../../../components/ui/Spinner';
import type { RunnerStatus } from '../../../types/runner';

const LABELS: Record<RunnerStatus, string> = {
  offline: 'Offline',
  idle: 'Idle',
  busy: 'Busy',
  disabled: 'Disabled',
};

interface RunnerStatusBadgeProps {
  status: RunnerStatus;
  /** True while the runner is finishing its current job and will not take another. */
  draining?: boolean;
}

export function RunnerStatusBadge({ status, draining }: RunnerStatusBadgeProps) {
  if (status === 'busy' && draining) {
    return (
      <Badge variant="info">
        <Spinner className="h-3 w-3" />
        Draining
      </Badge>
    );
  }
  if (status === 'busy') {
    return (
      <Badge variant="primary">
        <Spinner className="h-3 w-3" />
        {LABELS[status]}
      </Badge>
    );
  }
  if (status === 'idle') {
    return <Badge variant="success">{LABELS[status]}</Badge>;
  }
  return <Badge variant="neutral">{LABELS[status]}</Badge>;
}
