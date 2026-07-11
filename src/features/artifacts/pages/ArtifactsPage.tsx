import { Clock, Package } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import { useWorkspaceStream } from '../../dashboard/hooks/useWorkspaceStream';
import type { ArtifactCatalogFilters as ApiFilters } from '../api/artifactsApi';
import {
  ArtifactFilters,
  EMPTY_ARTIFACT_FILTERS,
  type ArtifactFilterState,
} from '../components/ArtifactFilters';
import { ArtifactsSummaryStrip } from '../components/ArtifactsSummaryStrip';
import { ArtifactsTable } from '../components/ArtifactsTable';
import { RetentionPolicyDialog } from '../components/RetentionPolicyDialog';
import { useArtifactsCatalog, useArtifactsSummary } from '../hooks/useArtifactsCatalog';

/** Trailing-edge debounce for the free-text input. */
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

function filtersFromParams(params: URLSearchParams): ArtifactFilterState {
  return {
    q: params.get('q') ?? '',
    status: params.get('status') ?? '',
    repositoryId: params.get('repo') ?? '',
    workflowId: params.get('workflow') ?? '',
    branch: params.get('branch') ?? '',
    kind: params.get('kind') ?? '',
    retention: params.get('retention') ?? '',
    minMb: params.get('min') ?? '',
    maxMb: params.get('max') ?? '',
    job: params.get('job') ?? '',
    from: params.get('from') ?? '',
    to: params.get('to') ?? '',
  };
}

/** MB text input -> whole bytes, stringified for the API (empty on junk). */
function mbToBytes(raw: string): string | undefined {
  const mb = Number(raw);
  if (!raw.trim() || !Number.isFinite(mb) || mb < 0) return undefined;
  return String(Math.round(mb * 1024 * 1024));
}

/**
 * The workspace artifact catalog: every execution output across every
 * repository, newest first, filterable and keyset-paginated with
 * sentinel-driven infinite scroll. Filters live in the URL so views are
 * shareable. Selecting a row opens the artifact detail page.
 */
export function ArtifactsPage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const [searchParams, setSearchParams] = useSearchParams();
  const [filters, setFilters] = useState<ArtifactFilterState>(() =>
    filtersFromParams(searchParams),
  );

  const [retentionOpen, setRetentionOpen] = useState(false);

  // Live catalog refresh: artifact_update frames on the workspace stream
  // invalidate the artifact query prefix (poll-while-pending remains the
  // fallback while disconnected).
  useWorkspaceStream();

  // Mirror the filters into the URL (replace — no history spam).
  useEffect(() => {
    const next = new URLSearchParams();
    if (filters.q) next.set('q', filters.q);
    if (filters.status) next.set('status', filters.status);
    if (filters.repositoryId) next.set('repo', filters.repositoryId);
    if (filters.workflowId) next.set('workflow', filters.workflowId);
    if (filters.branch) next.set('branch', filters.branch);
    if (filters.kind) next.set('kind', filters.kind);
    if (filters.retention) next.set('retention', filters.retention);
    if (filters.minMb) next.set('min', filters.minMb);
    if (filters.maxMb) next.set('max', filters.maxMb);
    if (filters.job) next.set('job', filters.job);
    if (filters.from) next.set('from', filters.from);
    if (filters.to) next.set('to', filters.to);
    setSearchParams(next, { replace: true });
  }, [filters, setSearchParams]);

  const debouncedQ = useDebouncedValue(filters.q, 300);
  const debouncedBranch = useDebouncedValue(filters.branch, 300);
  const debouncedJob = useDebouncedValue(filters.job, 300);
  const debouncedMinMb = useDebouncedValue(filters.minMb, 300);
  const debouncedMaxMb = useDebouncedValue(filters.maxMb, 300);

  const apiFilters = useMemo<ApiFilters>(
    () => ({
      q: debouncedQ.trim() || undefined,
      status: filters.status || undefined,
      repositoryId: filters.repositoryId || undefined,
      workflowId: filters.workflowId || undefined,
      branch: debouncedBranch.trim() || undefined,
      kind: filters.kind || undefined,
      retention: filters.retention || undefined,
      minSize: mbToBytes(debouncedMinMb),
      maxSize: mbToBytes(debouncedMaxMb),
      job: debouncedJob.trim() || undefined,
      createdAfter: dayBound(filters.from, false),
      createdBefore: dayBound(filters.to, true),
    }),
    [filters, debouncedQ, debouncedBranch, debouncedJob, debouncedMinMb, debouncedMaxMb],
  );

  const summary = useArtifactsSummary();
  const query = useArtifactsCatalog(apiFilters);
  const artifacts = (query.data?.pages ?? []).flatMap((page) => page.artifacts);
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
        title="Artifacts"
        description="Build outputs produced by pipeline runs — browse, download, and manage retention across the workspace."
        actions={
          <Button variant="secondary" size="sm" onClick={() => setRetentionOpen(true)}>
            <Clock size={14} aria-hidden="true" />
            Retention
          </Button>
        }
      />

      <RetentionPolicyDialog open={retentionOpen} onClose={() => setRetentionOpen(false)} />

      <ArtifactsSummaryStrip
        summary={summary.data}
        loading={summary.isLoading}
        error={summary.isError}
      />

      <ArtifactFilters
        value={filters}
        onChange={(patch) => setFilters((current) => ({ ...current, ...patch }))}
      />

      {query.isLoading ? (
        <div className="flex min-h-[40vh] items-center justify-center">
          <Spinner className="h-6 w-6 text-steel" />
        </div>
      ) : artifacts.length === 0 ? (
        <div>
          <EmptyState
            icon={Package}
            title={hasFilters ? 'No matching artifacts' : 'No artifacts yet'}
            description={
              hasFilters
                ? 'Nothing matches the current filters. Clear them to see every stored artifact.'
                : 'Files a job leaves in .overup/artifacts/ inside its workspace are uploaded and cataloged here.'
            }
          />
          {hasFilters && (
            <div className="mt-3 flex justify-center">
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setFilters(EMPTY_ARTIFACT_FILTERS)}
              >
                Clear filters
              </Button>
            </div>
          )}
        </div>
      ) : (
        <>
          <ArtifactsTable slug={slug} artifacts={artifacts} />
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
