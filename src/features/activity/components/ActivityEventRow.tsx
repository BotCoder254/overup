import { format, formatDistanceToNow } from 'date-fns';
import { ChevronDown, ExternalLink, ShieldAlert } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Badge } from '../../../components/ui/Badge';
import { cn } from '../../../lib/cn';
import type { ActivityEvent } from '../../../types/activity';
import {
  CATEGORY_LABELS,
  describeEvent,
  eventLink,
  severityBadgeVariant,
} from '../lib/eventPresentation';
import { Avatar } from '../../../components/ui/Avatar';

/** "token_regenerated" → "Token regenerated" for the severity badge. */
function actionLabel(action: string): string {
  const suffix = action.split('.').pop() ?? action;
  const words = suffix.replace(/_/g, ' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5 sm:flex-row sm:gap-2">
      <dt className="w-28 shrink-0 text-xs font-medium uppercase tracking-wider text-steel">
        {label}
      </dt>
      <dd className="min-w-0 break-all font-mono text-xs text-charcoal">{value}</dd>
    </div>
  );
}

interface ActivityEventRowProps {
  event: ActivityEvent;
  slug: string;
  /** Clicking the actor's name narrows the feed to that actor. */
  onSelectActor?: (actorId: string, login: string | null) => void;
}

/**
 * One immutable ledger entry: circular actor avatar, human-readable
 * sentence, severity badge, relative timestamp — expandable into the full
 * audit detail (identifiers, request id, metadata) with a deep link to the
 * subject's page when it still exists. Audit metadata is value-free by
 * construction, so everything here is safe to render verbatim.
 */
export function ActivityEventRow({ event, slug, onSelectActor }: ActivityEventRowProps) {
  const [expanded, setExpanded] = useState(false);
  const actorId = event.actorId;
  const { verb, subjectLabel } = describeEvent(event);
  const link = eventLink(event, slug);
  const createdAt = new Date(event.createdAt);
  const metadataEntries = Object.entries(event.metadata);

  return (
    <li className="py-3">
      <div className="flex items-start gap-3">
        <Avatar login={event.actorLogin} avatarUrl={event.actorAvatarUrl} />

        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-sm text-charcoal">
            {actorId && onSelectActor ? (
              <button
                type="button"
                onClick={() => onSelectActor(actorId, event.actorLogin)}
                title={`Filter by ${event.actorLogin ?? 'this actor'}`}
                className="rounded font-medium hover:text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
              >
                {event.actorLogin ?? 'System'}
              </button>
            ) : (
              <span className="font-medium">{event.actorLogin ?? 'System'}</span>
            )}
            <span>{verb}</span>
            {subjectLabel && (
              <span className="break-all font-mono text-xs">{subjectLabel}</span>
            )}
            <Badge variant={severityBadgeVariant(event.severity)}>
              {actionLabel(event.action)}
            </Badge>
            {event.security && (
              <ShieldAlert
                size={14}
                className="shrink-0 text-steel"
                aria-label="Security-sensitive event"
              />
            )}
            <Badge variant="outline" className="hidden sm:inline-flex">
              {CATEGORY_LABELS[event.category] ?? event.category}
            </Badge>
          </p>
          <p className="mt-0.5 text-xs text-steel">
            <time dateTime={event.createdAt} title={format(createdAt, 'PPpp')}>
              {formatDistanceToNow(createdAt, { addSuffix: true })}
            </time>
          </p>
        </div>

        <button
          type="button"
          onClick={() => setExpanded((current) => !current)}
          aria-expanded={expanded}
          aria-label={expanded ? 'Collapse event details' : 'Expand event details'}
          className="shrink-0 rounded p-1 text-steel transition-colors hover:bg-surface hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <ChevronDown
            size={16}
            aria-hidden="true"
            className={cn('transition-transform', expanded && 'rotate-180')}
          />
        </button>
      </div>

      {expanded && (
        <div className="ml-11 mt-2 rounded border border-steel/10 bg-surface/60 p-3">
          <dl className="space-y-1.5">
            <DetailRow label="Event id" value={event.id} />
            {event.requestId && <DetailRow label="Request id" value={event.requestId} />}
            <DetailRow label="Subject" value={event.subjectType} />
            {event.subjectId && <DetailRow label="Subject id" value={event.subjectId} />}
            <DetailRow label="Category" value={CATEGORY_LABELS[event.category] ?? event.category} />
            {metadataEntries.map(([key, value]) => (
              <DetailRow
                key={key}
                label={key}
                value={typeof value === 'string' ? value : JSON.stringify(value)}
              />
            ))}
          </dl>
          {link && (
            <Link
              to={link}
              className="mt-3 inline-flex items-center gap-1 text-xs font-medium text-link hover:underline"
            >
              <ExternalLink size={12} aria-hidden="true" />
              View {event.subjectType.replace('_', ' ')}
            </Link>
          )}
        </div>
      )}
    </li>
  );
}
