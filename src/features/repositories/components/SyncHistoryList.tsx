import { format, formatDistanceToNow } from 'date-fns';
import { Badge } from '../../../components/ui/Badge';
import { Spinner } from '../../../components/ui/Spinner';
import { TBody, Table, Td, Th, THead, Tr } from '../../../components/ui/Table';
import type { SyncRun } from '../../../types/repository';

const TRIGGER_LABELS: Record<SyncRun['trigger'], string> = {
  import: 'Initial import',
  manual: 'Manual',
  webhook: 'Webhook',
};

export function SyncHistoryList({ runs }: { runs: SyncRun[] }) {
  if (runs.length === 0) {
    return <p className="py-8 text-center text-sm text-steel">No synchronizations yet.</p>;
  }

  return (
    <Table>
      <THead>
        <Tr>
          <Th>Started</Th>
          <Th>Trigger</Th>
          <Th>Status</Th>
          <Th>Result</Th>
        </Tr>
      </THead>
      <TBody>
        {runs.map((run) => (
          <Tr key={run.id}>
            <Td>
              <span title={format(new Date(run.startedAt), 'PPpp')}>
                {formatDistanceToNow(new Date(run.startedAt), { addSuffix: true })}
              </span>
            </Td>
            <Td className="text-steel">{TRIGGER_LABELS[run.trigger]}</Td>
            <Td>
              {run.status === 'running' ? (
                <Badge variant="primary">
                  <Spinner className="h-3 w-3" />
                  Running
                </Badge>
              ) : run.status === 'failed' ? (
                <Badge variant="danger">Failed</Badge>
              ) : (
                <Badge variant="neutral">Success</Badge>
              )}
            </Td>
            <Td className="text-xs text-steel">
              {run.status === 'failed'
                ? run.error ?? 'failed'
                : run.status === 'success'
                  ? `${run.stats.workflows ?? 0} workflows, ${run.stats.branches ?? 0} branches`
                  : '—'}
            </Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
