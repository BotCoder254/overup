import { Badge } from '../../../components/ui/Badge';
import { Spinner } from '../../../components/ui/Spinner';
import type {
  JobConclusion,
  PipelineConclusion,
  PipelineStatus,
} from '../../../types/pipeline';

type AnyConclusion = PipelineConclusion | JobConclusion | null;

/**
 * Status presentation shared by pipelines and jobs, in the shell palette:
 * running = link blue (+ spinner), success = primary, failures = danger,
 * queued/skipped = steel, cancelled = charcoal-ish neutral.
 */
export function statusLabel(status: PipelineStatus, conclusion: AnyConclusion): string {
  if (status === 'queued') return 'Queued';
  if (status === 'in_progress') return 'Running';
  switch (conclusion) {
    case 'success':
      return 'Success';
    case 'failure':
      return 'Failed';
    case 'cancelled':
      return 'Cancelled';
    case 'timed_out':
      return 'Timed out';
    case 'partial':
      return 'Partial';
    case 'skipped':
      return 'Skipped';
    default:
      return 'Completed';
  }
}

export function PipelineStatusBadge({
  status,
  conclusion,
}: {
  status: PipelineStatus;
  conclusion: AnyConclusion;
}) {
  const label = statusLabel(status, conclusion);
  if (status === 'in_progress') {
    return (
      <Badge variant="info">
        <Spinner className="h-3 w-3" />
        {label}
      </Badge>
    );
  }
  if (status === 'queued') {
    return <Badge variant="neutral">{label}</Badge>;
  }
  switch (conclusion) {
    case 'success':
      return <Badge variant="success">{label}</Badge>;
    case 'failure':
    case 'timed_out':
      return <Badge variant="danger">{label}</Badge>;
    case 'partial':
      return <Badge variant="info">{label}</Badge>;
    default:
      return <Badge variant="neutral">{label}</Badge>;
  }
}
