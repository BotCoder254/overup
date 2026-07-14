import { ListChecks } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import type { QueueJob } from '../../../types/job';
import { useWorkspaceStream } from '../../dashboard/hooks/useWorkspaceStream';
import { useCancelJob } from '../../pipelines/hooks/usePipelines';
import { useRunners } from '../../runners/hooks/useRunners';
import type { QueueFilters as ApiFilters } from '../api/jobsApi';
import {
  EMPTY_QUEUE_FILTERS,
  QueueFilters,
  type QueueFilterState,
} from '../components/QueueFilters';
import { QueueSummaryStrip } from '../components/QueueSummaryStrip';
import { QueueTable } from '../components/QueueTable';
import { RunnerAvailabilityPanel } from '../components/RunnerAvailabilityPanel';
import { useQueueJobs, useQueueSummary } from '../hooks/useJobs';

/** Trailing-edge debounce for the free-text inputs. */
function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const handle = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(handle);
  }, [value, delayMs]);
  return debounced;
}

function filtersFromParams(params: URLSearchParams): QueueFilterState {
  return {
    status: params.get('status') ?? '',
    repositoryId: params.get('repo') ?? '',
    workflowId: params.get('workflow') ?? '',
    runnerId: params.get('runner') ?? '',
    label: params.get('label') ?? '',
    q: params.get('q') ?? '',
  };
}

/**
 * The scheduler's console: every active job across the workspace in queue
 * order, each annotated with the server-computed reason it has not started
 * (dependencies, runner availability, label mismatch, capacity), beside the
 * runner fleet that explains it. Filters live in the URL; the list polls
 * briskly while anything is active and the workspace stream nudges it on
 * pipeline transitions.
 */
export function JobQueuePage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const [searchParams, setSearchParams] = useSearchParams();
  const [filters, setFilters] = useState<QueueFilterState>(() =>
    filtersFromParams(searchParams),
  );
  const [cancelTarget, setCancelTarget] = useState<QueueJob | null>(null);

  // Mirror the filters into the URL (replace — no history spam).
  useEffect(() => {
    const next = new URLSearchParams();
    if (filters.status) next.set('status', filters.status);
    if (filters.repositoryId) next.set('repo', filters.repositoryId);
    if (filters.workflowId) next.set('workflow', filters.workflowId);
    if (filters.runnerId) next.set('runner', filters.runnerId);
    if (filters.label) next.set('label', filters.label);
    if (filters.q) next.set('q', filters.q);
    setSearchParams(next, { replace: true });
  }, [filters, setSearchParams]);

  const debouncedQ = useDebouncedValue(filters.q, 300);
  const debouncedLabel = useDebouncedValue(filters.label, 300);

  const apiFilters = useMemo<ApiFilters>(
    () => ({
      status:
        filters.status === 'queued' || filters.status === 'in_progress'
          ? filters.status
          : undefined,
      repositoryId: filters.repositoryId || undefined,
      workflowId: filters.workflowId || undefined,
      runnerId: filters.runnerId || undefined,
      label: debouncedLabel.trim() || undefined,
      q: debouncedQ.trim() || undefined,
    }),
    [filters, debouncedQ, debouncedLabel],
  );

  const stream = useWorkspaceStream();
  const query = useQueueJobs(apiFilters, stream.connected);
  const summary = useQueueSummary(stream.connected);
  const runners = useRunners(stream.connected);
  const cancelJob = useCancelJob();

  const jobs = (query.data?.pages ?? []).flatMap((page) => page.jobs);
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
        title="Jobs"
        description="Queued and running jobs across every pipeline — queue position, wait reason, and the runner capacity behind them."
      />

      <QueueSummaryStrip
        summary={summary.data}
        loading={summary.isLoading}
        error={summary.isError}
      />

      <QueueFilters
        value={filters}
        onChange={(patch) => setFilters((current) => ({ ...current, ...patch }))}
        runners={runners.data ?? []}
      />

      <div className="grid gap-4 lg:grid-cols-3">
        <div className="min-w-0 lg:col-span-2">
          {query.isLoading ? (
            <div className="flex min-h-[40vh] items-center justify-center">
              <Spinner className="h-6 w-6 text-steel" />
            </div>
          ) : jobs.length === 0 ? (
            <div>
              <EmptyState
                icon={ListChecks}
                className="min-h-[40vh]"
                title={hasFilters ? 'No matching jobs' : 'The queue is clear'}
                description={
                  hasFilters
                    ? 'Nothing active matches the current filters. Clear them to see every queued and running job.'
                    : 'Every job has been executed. New jobs appear here the moment a pipeline is triggered.'
                }
              />
              {hasFilters && (
                <div className="mt-3 flex justify-center">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => setFilters(EMPTY_QUEUE_FILTERS)}
                  >
                    Clear filters
                  </Button>
                </div>
              )}
            </div>
          ) : (
            <>
              <QueueTable slug={slug} jobs={jobs} onCancel={setCancelTarget} />
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

        <RunnerAvailabilityPanel
          slug={slug}
          runners={runners.data}
          loading={runners.isLoading}
          summary={summary.data}
        />
      </div>

      <Dialog
        open={cancelTarget !== null}
        onClose={() => setCancelTarget(null)}
        title={`Cancel ${cancelTarget?.name ?? cancelTarget?.key ?? 'this job'}?`}
        description="A queued job stops immediately; a running job is signalled to abort. Jobs that depend on it will be skipped. This cannot be undone."
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setCancelTarget(null)}>
              Keep it
            </Button>
            <Button
              variant="primary"
              size="sm"
              isLoading={cancelJob.isPending}
              onClick={() => {
                if (!cancelTarget) return;
                cancelJob.mutate(
                  { pipelineId: cancelTarget.pipelineId, jobId: cancelTarget.id },
                  { onSettled: () => setCancelTarget(null) },
                );
              }}
            >
              Cancel job
            </Button>
          </>
        }
      />
    </>
  );
}
