import { Activity, Download } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { toast } from 'sonner';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import { useWorkspaceStream } from '../../dashboard/hooks/useWorkspaceStream';
import {
  exportActivityFeed,
  type ActivityFeedFilters as ApiFilters,
} from '../api/activityApi';
import { ActivityDistributionCard } from '../components/ActivityDistributionCard';
import { ActivityFeed } from '../components/ActivityFeed';
import {
  ActivityFilterBar,
  EMPTY_ACTIVITY_FILTERS,
  type ActivityFilterState,
} from '../components/ActivityFilterBar';
import { ActivitySummaryStrip } from '../components/ActivitySummaryStrip';
import { useActivityFeed, useActivitySummary, useWorkspaceId } from '../hooks/useActivity';

/** Trailing-edge debounce for the free-text input. */
function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const handle = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(handle);
  }, [value, delayMs]);
  return debounced;
}

/** yyyy-mm-dd -> RFC3339 at the start (or end) of that UTC day. */
function dayBound(date: string, end: boolean): string | undefined {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return undefined;
  return `${date}T${end ? '23:59:59' : '00:00:00'}Z`;
}

function filtersFromParams(params: URLSearchParams): ActivityFilterState {
  return {
    q: params.get('q') ?? '',
    category: params.get('category') ?? '',
    action: params.get('action') ?? '',
    actorId: params.get('actorId') ?? '',
    actorLogin: params.get('actor') ?? '',
    from: params.get('from') ?? '',
    to: params.get('to') ?? '',
  };
}

/**
 * The workspace activity feed: the operational audit center, read straight
 * off the immutable audit_logs ledger. The wider column is the
 * chronological timeline (filters live in the URL so views are shareable,
 * keyset-paginated with sentinel-driven infinite scroll); the narrower
 * right column carries the category distribution, which doubles as a
 * filter. Live updates ride the workspace stream with a polling fallback.
 */
export function ActivityPage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const [searchParams, setSearchParams] = useSearchParams();
  const [filters, setFilters] = useState<ActivityFilterState>(() =>
    filtersFromParams(searchParams),
  );

  // Mirror the filters into the URL (replace — no history spam).
  useEffect(() => {
    const next = new URLSearchParams();
    if (filters.q) next.set('q', filters.q);
    if (filters.category) next.set('category', filters.category);
    if (filters.action) next.set('action', filters.action);
    if (filters.actorId) next.set('actorId', filters.actorId);
    if (filters.actorLogin) next.set('actor', filters.actorLogin);
    if (filters.from) next.set('from', filters.from);
    if (filters.to) next.set('to', filters.to);
    setSearchParams(next, { replace: true });
  }, [filters, setSearchParams]);

  const debouncedQ = useDebouncedValue(filters.q, 300);
  const apiFilters = useMemo<ApiFilters>(
    () => ({
      q: debouncedQ.trim() || undefined,
      category: filters.category || undefined,
      action: filters.action || undefined,
      actorId: filters.actorId || undefined,
      createdAfter: dayBound(filters.from, false),
      createdBefore: dayBound(filters.to, true),
    }),
    [filters, debouncedQ],
  );

  // Connected: workspace-stream frames invalidate the feed queries.
  // Disconnected: the hooks fall back to a slow poll.
  const { connected } = useWorkspaceStream();
  const summary = useActivitySummary(connected);
  const query = useActivityFeed(apiFilters, connected);
  const events = (query.data?.pages ?? []).flatMap((page) => page.events);
  const hasFilters = Object.values(filters).some(Boolean);

  // Compliance export: the current filters ride along; the server
  // re-validates them, caps the rows, and hardens the CSV.
  const workspaceId = useWorkspaceId();
  const [exporting, setExporting] = useState(false);
  const handleExport = async () => {
    if (!workspaceId || exporting) return;
    setExporting(true);
    try {
      const blob = await exportActivityFeed(workspaceId, apiFilters);
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = 'activity-export.csv';
      anchor.click();
      URL.revokeObjectURL(url);
    } catch {
      toast.error('Export failed — try again.');
    } finally {
      setExporting(false);
    }
  };

  // Infinite scroll: fetch the next page when the sentinel becomes visible.
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

  return (
    <>
      <PageHeader
        title="Activity"
        description="A live feed of everything happening in this workspace — pushes, runs, deploys, and configuration changes, recorded on an immutable audit ledger."
        actions={
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void handleExport()}
            disabled={exporting || query.isLoading}
          >
            <Download size={14} aria-hidden="true" />
            {exporting ? 'Exporting…' : 'Export CSV'}
          </Button>
        }
      />

      <ActivitySummaryStrip
        summary={summary.data}
        loading={summary.isLoading}
        error={summary.isError}
      />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
        <div className="min-w-0">
          <ActivityFilterBar
            value={filters}
            onChange={(patch) => setFilters((current) => ({ ...current, ...patch }))}
          />

          {query.isLoading ? (
            <div className="flex min-h-[40vh] items-center justify-center">
              <Spinner className="h-6 w-6 text-steel" />
            </div>
          ) : events.length === 0 ? (
            <div>
              <EmptyState
                icon={Activity}
                title={hasFilters ? 'No matching events' : 'No activity yet'}
                description={
                  hasFilters
                    ? 'Nothing matches the current filters. Clear them to see the full ledger.'
                    : 'Every meaningful operation — imports, runs, runner changes, secret rotations — lands here as an immutable audit event.'
                }
              />
              {hasFilters && (
                <div className="mt-3 flex justify-center">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => setFilters(EMPTY_ACTIVITY_FILTERS)}
                  >
                    Clear filters
                  </Button>
                </div>
              )}
            </div>
          ) : (
            <>
              <ActivityFeed
                events={events}
                slug={slug}
                onSelectActor={(actorId, login) =>
                  setFilters((current) => ({
                    ...current,
                    actorId,
                    actorLogin: login ?? '',
                  }))
                }
              />
              {query.hasNextPage && (
                <div ref={sentinelRef} className="mt-4 flex justify-center">
                  {query.isFetchingNextPage ? (
                    <Spinner className="h-5 w-5 text-steel" />
                  ) : (
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => void query.fetchNextPage()}
                    >
                      Load more
                    </Button>
                  )}
                </div>
              )}
            </>
          )}
        </div>

        <div className="min-w-0 space-y-4">
          <ActivityDistributionCard
            summary={summary.data}
            loading={summary.isLoading}
            activeCategory={filters.category}
            onSelectCategory={(category) => setFilters((current) => ({ ...current, category, action: '' }))}
          />
        </div>
      </div>
    </>
  );
}
