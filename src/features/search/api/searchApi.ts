import { api } from '../../../lib/api';
import type { SearchCategory, SearchResponse } from '../../../types/search';

export interface SearchFilters {
  q: string;
  category?: SearchCategory;
  cursor?: string;
  limit?: number;
  /** Palette mode: top hits per category, no pagination. */
  group?: boolean;
}

export async function getSearchResults(
  workspaceId: string,
  filters: SearchFilters,
): Promise<SearchResponse> {
  const params = new URLSearchParams();
  params.set('q', filters.q);
  if (filters.category) params.set('category', filters.category);
  if (filters.cursor) params.set('cursor', filters.cursor);
  if (filters.limit) params.set('limit', String(filters.limit));
  if (filters.group) params.set('group', 'true');
  return api
    .get(`/api/workspaces/${workspaceId}/search`, { searchParams: params })
    .json<SearchResponse>();
}
