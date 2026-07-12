import { formatDistanceToNow } from 'date-fns';
import { Link } from 'react-router-dom';
import { Badge } from '../../../components/ui/Badge';
import type { SearchResult } from '../../../types/search';
import { SEARCH_CATEGORY_META, searchResultPath } from '../categories';

/**
 * One ranked hit: category icon, title, subtitle, category badge, and a
 * couple of display chips off the indexed meta. All fields render as React
 * text nodes — never markup.
 */
export function SearchResultRow({ slug, result }: { slug: string; result: SearchResult }) {
  const meta = SEARCH_CATEGORY_META[result.category];
  const Icon = meta.icon;
  const status = typeof result.meta.status === 'string' ? result.meta.status : null;
  const kind = typeof result.meta.kind === 'string' ? result.meta.kind : null;

  return (
    <li>
      <Link
        to={searchResultPath(slug, result)}
        className="flex items-center gap-3 rounded px-3 py-2.5 transition-colors hover:bg-charcoal/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
      >
        <Icon size={16} aria-hidden="true" className="shrink-0 text-steel" />
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span className="truncate text-sm font-medium text-charcoal">{result.title}</span>
            {status && (
              <Badge variant="outline" className="shrink-0">
                {status}
              </Badge>
            )}
            {kind && kind !== status && (
              <Badge variant="outline" className="hidden shrink-0 sm:inline-flex">
                {kind}
              </Badge>
            )}
          </span>
          {result.subtitle && (
            <span className="block truncate text-xs text-steel">{result.subtitle}</span>
          )}
        </span>
        <span className="hidden shrink-0 items-center gap-2 sm:flex">
          <Badge variant="neutral">{meta.label}</Badge>
          <span className="text-xs text-steel">
            {formatDistanceToNow(new Date(result.updatedAt), { addSuffix: true })}
          </span>
        </span>
      </Link>
    </li>
  );
}
