import { format, formatDistanceToNow } from 'date-fns';
import { GitBranch, GitPullRequest, Tag } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Avatar } from '../../../components/ui/Avatar';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import { Card } from '../../../components/ui/Card';
import { Spinner } from '../../../components/ui/Spinner';
import type { RepositoryEvent, RepositoryEventOutcome } from '../../../types/repository';
import { useRepositoryEvents } from '../hooks/useRepositories';

const OUTCOME_BADGES: Record<
  RepositoryEventOutcome,
  { label: string; variant: 'success' | 'info' | 'neutral' | 'danger' }
> = {
  pipelines_created: { label: 'Pipelines', variant: 'success' },
  pipelines_and_sync: { label: 'Pipelines + sync', variant: 'success' },
  sync_scheduled: { label: 'Sync', variant: 'info' },
  ignored: { label: 'Ignored', variant: 'neutral' },
  failed: { label: 'Failed', variant: 'danger' },
};

/** Static skip/ignore categories rendered as human sentences. */
const REASON_LABELS: Record<string, string> = {
  no_matching_workflows: 'no workflows listen for this event',
  filters_not_matched: 'trigger filters did not match',
  branch_deleted: 'branch was deleted',
  tag_deleted: 'tag was deleted',
  no_head_commit: 'no head commit to build',
  fork_pr_skipped: 'fork pull requests are not executed',
  pr_action_ignored: 'activity type is not handled',
  pr_closed: 'pull request closed',
  event_not_supported: 'event is not supported',
  repository_deleted: 'repository was deleted on GitHub',
  access_revoked: 'repository access was revoked',
};

function eventTitle(event: RepositoryEvent): string {
  if (event.event === 'pull_request') {
    return event.action ? `Pull request ${event.action}` : 'Pull request';
  }
  if (event.event === 'push') {
    if (event.gitRef?.startsWith('refs/tags/')) return 'Tag push';
    return 'Push';
  }
  if (event.event === 'repository') {
    return event.action ? `Repository ${event.action}` : 'Repository update';
  }
  return event.action ? `${event.event} ${event.action}` : event.event;
}

function RefChip({ event }: { event: RepositoryEvent }) {
  const ref = event.gitRef;
  if (event.event === 'pull_request' && typeof event.summary.prNumber === 'number') {
    return (
      <Badge variant="outline">
        <GitPullRequest size={10} aria-hidden="true" />
        {`#${event.summary.prNumber}`}
      </Badge>
    );
  }
  if (!ref) return null;
  if (ref.startsWith('refs/tags/')) {
    return (
      <Badge variant="outline">
        <Tag size={10} aria-hidden="true" />
        {ref.slice('refs/tags/'.length)}
      </Badge>
    );
  }
  if (ref.startsWith('refs/heads/')) {
    return (
      <Badge variant="outline">
        <GitBranch size={10} aria-hidden="true" />
        {ref.slice('refs/heads/'.length)}
      </Badge>
    );
  }
  return null;
}

function EventRow({ event, slug }: { event: RepositoryEvent; slug: string }) {
  const badge = OUTCOME_BADGES[event.outcome] ?? OUTCOME_BADGES.ignored;
  const skipped = Array.isArray(event.summary.skipped) ? event.summary.skipped : [];
  const reason = event.ignoredReason
    ? REASON_LABELS[event.ignoredReason] ?? event.ignoredReason
    : null;

  return (
    <li className="flex items-start gap-3 px-4 py-3">
      <Badge variant={badge.variant} className="mt-0.5 shrink-0">
        {badge.label}
      </Badge>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2 text-sm text-charcoal">
          <span className="font-medium">{eventTitle(event)}</span>
          <RefChip event={event} />
          {event.headSha && (
            <span className="font-mono text-xs text-steel">{event.headSha.slice(0, 7)}</span>
          )}
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-steel">
          {event.actorLogin && (
            <span className="inline-flex items-center gap-1">
              <Avatar size="xs" login={event.actorLogin} avatarUrl={event.actorAvatarUrl} />
              {event.actorLogin}
            </span>
          )}
          <time
            dateTime={event.processedAt}
            title={format(new Date(event.processedAt), 'PPpp')}
          >
            {formatDistanceToNow(new Date(event.processedAt), { addSuffix: true })}
          </time>
          {reason && <span>· {reason}</span>}
          {event.summary.merged === true && <span>· merged</span>}
          {skipped.length > 0 && (
            <span
              title={skipped
                .map((entry) => `${entry.path} — ${REASON_LABELS[entry.reason] ?? entry.reason}`)
                .join('\n')}
            >
              · {skipped.length} workflow{skipped.length === 1 ? '' : 's'} filtered out
            </span>
          )}
        </div>
        {event.pipelineIds.length > 0 && (
          <div className="mt-1.5 flex flex-wrap items-center gap-2">
            {event.pipelineIds.map((pipelineId, index) => (
              <Link
                key={pipelineId}
                to={workspacePath(slug, `pipelines/${pipelineId}`)}
                className="text-xs font-medium text-link hover:underline"
              >
                {event.pipelineIds.length === 1 ? 'View pipeline' : `Pipeline ${index + 1}`}
              </Link>
            ))}
          </div>
        )}
      </div>
    </li>
  );
}

interface RepositoryEventsListProps {
  repositoryId: string;
  slug: string;
}

/**
 * The chronological repository event timeline (the ActivityFeed compact
 * pattern): every webhook event this platform processed for the repository,
 * with what it caused — created pipelines, scheduled syncs — or the static
 * reason it was ignored. Keyset infinite scroll with a Load-more fallback.
 */
export function RepositoryEventsList({ repositoryId, slug }: RepositoryEventsListProps) {
  const query = useRepositoryEvents(repositoryId);
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
    return (
      <div className="flex justify-center py-12">
        <Spinner className="h-5 w-5 text-steel" />
      </div>
    );
  }
  if (query.isError) {
    return (
      <Card className="p-8 text-center text-sm text-steel">
        The event timeline could not be loaded.
      </Card>
    );
  }

  const events = query.data?.pages.flatMap((page) => page.events) ?? [];
  if (events.length === 0) {
    return (
      <Card className="p-8 text-center text-sm text-steel">
        No repository events processed yet. Push a commit or open a pull request — deliveries
        appear here as they are processed.
      </Card>
    );
  }

  return (
    <>
      <div className="rounded border border-steel/20 bg-canvas">
        <ul className="divide-y divide-steel/10">
          {events.map((event) => (
            <EventRow key={event.id} event={event} slug={slug} />
          ))}
        </ul>
      </div>
      {query.hasNextPage && (
        <div ref={sentinelRef} className="mt-4 flex justify-center">
          {query.isFetchingNextPage ? (
            <Spinner className="h-5 w-5 text-steel" />
          ) : (
            <Button variant="secondary" size="sm" onClick={() => void query.fetchNextPage()}>
              Load more
            </Button>
          )}
        </div>
      )}
    </>
  );
}
