import { formatDistanceToNow } from 'date-fns';
import { Badge } from '../../../components/ui/Badge';
import type { EnvironmentAuditEvent } from '../../../types/environment';

function actionBadge(action: string) {
  switch (action) {
    case 'environment.created':
      return <Badge variant="success">Created</Badge>;
    case 'environment.updated':
      return <Badge variant="info">Updated</Badge>;
    case 'environment.deleted':
      return <Badge variant="danger">Deleted</Badge>;
    default:
      return <Badge variant="neutral">{action}</Badge>;
  }
}

interface EnvironmentAuditListProps {
  events: EnvironmentAuditEvent[];
  loading?: boolean;
}

/** Immutable audit trail entries for environment administration. */
export function EnvironmentAuditList({ events, loading }: EnvironmentAuditListProps) {
  if (loading) {
    return <p className="py-2 text-sm text-steel">Loading activity…</p>;
  }
  if (events.length === 0) {
    return <p className="py-2 text-sm text-steel">No environment activity recorded yet.</p>;
  }

  return (
    <ul className="divide-y divide-steel/10">
      {events.map((event, index) => {
        const name = typeof event.metadata.name === 'string' ? event.metadata.name : null;
        const deletedSecrets =
          typeof event.metadata.deletedSecrets === 'number' ? event.metadata.deletedSecrets : null;
        return (
          <li key={`${event.createdAt}-${index}`} className="flex items-start gap-2 py-2">
            <span className="shrink-0">{actionBadge(event.action)}</span>
            <span className="min-w-0 flex-1 text-xs text-charcoal">
              {name && <span className="font-mono">{name}</span>}
              {deletedSecrets !== null && deletedSecrets > 0 && (
                <span className="text-steel">
                  {' '}
                  · {deletedSecrets} secret{deletedSecrets === 1 ? '' : 's'} removed
                </span>
              )}
              <span className="mt-0.5 block text-steel">
                {event.actorLogin ?? 'system'} ·{' '}
                {formatDistanceToNow(new Date(event.createdAt), { addSuffix: true })}
              </span>
            </span>
          </li>
        );
      })}
    </ul>
  );
}
