import { Badge } from '../../../components/ui/Badge';
import { Spinner } from '../../../components/ui/Spinner';
import type { QueueReason } from '../../../types/job';

type BadgeVariant = 'neutral' | 'primary' | 'info' | 'danger' | 'success' | 'outline';

/** Static label + tone per scheduling reason. Waits that need operator
 * action (no runner) are danger; normal progress states are info/primary. */
const REASONS: Record<QueueReason, { label: string; variant: BadgeVariant; title: string }> = {
  waiting_dependencies: {
    label: 'Waiting on dependencies',
    variant: 'neutral',
    title: 'One or more `needs` jobs have not concluded successfully yet.',
  },
  no_runner_online: {
    label: 'No runner online',
    variant: 'danger',
    title: 'No connected, schedulable runner exists in this workspace.',
  },
  no_matching_runner: {
    label: 'No matching runner',
    variant: 'danger',
    title: "No online runner offers every label in the job's runs-on list.",
  },
  runners_busy: {
    label: 'Runners busy',
    variant: 'info',
    title: 'Compatible runners exist but all are executing other jobs.',
  },
  waiting_scheduler: {
    label: 'Scheduling',
    variant: 'neutral',
    title: 'A compatible idle runner exists; the scheduler is about to claim the job.',
  },
  dispatching: {
    label: 'Dispatching',
    variant: 'info',
    title: 'Assigned to a runner; waiting for the runner to acknowledge.',
  },
  starting: {
    label: 'Starting',
    variant: 'info',
    title: 'The runner is preparing the workspace and container.',
  },
  running: {
    label: 'Running',
    variant: 'primary',
    title: 'Steps are executing inside the container.',
  },
};

export function QueueReasonBadge({ reason }: { reason: QueueReason }) {
  const entry = REASONS[reason] ?? {
    label: String(reason).replaceAll('_', ' '),
    variant: 'neutral' as BadgeVariant,
    title: '',
  };
  const live = reason === 'starting' || reason === 'running' || reason === 'dispatching';
  return (
    <span title={entry.title}>
      <Badge variant={entry.variant}>
        {live && <Spinner className="h-3 w-3" />}
        {entry.label}
      </Badge>
    </span>
  );
}
