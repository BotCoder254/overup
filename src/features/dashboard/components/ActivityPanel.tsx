import { formatDistanceToNow } from 'date-fns';
import { ChevronLeft, ChevronRight, History, ShieldAlert } from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { Avatar } from '../../../components/ui/Avatar';
import { Badge } from '../../../components/ui/Badge';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import { cn } from '../../../lib/cn';
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

/** A single square page-number button. */
function PageButton({
  label,
  active,
  disabled,
  onClick,
  children,
}: {
  label: string;
  active?: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-current={active ? 'page' : undefined}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        'flex h-7 min-w-7 items-center justify-center rounded border px-2 text-xs font-medium transition-colors',
        active
          ? 'border-primary bg-primary text-white'
          : 'border-steel/20 text-charcoal hover:bg-surface',
        disabled && 'cursor-not-allowed opacity-40 hover:bg-transparent',
      )}
    >
      {children}
    </button>
  );
}

/**
 * Compact activity roster for the Dashboard's side rail — the same visual
 * language as RunnerHealthPanel (bordered, divided list), fed by the keyset
 * audit feed at five events per page. Navigation is numbered (Prev / 1 2 3 /
 * Next) rather than infinite scroll: each page shows a consistent five, and
 * Next fetches the following cursor page on demand. The full, filterable
 * timeline lives on the Activity page.
 */
export function ActivityPanel({ slug, connected }: ActivityPanelProps) {
  const query = useDashboardActivityFeed(connected);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = query;
  const pages = query.data?.pages ?? [];
  const [page, setPage] = useState(0);

  // A live refetch can shrink the page set (e.g. after invalidation the feed
  // resets to page 1) — keep the index in range.
  useEffect(() => {
    if (page > 0 && page >= pages.length) {
      setPage(Math.max(0, pages.length - 1));
    }
  }, [page, pages.length]);

  if (query.isLoading) {
    return <div className="h-40 animate-pulse rounded border border-steel/20 bg-canvas" />;
  }

  const currentEvents = pages[page]?.events ?? [];
  if (pages.length === 0 || (pages.length === 1 && currentEvents.length === 0)) {
    return (
      <EmptyState
        icon={History}
        title="No activity yet"
        description="Imports, runs, runner changes, and secret rotations appear here."
        className="min-h-0 py-10"
      />
    );
  }

  const canGoNext = page < pages.length - 1 || hasNextPage;
  const goNext = () => {
    if (page < pages.length - 1) {
      setPage(page + 1);
    } else if (hasNextPage && !isFetchingNextPage) {
      void fetchNextPage().then(() => setPage((current) => current + 1));
    }
  };

  return (
    <div className="rounded border border-steel/20 bg-canvas">
      <div className="divide-y divide-steel/10">
        {currentEvents.map((event) => (
          <ActivityRow key={event.id} event={event} slug={slug} />
        ))}
      </div>

      <div className="flex items-center justify-between gap-2 border-t border-steel/10 p-3">
        <PageButton label="Previous page" disabled={page === 0} onClick={() => setPage(page - 1)}>
          <ChevronLeft size={14} aria-hidden="true" />
        </PageButton>

        <div className="flex items-center gap-1">
          {pages.map((_, index) => (
            <PageButton
              key={index}
              label={`Page ${index + 1}`}
              active={index === page}
              onClick={() => setPage(index)}
            >
              {index + 1}
            </PageButton>
          ))}
          {/* Loaded pages are known; a next cursor means more may follow. */}
          {hasNextPage && <span className="px-1 text-xs text-steel">…</span>}
        </div>

        <PageButton label="Next page" disabled={!canGoNext} onClick={goNext}>
          {isFetchingNextPage ? (
            <Spinner className="h-3.5 w-3.5" />
          ) : (
            <ChevronRight size={14} aria-hidden="true" />
          )}
        </PageButton>
      </div>
    </div>
  );
}
