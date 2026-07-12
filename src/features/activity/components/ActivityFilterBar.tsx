import { Search, UserRound, X } from 'lucide-react';
import { Button } from '../../../components/ui/Button';
import { ACTION_GROUPS, CATEGORY_LABELS, CATEGORY_ORDER } from '../lib/eventPresentation';

export interface ActivityFilterState {
  q: string;
  category: string;
  action: string;
  /** Actor narrowing is applied by clicking an actor in the feed — the id
   *  filters, the login is carried alongside purely for chip display. */
  actorId: string;
  actorLogin: string;
  from: string;
  to: string;
}

export const EMPTY_ACTIVITY_FILTERS: ActivityFilterState = {
  q: '',
  category: '',
  action: '',
  actorId: '',
  actorLogin: '',
  from: '',
  to: '',
};

const controlClasses =
  'h-9 rounded border border-steel/30 bg-canvas px-2 text-sm text-charcoal ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary';

interface ActivityFilterBarProps {
  value: ActivityFilterState;
  onChange: (patch: Partial<ActivityFilterState>) => void;
}

/**
 * URL-synced feed filters (the catalog pattern): free-text search over
 * actions, metadata, and actors; category and exact-action allow-lists;
 * and a date range. Everything is re-validated server-side.
 */
export function ActivityFilterBar({ value, onChange }: ActivityFilterBarProps) {
  const dirty = Object.values(value).some(Boolean);
  const visibleGroups = value.category
    ? ACTION_GROUPS.filter((group) => group.category === value.category)
    : ACTION_GROUPS;

  return (
    <div className="mb-4 flex flex-wrap items-center gap-2">
      <div className="relative w-full sm:w-64">
        <Search
          size={14}
          aria-hidden="true"
          className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
        />
        <input
          type="search"
          value={value.q}
          onChange={(event) => onChange({ q: event.target.value })}
          placeholder="Search events, actors, metadata…"
          aria-label="Search activity"
          className={`${controlClasses} w-full pl-8`}
        />
      </div>

      <select
        value={value.category}
        onChange={(event) =>
          // A category switch invalidates any exact-action pick from
          // another category, so clear it.
          onChange({ category: event.target.value, action: '' })
        }
        aria-label="Filter by category"
        className={`${controlClasses} w-full sm:w-auto`}
      >
        <option value="">All categories</option>
        {CATEGORY_ORDER.map((category) => (
          <option key={category} value={category}>
            {CATEGORY_LABELS[category]}
          </option>
        ))}
      </select>

      <select
        value={value.action}
        onChange={(event) => onChange({ action: event.target.value })}
        aria-label="Filter by action"
        className={`${controlClasses} w-full sm:w-auto`}
      >
        <option value="">All actions</option>
        {visibleGroups.map((group) => (
          <optgroup key={group.category} label={CATEGORY_LABELS[group.category]}>
            {group.actions.map((action) => (
              <option key={action} value={action}>
                {action}
              </option>
            ))}
          </optgroup>
        ))}
      </select>

      <input
        type="date"
        value={value.from}
        onChange={(event) => onChange({ from: event.target.value })}
        aria-label="From date"
        className={`${controlClasses} w-full sm:w-auto`}
      />
      <input
        type="date"
        value={value.to}
        onChange={(event) => onChange({ to: event.target.value })}
        aria-label="To date"
        className={`${controlClasses} w-full sm:w-auto`}
      />

      {value.actorId && (
        // Dismissible actor chip — set by clicking an actor in the feed
        // (no member dropdown; the feed itself is the picker).
        <button
          type="button"
          onClick={() => onChange({ actorId: '', actorLogin: '' })}
          aria-label={`Stop filtering by ${value.actorLogin || 'this actor'}`}
          className={`${controlClasses} inline-flex w-full items-center gap-1.5 sm:w-auto`}
        >
          <UserRound size={14} aria-hidden="true" className="text-steel" />
          <span className="truncate font-medium">{value.actorLogin || 'Actor'}</span>
          <X size={12} aria-hidden="true" className="text-steel" />
        </button>
      )}

      {dirty && (
        <Button variant="secondary" size="sm" onClick={() => onChange(EMPTY_ACTIVITY_FILTERS)}>
          <X size={14} aria-hidden="true" />
          Clear
        </Button>
      )}
    </div>
  );
}
