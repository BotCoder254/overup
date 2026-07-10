import { Layers } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import type { PipelineStatus } from '../../../types/pipeline';
import type { PipelineFilters as ApiFilters } from '../api/pipelinesApi';
import {
  EMPTY_LEDGER_FILTERS,
  PipelineFilters,
  type LedgerFilterState,
} from '../components/PipelineFilters';
import { PipelinesTable } from '../components/PipelinesTable';
import { usePipelines } from '../hooks/usePipelines';

const STATUSES: readonly string[] = ['queued', 'in_progress', 'completed'];
const CONCLUSIONS: readonly string[] = [
  'success',
  'failure',
  'partial',
  'cancelled',
  'timed_out',
];

/** Trailing-edge debounce for the free-text inputs. */
function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const handle = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(handle);
  }, [value, delayMs]);
  return debounced;
}

/** yyyy-mm-dd -> RFC3339 at the start (or exclusive end) of that UTC day. */
function dayBound(date: string, end: boolean): string | undefined {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return undefined;
  return `${date}T${end ? '23:59:59' : '00:00:00'}Z`;
}

function filtersFromParams(params: URLSearchParams): LedgerFilterState {
  return {
    state: params.get('state') ?? '',
    repositoryId: params.get('repo') ?? '',
    workflowId: params.get('workflow') ?? '',
    trigger: params.get('trigger') ?? '',
    branch: params.get('branch') ?? '',
    q: params.get('q') ?? '',
    from: params.get('from') ?? '',
    to: params.get('to') ?? '',
  };
}

/**
 * The workspace's execution ledger: every pipeline across every repository,
 * newest first, filterable (state, repository, workflow, trigger, branch,
 * date range, free-text) and keyset-paginated with sentinel-driven infinite
 * scroll. Filters live in the URL so views are shareable. Rows poll while
 * anything is live; selecting one opens the execution workspace.
 */
export function PipelinesPage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const [searchParams, setSearchParams] = useSearchParams();
  const [filters, setFilters] = useState<LedgerFilterState>(() =>
    filtersFromParams(searchParams),
  );

  // Mirror the filters into the URL (replace — no history spam).
  useEffect(() => {
    const next = new URLSearchParams();
    if (filters.state) next.set('state', filters.state);
    if (filters.repositoryId) next.set('repo', filters.repositoryId);
    if (filters.workflowId) next.set('workflow', filters.workflowId);
    if (filters.trigger) next.set('trigger', filters.trigger);
    if (filters.branch) next.set('branch', filters.branch);
    if (filters.q) next.set('q', filters.q);
    if (filters.from) next.set('from', filters.from);
    if (filters.to) next.set('to', filters.to);
    setSearchParams(next, { replace: true });
  }, [filters, setSearchParams]);

  const debouncedQ = useDebouncedValue(filters.q, 300);
  const debouncedBranch = useDebouncedValue(filters.branch, 300);

  const apiFilters = useMemo<ApiFilters>(() => {
    const isStatus = STATUSES.includes(filters.state);
    const isConclusion = CONCLUSIONS.includes(filters.state);
    return {
      repositoryId: filters.repositoryId || undefined,
      workflowId: filters.workflowId || undefined,
      status: isStatus ? (filters.state as PipelineStatus) : undefined,
      conclusion: isConclusion ? filters.state : undefined,
      trigger: filters.trigger || undefined,
      branch: debouncedBranch.trim() || undefined,
      q: debouncedQ.trim() || undefined,
      createdAfter: dayBound(filters.from, false),
      createdBefore: dayBound(filters.to, true),
    };
  }, [filters, debouncedQ, debouncedBranch]);

  const query = usePipelines(apiFilters);
  const pipelines = (query.data?.pages ?? []).flatMap((page) => page.pipelines);
  const hasFilters = Object.values(filters).some(Boolean);

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
        title="Pipelines"
        description="Every pipeline execution across the workspace — status, duration, triggers, and full run history."
      />

      <PipelineFilters
        value={filters}
        onChange={(patch) => setFilters((current) => ({ ...current, ...patch }))}
      />

      {query.isLoading ? (
        <div className="flex min-h-[40vh] items-center justify-center">
          <Spinner className="h-6 w-6 text-steel" />
        </div>
      ) : pipelines.length === 0 ? (
        <div>
          <EmptyState
            icon={Layers}
            title={hasFilters ? 'No matching pipelines' : 'No pipelines yet'}
            description={
              hasFilters
                ? 'Nothing matches the current filters. Clear them to see the full execution history.'
                : 'Pipelines appear here when a push hits a connected repository or a workflow is dispatched manually from its detail page.'
            }
          />
          {hasFilters && (
            <div className="mt-3 flex justify-center">
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setFilters(EMPTY_LEDGER_FILTERS)}
              >
                Clear filters
              </Button>
            </div>
          )}
        </div>
      ) : (
        <>
          <PipelinesTable slug={slug} pipelines={pipelines} />
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
    </>
  );
}
