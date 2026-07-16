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
  className?: string;
}

export function RunnerStatusBadge({ status, draining, className }: RunnerStatusBadgeProps) {
  if (status === 'busy' && draining) {
    return (
      <Badge variant="info" className={className}>
        <Spinner className="h-3 w-3" />
        Draining
      </Badge>
    );
  }
  if (status === 'busy') {
    return (
      <Badge variant="primary" className={className}>
        <Spinner className="h-3 w-3" />
        {LABELS[status]}
      </Badge>
    );
  }
  if (status === 'idle') {
    return (
      <Badge variant="success" className={className}>
        {LABELS[status]}
      </Badge>
    );
  }
  return (
    <Badge variant="neutral" className={className}>
      {LABELS[status]}
    </Badge>
  );
}
