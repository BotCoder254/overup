import { Search } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import { cn } from '../../../lib/cn';
import { useDebouncedValue } from '../../../lib/useDebouncedValue';
import { SEARCH_CATEGORIES, type SearchCategory } from '../../../types/search';
import { SEARCH_CATEGORY_META } from '../categories';
import { SearchResultRow } from '../components/SearchResultRow';
import { MIN_QUERY_LENGTH, useSearchResults } from '../hooks/useSearch';

function isCategory(raw: string | null): raw is SearchCategory {
  return raw !== null && (SEARCH_CATEGORIES as string[]).includes(raw);
}

/**
 * The dedicated Global Search page: URL-synced query + category filter over
 * the ranked full-text index, with category tabs from the first page's
 * counts and keyset infinite scroll — the environments-catalog layout
 * language.
 */
export function SearchPage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const [searchParams, setSearchParams] = useSearchParams();
  const [q, setQ] = useState(() => searchParams.get('q') ?? '');
  const [category, setCategory] = useState<SearchCategory | undefined>(() => {
    const raw = searchParams.get('category');
    return isCategory(raw) ? raw : undefined;
  });

  // Mirror the filters into the URL (replace — no history spam).
  useEffect(() => {
    const next = new URLSearchParams();
    if (q) next.set('q', q);
    if (category) next.set('category', category);
    setSearchParams(next, { replace: true });
  }, [q, category, setSearchParams]);

  const debouncedQ = useDebouncedValue(q, 300);
  const trimmedQ = debouncedQ.trim();
  const filters = useMemo(() => ({ q: trimmedQ, category }), [trimmedQ, category]);

  const query = useSearchResults(filters);
  const pages = query.data?.pages ?? [];
  const results = pages.flatMap((page) => page.results);
  // Counts arrive on the first page only and ignore the category filter, so
  // the tabs stay stable while switching between them.
  const counts = pages[0]?.counts ?? null;
  const total = counts ? Object.values(counts).reduce((sum, n) => sum + (n ?? 0), 0) : 0;

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

  const tooShort = trimmedQ.length < MIN_QUERY_LENGTH;
  const tabClasses = (active: boolean) =>
    cn(
      'inline-flex shrink-0 items-center gap-1.5 rounded border px-2.5 py-1 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
      active
        ? 'border-primary/30 bg-primary/10 font-medium text-primary'
        : 'border-steel/20 bg-canvas text-charcoal/80 hover:bg-charcoal/5',
    );

  return (
    <>
      <PageHeader
        title="Search"
        description="Everything in the workspace — repositories, workflows, pipelines, runners, artifacts, environments, secrets metadata, and activity — ranked in one place."
      />

      <div className="mb-4">
        <label className="sr-only" htmlFor="global-search">
          Search workspace
        </label>
        <div className="relative w-full sm:max-w-xl">
          <Search
            size={14}
            aria-hidden="true"
            className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
          />
          <input
            id="global-search"
            type="search"
            placeholder="Search the workspace…"
            maxLength={200}
            autoFocus
            className="h-9 w-full rounded border border-steel/30 bg-canvas px-2 pl-8 pr-8 text-sm text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            value={q}
            onChange={(event) => setQ(event.target.value)}
          />
          {query.isFetching && (
            <Spinner className="absolute right-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-steel" />
          )}
        </div>
      </div>

      {counts && (
        <div className="mb-4 flex flex-wrap items-center gap-2">
          <button type="button" className={tabClasses(!category)} onClick={() => setCategory(undefined)}>
            All
            <span className="text-xs text-steel">{total}</span>
          </button>
          {SEARCH_CATEGORIES.filter((c) => (counts[c] ?? 0) > 0).map((c) => {
            const meta = SEARCH_CATEGORY_META[c];
            return (
              <button
                key={c}
                type="button"
                className={tabClasses(category === c)}
                onClick={() => setCategory(category === c ? undefined : c)}
              >
                {meta.plural}
                <span className="text-xs text-steel">{counts[c]}</span>
              </button>
            );
          })}
        </div>
      )}

      {tooShort ? (
        <EmptyState
          icon={Search}
          title="Search the workspace"
          description={`Type at least ${MIN_QUERY_LENGTH} characters to search repositories, workflows, pipelines, runners, artifacts, environments, secrets metadata, and activity.`}
        />
      ) : query.isLoading ? (
        <div className="flex min-h-[40vh] items-center justify-center">
          <Spinner className="h-6 w-6 text-steel" />
        </div>
      ) : results.length === 0 ? (
        <div>
          <EmptyState
            icon={Search}
            title="No results"
            description={`Nothing matches “${trimmedQ}”${category ? ` in ${SEARCH_CATEGORY_META[category].plural.toLowerCase()}` : ''}. Try fewer or shorter terms — matching is prefix-based.`}
          />
          {category && (
            <div className="mt-3 flex justify-center">
              <Button variant="secondary" size="sm" onClick={() => setCategory(undefined)}>
                Search all categories
              </Button>
            </div>
          )}
        </div>
      ) : (
        <>
          <ul className="divide-y divide-steel/10 rounded border border-steel/20 bg-canvas">
            {results.map((result) => (
              <SearchResultRow key={result.id} slug={slug} result={result} />
            ))}
          </ul>
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
      )}
    </>
  );
}
