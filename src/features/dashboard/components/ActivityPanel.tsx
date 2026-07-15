import { formatDistanceToNow } from 'date-fns';
import { History, ShieldAlert } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { Link } from 'react-router-dom';
import { Avatar } from '../../../components/ui/Avatar';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import { describeEvent, eventLink, severityBadgeVariant } from '../../activity/lib/eventPresentation';
import { useDashboardActivityFeed } from '../hooks/useDashboard';
import type { ActivityEvent } from '../../../types/activity';

interface ActivityPanelProps {
  slug: string;
  connected: boolean;
}

/** One slim ledger row — avatar, sentence, severity badge, relative time. */
function ActivityRow({ event, slug }: { event: ActivityEvent; slug: string }) {
  const { verb, subjectLabel } = describeEvent(event);
  const link = eventLink(event, slug);
  const createdAt = new Date(event.createdAt);

  const body = (
    <div className="flex items-start gap-3 px-4 py-3">
      <Avatar login={event.actorLogin} avatarUrl={event.actorAvatarUrl} size="sm" />
      <div className="min-w-0 flex-1">
        <p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-sm text-charcoal">
          <span className="font-medium">{event.actorLogin ?? 'System'}</span>
          <span>{verb}</span>
          {subjectLabel && <span className="break-all font-mono text-xs">{subjectLabel}</span>}
          <Badge variant={severityBadgeVariant(event.severity)}>{event.severity}</Badge>
          {event.security && (
            <ShieldAlert size={14} className="shrink-0 text-steel" aria-label="Security-sensitive event" />
          )}
        </p>
        <p className="mt-0.5 text-xs text-steel">
          {formatDistanceToNow(createdAt, { addSuffix: true })}
        </p>
      </div>
    </div>
  );

  return link ? (
    <Link to={link} className="block transition-colors hover:bg-surface">
      {body}
    </Link>
  ) : (
    body
  );
}

/**
 * Compact activity roster for the Dashboard's side rail — the same visual
 * language as RunnerHealthPanel (bordered, divided list), fed by the shared
 * keyset-paginated audit feed with a Load-more / sentinel infinite scroll.
 * The full, filterable timeline lives on the Activity page.
 */
export function ActivityPanel({ slug, connected }: ActivityPanelProps) {
  const query = useDashboardActivityFeed(connected);
  const events = (query.data?.pages ?? []).flatMap((page) => page.events);

  const sentinelRef = useRef<HTMLDivElement | null>(null);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = query;
  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (!sentinel || !hasNextPage) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting) && !isFetchingNextPage) {
          void fetchNextPage();
        }
      },
      { rootMargin: '200px' },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  if (query.isLoading) {
    return <div className="h-40 animate-pulse rounded border border-steel/20 bg-canvas" />;
  }
  if (events.length === 0) {
    return (
      <EmptyState
        icon={History}
        title="No activity yet"
        description="Imports, runs, runner changes, and secret rotations appear here."
        className="min-h-0 py-10"
      />
    );
  }

  return (
    <div className="rounded border border-steel/20 bg-canvas">
      <div className="divide-y divide-steel/10">
        {events.map((event) => (
          <ActivityRow key={event.id} event={event} slug={slug} />
        ))}
      </div>
      {hasNextPage && (
        <div ref={sentinelRef} className="flex justify-center border-t border-steel/10 p-3">
          {isFetchingNextPage ? (
            <Spinner className="h-5 w-5 text-steel" />
          ) : (
            <Button variant="secondary" size="sm" onClick={() => void fetchNextPage()}>
              Load more
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
