import { Search, X } from 'lucide-react';
import type {
  NotificationCategory,
  NotificationSeverity,
} from '../../../types/notification';
import { CATEGORY_LABELS, SEVERITY_LABELS } from '../lib/notificationPresentation';

const controlClasses =
  'rounded border border-steel/20 bg-canvas px-2.5 py-1.5 text-sm text-charcoal transition-colors hover:border-steel/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary';

export interface HistoryFilters {
  q: string;
  category: string;
  severity: string;
  unread: boolean;
  archived: 'exclude' | 'include' | 'only';
}

interface NotificationFilterBarProps {
  filters: HistoryFilters;
  onChange: (next: Partial<HistoryFilters>) => void;
}

/**
 * URL-synced filter bar for the notification history page: free-text
 * search, severity/category allow-list selects, unread toggle, and the
 * archived view switch. Values feed straight into the validated REST query.
 */
export function NotificationFilterBar({ filters, onChange }: NotificationFilterBarProps) {
  const hasFilters =
    filters.q !== '' ||
    filters.category !== '' ||
    filters.severity !== '' ||
    filters.unread ||
    filters.archived !== 'exclude';

  return (
    <div className="flex flex-wrap items-center gap-2">
      <div className="relative min-w-0 flex-1 basis-56">
        <Search
          size={14}
          aria-hidden="true"
          className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
        />
        <input
          type="search"
          aria-label="Search notifications"
          placeholder="Search notifications…"
          value={filters.q}
          maxLength={200}
          onChange={(event) => onChange({ q: event.target.value })}
          className={`${controlClasses} w-full pl-8`}
        />
      </div>

      <select
        aria-label="Filter by severity"
        value={filters.severity}
        onChange={(event) => onChange({ severity: event.target.value })}
        className={controlClasses}
      >
        <option value="">All severities</option>
        {(Object.keys(SEVERITY_LABELS) as NotificationSeverity[]).map((severity) => (
          <option key={severity} value={severity}>
            {SEVERITY_LABELS[severity]}
          </option>
        ))}
      </select>

      <select
        aria-label="Filter by category"
        value={filters.category}
        onChange={(event) => onChange({ category: event.target.value })}
        className={controlClasses}
      >
        <option value="">All categories</option>
        {(Object.keys(CATEGORY_LABELS) as NotificationCategory[]).map((category) => (
          <option key={category} value={category}>
            {CATEGORY_LABELS[category]}
          </option>
        ))}
      </select>

      <select
        aria-label="Archived view"
        value={filters.archived}
        onChange={(event) =>
          onChange({ archived: event.target.value as HistoryFilters['archived'] })
        }
        className={controlClasses}
      >
        <option value="exclude">Active</option>
        <option value="include">Active + archived</option>
        <option value="only">Archived only</option>
      </select>

      <label className={`${controlClasses} flex cursor-pointer items-center gap-2`}>
        <input
          type="checkbox"
          checked={filters.unread}
          onChange={(event) => onChange({ unread: event.target.checked })}
          className="h-3.5 w-3.5 accent-primary"
        />
        Unread only
      </label>

      {hasFilters && (
        <button
          type="button"
          onClick={() =>
            onChange({ q: '', category: '', severity: '', unread: false, archived: 'exclude' })
          }
          className="inline-flex items-center gap-1 rounded px-2 py-1.5 text-sm text-steel transition-colors hover:bg-surface hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <X size={14} aria-hidden="true" />
          Clear
        </button>
      )}
    </div>
  );
}
