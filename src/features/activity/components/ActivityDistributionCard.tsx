import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { cn } from '../../../lib/cn';
import type { ActivityCategory, ActivitySummary } from '../../../types/activity';
import { CATEGORY_LABELS, CATEGORY_ORDER } from '../lib/eventPresentation';

interface ActivityDistributionCardProps {
  summary: ActivitySummary | undefined;
  loading: boolean;
  activeCategory: string;
  onSelectCategory: (category: string) => void;
}

/**
 * Event distribution by category over the summary window. Each row doubles
 * as a filter toggle for the timeline — clicking a category narrows the
 * feed, clicking it again clears it.
 */
export function ActivityDistributionCard({
  summary,
  loading,
  activeCategory,
  onSelectCategory,
}: ActivityDistributionCardProps) {
  const counts = summary?.byCategory;
  const max = counts ? Math.max(1, ...CATEGORY_ORDER.map((c) => counts[c] ?? 0)) : 1;

  return (
    <Card>
      <CardHeader>
        <h2 className="text-sm font-semibold text-charcoal">Event distribution</h2>
      </CardHeader>
      <CardBody>
        {loading || !counts ? (
          <p className="py-2 text-sm text-steel">Loading distribution…</p>
        ) : (
          <ul className="space-y-1">
            {CATEGORY_ORDER.map((category: ActivityCategory) => {
              const count = counts[category] ?? 0;
              const active = activeCategory === category;
              return (
                <li key={category}>
                  <button
                    type="button"
                    onClick={() => onSelectCategory(active ? '' : category)}
                    aria-pressed={active}
                    className={cn(
                      'w-full rounded px-2 py-1.5 text-left transition-colors',
                      'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
                      active ? 'bg-primary/10' : 'hover:bg-surface',
                    )}
                  >
                    <span className="flex items-center justify-between gap-2 text-xs">
                      <span className={cn('font-medium', active ? 'text-primary' : 'text-charcoal')}>
                        {CATEGORY_LABELS[category]}
                      </span>
                      <span className="tabular-nums text-steel">{count}</span>
                    </span>
                    <span className="mt-1 block h-1 w-full overflow-hidden rounded bg-steel/10">
                      <span
                        className="block h-full rounded bg-primary"
                        style={{ width: `${Math.round((count / max) * 100)}%` }}
                      />
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        <p className="mt-3 text-xs text-steel">
          Last {summary?.windowDays ?? 30} days · click a category to filter the timeline.
        </p>
      </CardBody>
    </Card>
  );
}
