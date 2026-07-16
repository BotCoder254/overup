import { Boxes, Layers, Lock, Plus, Search, Workflow } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import type { EnvironmentsCatalogFilters as ApiFilters } from '../api/environmentsApi';
import { DetectedEnvironmentsCard } from '../components/DetectedEnvironmentsCard';
import { EnvironmentAuditList } from '../components/EnvironmentAuditList';
import { EnvironmentFormDialog } from '../components/EnvironmentFormDialog';
import { EnvironmentsSummaryStrip } from '../components/EnvironmentsSummaryStrip';
import { EnvironmentsTable } from '../components/EnvironmentsTable';
import {
  useEnvironmentsAudit,
  useEnvironmentsCatalog,
  useEnvironmentsRequirements,
  useEnvironmentsSummary,
} from '../hooks/useEnvironments';

/** Trailing-edge debounce for the free-text input. */
function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const handle = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(handle);
  }, [value, delayMs]);
  return debounced;
}

function HowRow({
  icon: Icon,
  title,
  detail,
}: {
  icon: LucideIcon;
  title: string;
  detail: string;
}) {
  return (
    <li className="flex items-start gap-2.5 py-2">
      <Icon size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-primary" />
      <span className="min-w-0">
        <span className="block text-sm font-medium text-charcoal">{title}</span>
        <span className="block text-xs text-steel">{detail}</span>
      </span>
    </li>
  );
}

/**
 * The workspace environments catalog: named deployment targets that act as
 * the highest-precedence secrets scope. Same layout language as the Secrets
 * page — URL-synced search, keyset infinite scroll, posture column.
 */
export function EnvironmentsPage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const [searchParams, setSearchParams] = useSearchParams();
  const [q, setQ] = useState(() => searchParams.get('q') ?? '');
  const [createOpen, setCreateOpen] = useState(false);
  // Seeded by a detected-environment "Create" click; cleared on close.
  const [presetName, setPresetName] = useState<string | null>(null);

  // Mirror the filter into the URL (replace — no history spam).
  useEffect(() => {
    const next = new URLSearchParams();
    if (q) next.set('q', q);
    setSearchParams(next, { replace: true });
  }, [q, setSearchParams]);

  const debouncedQ = useDebouncedValue(q, 300);
  const apiFilters = useMemo<ApiFilters>(
    () => ({ q: debouncedQ.trim() || undefined }),
    [debouncedQ],
  );

  const summary = useEnvironmentsSummary();
  const audit = useEnvironmentsAudit();
  const requirements = useEnvironmentsRequirements();
  const query = useEnvironmentsCatalog(apiFilters);
  const environments = (query.data?.pages ?? []).flatMap((page) => page.environments);

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
        title="Environments"
        description="Named deployment targets referenced from workflow YAML — each carries its own secrets, injected with the highest precedence."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <Plus size={14} aria-hidden="true" />
            New environment
          </Button>
        }
      />

      <EnvironmentFormDialog
        open={createOpen}
        onClose={() => {
          setCreateOpen(false);
          setPresetName(null);
        }}
        presetName={presetName}
      />

      <EnvironmentsSummaryStrip
        summary={summary.data}
        loading={summary.isLoading}
        error={summary.isError}
      />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
        <div className="min-w-0">
          <div className="mb-4 flex flex-wrap items-center gap-2">
            <label className="sr-only" htmlFor="environment-search">
              Search environments
            </label>
            <div className="relative w-full sm:w-64">
              <Search
                size={14}
                aria-hidden="true"
                className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
              />
              <input
                id="environment-search"
                type="search"
                placeholder="Search name or description…"
                maxLength={200}
                className="h-9 w-full rounded border border-steel/30 bg-canvas px-2 pl-8 text-sm text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                value={q}
                onChange={(event) => setQ(event.target.value)}
              />
            </div>
          </div>

          {query.isLoading ? (
            <div className="flex min-h-[40vh] items-center justify-center">
              <Spinner className="h-6 w-6 text-steel" />
            </div>
          ) : environments.length === 0 ? (
            <div>
              <EmptyState
                icon={Boxes}
                title={q ? 'No matching environments' : 'No environments yet'}
                description={
                  q
                    ? 'Nothing matches the current search. Clear it to see every environment.'
                    : 'Create environments like production or staging, give each its own secrets, and reference them from workflow YAML with `environment: <name>`.'
                }
                action={
                  !q ? (
                    <Button size="sm" onClick={() => setCreateOpen(true)}>
                      <Plus size={14} aria-hidden="true" />
                      New environment
                    </Button>
                  ) : undefined
                }
              />
              {q && (
                <div className="mt-3 flex justify-center">
                  <Button variant="secondary" size="sm" onClick={() => setQ('')}>
                    Clear search
                  </Button>
                </div>
              )}
            </div>
          ) : (
            <>
              <EnvironmentsTable slug={slug} environments={environments} />
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
          <DetectedEnvironmentsCard
            requirements={requirements.data}
            onCreate={(name) => {
              setPresetName(name);
              setCreateOpen(true);
            }}
          />
          <Card>
            <CardHeader>
              <h2 className="text-sm font-semibold text-charcoal">How environments work</h2>
            </CardHeader>
            <CardBody>
              <ul className="divide-y divide-steel/10">
                <HowRow
                  icon={Workflow}
                  title="Bound by name in YAML"
                  detail="A job declares `environment: production`; the name resolves case-insensitively at dispatch time, so reruns always use the current definition."
                />
                <HowRow
                  icon={Lock}
                  title="Highest secret precedence"
                  detail="An environment secret shadows repository and workspace secrets of the same name — only for jobs that reference the environment."
                />
                <HowRow
                  icon={Layers}
                  title="Unknown names never fail a run"
                  detail="A job referencing an undefined environment still executes; it just runs without environment secrets, with a notice recorded on its plan."
                />
              </ul>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>
              <h2 className="text-sm font-semibold text-charcoal">Recent activity</h2>
            </CardHeader>
            <CardBody>
              <EnvironmentAuditList events={audit.data?.events ?? []} loading={audit.isLoading} />
            </CardBody>
          </Card>
        </div>
      </div>
    </>
  );
}
