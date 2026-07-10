import { Badge } from '../../../components/ui/Badge';
import { Spinner } from '../../../components/ui/Spinner';
import type { SyncStatus } from '../../../types/repository';

const LABELS: Record<SyncStatus, string> = {
  pending: 'Queued',
  syncing: 'Syncing',
  synced: 'Synced',
  failed: 'Failed',
};

export function SyncStatusBadge({ status }: { status: SyncStatus }) {
  if (status === 'syncing' || status === 'pending') {
    return (
      <Badge variant="primary">
        <Spinner className="h-3 w-3" />
        {LABELS[status]}
      </Badge>
    );
  }
  return <Badge variant={status === 'failed' ? 'danger' : 'neutral'}>{LABELS[status]}</Badge>;
}
