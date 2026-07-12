/** The indexed entity types — mirrors the backend category allow-list. */
export type SearchCategory =
  | 'repository'
  | 'workflow'
  | 'pipeline'
  | 'runner'
  | 'artifact'
  | 'environment'
  | 'secret'
  | 'activity';

export const SEARCH_CATEGORIES: SearchCategory[] = [
  'repository',
  'workflow',
  'pipeline',
  'runner',
  'artifact',
  'environment',
  'secret',
  'activity',
];

/** One ranked hit. `meta` carries display-only chips written by the indexer. */
export interface SearchResult {
  id: string;
  category: SearchCategory;
  entityId: string;
  title: string;
  subtitle: string;
  meta: Record<string, unknown>;
  updatedAt: string;
  score: number;
}

export interface SearchResponse {
  results: SearchResult[];
  /** Per-category totals; null on paginated follow-up pages. */
  counts: Partial<Record<SearchCategory, number>> | null;
  nextCursor: string | null;
}
