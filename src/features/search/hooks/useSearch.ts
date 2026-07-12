import { keepPreviousData, useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import type { SearchCategory } from '../../../types/search';
import { getSearchResults } from '../api/searchApi';

export { useWorkspaceId };

/** Server-side minimum mirrored client-side: don't query on 1 character. */
export const MIN_QUERY_LENGTH = 2;

export const searchResultsKey = (
  workspaceId: string,
  q: string,
  category?: SearchCategory,
) => ['workspaces', workspaceId, 'search', { q, category: category ?? '' }] as const;

/** Flat, score-ordered keyset pagination for the dedicated search page. */
export function useSearchResults(filters: { q: string; category?: SearchCategory }) {
  const workspaceId = useWorkspaceId();
  const q = filters.q.trim();
  return useInfiniteQuery({
    queryKey: searchResultsKey(workspaceId ?? '', q, filters.category),
    queryFn: ({ pageParam }) =>
      getSearchResults(workspaceId!, {
        q,
        category: filters.category,
        cursor: pageParam || undefined,
      }),
    initialPageParam: '',
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    enabled: Boolean(workspaceId) && q.length >= MIN_QUERY_LENGTH,
  });
}

/** Grouped top-hits-per-category for the command palette (live search). */
export function usePaletteSearch(q: string, open: boolean) {
  const workspaceId = useWorkspaceId();
  const trimmed = q.trim();
  return useQuery({
    queryKey: ['workspaces', workspaceId ?? '', 'search', 'palette', trimmed] as const,
    queryFn: () => getSearchResults(workspaceId!, { q: trimmed, group: true }),
    enabled: Boolean(workspaceId) && open && trimmed.length >= MIN_QUERY_LENGTH,
    placeholderData: keepPreviousData,
    staleTime: 15_000,
  });
}
