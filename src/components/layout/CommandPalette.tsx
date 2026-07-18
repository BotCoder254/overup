import { Command } from 'cmdk';
import { Search } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useHotkeys } from 'react-hotkeys-hook';
import { NAV_GROUPS, workspacePath } from '../../app/navigation';
import { useMe } from '../../features/auth/hooks/useAuth';
import {
  SEARCH_CATEGORY_META,
  searchResultPath,
} from '../../features/search/categories';
import { MIN_QUERY_LENGTH, usePaletteSearch } from '../../features/search/hooks/useSearch';
import { useDebouncedValue } from '../../lib/useDebouncedValue';
import type { SearchCategory, SearchResult } from '../../types/search';
import { Spinner } from '../ui/Spinner';

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Text typed into the sidebar input before the palette took focus. */
  initialQuery?: string;
}

const GROUP_CLASSES =
  '[&_[cmdk-group-heading]]:px-2.5 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:pt-2.5 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wider [&_[cmdk-group-heading]]:text-steel';

const ITEM_CLASSES =
  'flex cursor-pointer items-center gap-2.5 rounded px-2.5 py-2 text-sm text-charcoal data-[selected=true]:bg-primary/10 data-[selected=true]:text-primary';

/**
 * Global search palette (Ctrl/Cmd+K). An empty query shows the navigation
 * jump list (the original behavior); typing runs live, debounced Global
 * Search against the server — grouped, permission-aware results with a
 * spinner while fetching — plus a pinned jump to the full results page.
 * cmdk's built-in filtering is off (`shouldFilter=false`): the server ranks,
 * the nav list is filtered manually.
 */
export function CommandPalette({ open, onOpenChange, initialQuery }: CommandPaletteProps) {
  const { data: me } = useMe();
  const navigate = useNavigate();
  const [q, setQ] = useState('');

  useHotkeys(
    'mod+k',
    () => onOpenChange(!open),
    {
      enableOnFormTags: true,
      preventDefault: true,
    },
    // Without deps the callback keeps the first render's `open` (false), so
    // the shortcut could open the palette but never toggle it closed.
    [open, onOpenChange],
  );

  // Seed (or clear) the query whenever the palette opens; typing in the
  // sidebar input while open appends through the same channel.
  useEffect(() => {
    if (open) setQ(initialQuery ?? '');
  }, [open, initialQuery]);

  const trimmed = q.trim();
  const debounced = useDebouncedValue(trimmed, 300);
  const search = usePaletteSearch(debounced, open);
  const searching = trimmed.length >= MIN_QUERY_LENGTH;

  // Group server hits by category, preserving the server's rank order.
  const grouped = useMemo(() => {
    const groups = new Map<SearchCategory, SearchResult[]>();
    for (const result of search.data?.results ?? []) {
      const bucket = groups.get(result.category);
      if (bucket) {
        bucket.push(result);
      } else {
        groups.set(result.category, [result]);
      }
    }
    return groups;
  }, [search.data]);

  const slug = me?.workspace?.slug;
  if (!slug) return null;

  const navGroups = trimmed
    ? NAV_GROUPS.map((group) => ({
        ...group,
        items: group.items.filter((item) =>
          item.label.toLowerCase().includes(trimmed.toLowerCase()),
        ),
      })).filter((group) => group.items.length > 0)
    : NAV_GROUPS;

  const close = () => onOpenChange(false);

  return (
    <Command.Dialog
      open={open}
      onOpenChange={onOpenChange}
      label="Search"
      shouldFilter={false}
      overlayClassName="fixed inset-0 z-50 bg-navy/40 animate-fade-in"
      contentClassName="fixed left-1/2 top-24 z-50 w-[calc(100%-2rem)] max-w-md -translate-x-1/2 rounded border border-steel/20 bg-canvas p-2 animate-scale-in"
    >
      <div className="relative">
        <Command.Input
          value={q}
          onValueChange={setQ}
          placeholder="Search the workspace…"
          className="w-full border-b border-steel/20 bg-transparent px-2.5 pb-2.5 pr-8 pt-1 text-sm text-charcoal placeholder:text-steel focus:outline-none"
        />
        {searching && search.isFetching && (
          <Spinner className="absolute right-2 top-1 h-4 w-4 text-steel" />
        )}
      </div>
      <Command.List className="max-h-72 overflow-y-auto pt-1">
        <Command.Empty className="px-2.5 py-6 text-center text-sm text-steel">
          {searching && search.isFetching ? 'Searching…' : 'No results found.'}
        </Command.Empty>

        {searching && (
          <>
            {Array.from(grouped.entries()).map(([category, results]) => {
              const meta = SEARCH_CATEGORY_META[category];
              return (
                <Command.Group key={category} heading={meta.plural} className={GROUP_CLASSES}>
                  {results.map((result) => (
                    <Command.Item
                      key={result.id}
                      value={`${category}:${result.id}`}
                      onSelect={() => {
                        navigate(searchResultPath(slug, result));
                        close();
                      }}
                      className={ITEM_CLASSES}
                    >
                      <meta.icon size={16} strokeWidth={2} aria-hidden="true" className="shrink-0" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate">{result.title}</span>
                        {result.subtitle && (
                          <span className="block truncate text-xs text-steel">
                            {result.subtitle}
                          </span>
                        )}
                      </span>
                    </Command.Item>
                  ))}
                </Command.Group>
              );
            })}
            <Command.Group heading="More" className={GROUP_CLASSES}>
              <Command.Item
                value={`see-all:${trimmed}`}
                onSelect={() => {
                  navigate(
                    `${workspacePath(slug, 'search')}?q=${encodeURIComponent(trimmed)}`,
                  );
                  close();
                }}
                className={ITEM_CLASSES}
              >
                <Search size={16} strokeWidth={2} aria-hidden="true" className="shrink-0" />
                See all results for “{trimmed}”
              </Command.Item>
            </Command.Group>
          </>
        )}

        {navGroups.map((group) => (
          <Command.Group key={group.label} heading={group.label} className={GROUP_CLASSES}>
            {group.items.map((item) => (
              <Command.Item
                key={item.segment || 'dashboard'}
                value={`nav:${item.label}`}
                onSelect={() => {
                  navigate(workspacePath(slug, item.segment));
                  close();
                }}
                className={ITEM_CLASSES}
              >
                <item.icon size={16} strokeWidth={2} aria-hidden="true" className="shrink-0" />
                {item.label}
              </Command.Item>
            ))}
          </Command.Group>
        ))}
      </Command.List>
    </Command.Dialog>
  );
}
