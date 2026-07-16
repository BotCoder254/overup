import { KeyRound, Plus } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import type { SecretsCatalogFilters as ApiFilters } from '../api/secretsApi';
import { DetectedRequirementsCard } from '../components/DetectedRequirementsCard';
import { SecretAuditList } from '../components/SecretAuditList';
import {
  EMPTY_SECRET_FILTERS,
  SecretFilters,
  type SecretFilterState,
} from '../components/SecretFilters';
import { SecretFormDialog } from '../components/SecretFormDialog';
import { SecretsSecurityCard } from '../components/SecretsSecurityCard';
import { SecretsSummaryStrip } from '../components/SecretsSummaryStrip';
import { SecretsTable } from '../components/SecretsTable';
import {
  useSecretsAudit,
  useSecretsCatalog,
  useSecretsRequirements,
  useSecretsSummary,
} from '../hooks/useSecrets';

/** Trailing-edge debounce for the free-text input. */
function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const handle = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(handle);
  }, [value, delayMs]);
  return debounced;
}

function filtersFromParams(params: URLSearchParams): SecretFilterState {
  return {
    q: params.get('q') ?? '',
    scope: params.get('scope') ?? '',
    repositoryId: params.get('repo') ?? '',
    environmentId: params.get('env') ?? '',
  };
}

/**
 * The workspace secrets catalog: encrypted, write-only credentials injected
 * into pipeline jobs at dispatch. The wider column lists secrets (filters
 * live in the URL so views are shareable, keyset-paginated with
 * sentinel-driven infinite scroll); the narrower right column carries the
 * security posture and recent audit activity. Values never appear anywhere.
 */
export function SecretsPage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const [searchParams, setSearchParams] = useSearchParams();
  const [filters, setFilters] = useState<SecretFilterState>(() =>
    filtersFromParams(searchParams),
  );
  const [createOpen, setCreateOpen] = useState(false);
  // Seeded by a detected-requirement "Add" click; cleared on dialog close.
  const [preset, setPreset] = useState<{
    name: string;
    repository?: { id: string; name: string };
  } | null>(null);

  // Mirror the filters into the URL (replace — no history spam).
  useEffect(() => {
    const next = new URLSearchParams();
    if (filters.q) next.set('q', filters.q);
    if (filters.scope) next.set('scope', filters.scope);
    if (filters.repositoryId) next.set('repo', filters.repositoryId);
    if (filters.environmentId) next.set('env', filters.environmentId);
    setSearchParams(next, { replace: true });
  }, [filters, setSearchParams]);

  const debouncedQ = useDebouncedValue(filters.q, 300);
  const apiFilters = useMemo<ApiFilters>(
    () => ({
      q: debouncedQ.trim() || undefined,
      scope: filters.scope || undefined,
      repositoryId: filters.repositoryId || undefined,
      environmentId: filters.environmentId || undefined,
    }),
    [filters, debouncedQ],
  );

  const summary = useSecretsSummary();
  const audit = useSecretsAudit();
  const requirements = useSecretsRequirements();
  const query = useSecretsCatalog(apiFilters);
  const secrets = (query.data?.pages ?? []).flatMap((page) => page.secrets);
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
        title="Secrets"
        description="Encrypted credentials the control plane injects into pipeline jobs at execution time — values are write-only and masked in every log."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <Plus size={14} aria-hidden="true" />
            New secret
          </Button>
        }
      />

      <SecretFormDialog
        open={createOpen}
        onClose={() => {
          setCreateOpen(false);
          setPreset(null);
        }}
        presetName={preset?.name}
        presetRepository={preset?.repository}
      />

      <SecretsSummaryStrip
        summary={summary.data}
        loading={summary.isLoading}
        error={summary.isError}
      />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
        <div className="min-w-0">
          <SecretFilters
            value={filters}
            onChange={(patch) => setFilters((current) => ({ ...current, ...patch }))}
          />

          {query.isLoading ? (
            <div className="flex min-h-[40vh] items-center justify-center">
              <Spinner className="h-6 w-6 text-steel" />
            </div>
          ) : secrets.length === 0 ? (
            <div>
              <EmptyState
                icon={KeyRound}
                title={hasFilters ? 'No matching secrets' : 'No secrets yet'}
                description={
                  hasFilters
                    ? 'Nothing matches the current filters. Clear them to see every secret.'
                    : 'Store deployment tokens, registry credentials, and API keys here — encrypted at rest, injected into jobs at run time, masked in logs.'
                }
                action={
                  !hasFilters ? (
                    <Button size="sm" onClick={() => setCreateOpen(true)}>
                      <Plus size={14} aria-hidden="true" />
                      New secret
                    </Button>
                  ) : undefined
                }
              />
              {hasFilters && (
                <div className="mt-3 flex justify-center">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => setFilters(EMPTY_SECRET_FILTERS)}
                  >
                    Clear filters
                  </Button>
                </div>
              )}
            </div>
          ) : (
            <>
              <SecretsTable slug={slug} secrets={secrets} />
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
          <DetectedRequirementsCard
            requirements={requirements.data}
            onAdd={(name, repository) => {
              setPreset({ name, repository });
              setCreateOpen(true);
            }}
          />
          <SecretsSecurityCard
            encryptionConfigured={summary.data?.encryptionConfigured}
            stale={summary.data?.stale}
            staleAfterDays={summary.data?.staleAfterDays}
          />
          <Card>
            <CardHeader>
              <h2 className="text-sm font-semibold text-charcoal">Recent activity</h2>
            </CardHeader>
            <CardBody>
              <SecretAuditList events={audit.data?.events ?? []} loading={audit.isLoading} />
            </CardBody>
          </Card>
        </div>
      </div>
    </>
  );
}
