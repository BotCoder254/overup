import { formatDistanceToNow } from 'date-fns';
import { Badge } from '../../../components/ui/Badge';
import type { SecretAuditEvent } from '../../../types/secret';

function actionBadge(action: string) {
  switch (action) {
    case 'secret.created':
      return <Badge variant="success">Created</Badge>;
    case 'secret.updated':
      return <Badge variant="info">Updated</Badge>;
    case 'secret.deleted':
      return <Badge variant="danger">Deleted</Badge>;
    default:
      return <Badge variant="neutral">{action}</Badge>;
  }
}

interface SecretAuditListProps {
  events: SecretAuditEvent[];
  loading?: boolean;
}

/**
 * Immutable audit trail entries for secret administration. Metadata never
 * contains values — only names, scopes, and which field changed.
 */
export function SecretAuditList({ events, loading }: SecretAuditListProps) {
  if (loading) {
    return <p className="py-2 text-sm text-steel">Loading activity…</p>;
  }
  if (events.length === 0) {
    return <p className="py-2 text-sm text-steel">No secret activity recorded yet.</p>;
  }

  return (
    <ul className="divide-y divide-steel/10">
      {events.map((event, index) => {
        const name = typeof event.metadata.name === 'string' ? event.metadata.name : null;
        const field = typeof event.metadata.field === 'string' ? event.metadata.field : null;
        return (
          <li key={`${event.createdAt}-${index}`} className="flex items-start gap-2 py-2">
            <span className="shrink-0">{actionBadge(event.action)}</span>
            <span className="min-w-0 flex-1 text-xs text-charcoal">
              {name && <span className="font-mono">{name}</span>}
              {field && <span className="text-steel"> · {field} changed</span>}
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
