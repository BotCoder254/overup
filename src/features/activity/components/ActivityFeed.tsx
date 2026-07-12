import { format, isToday, isYesterday } from 'date-fns';
import { useMemo } from 'react';
import type { ActivityEvent } from '../../../types/activity';
import { ActivityEventRow } from './ActivityEventRow';

function dayLabel(date: Date): string {
  if (isToday(date)) return 'Today';
  if (isYesterday(date)) return 'Yesterday';
  return format(date, 'PPP');
}

interface DayGroup {
  key: string;
  label: string;
  events: ActivityEvent[];
}

interface ActivityFeedProps {
  events: ActivityEvent[];
  slug: string;
  /** Forwarded to each row: clicking an actor narrows the feed to them. */
  onSelectActor?: (actorId: string, login: string | null) => void;
}

/**
 * The chronological timeline: ledger entries grouped under sticky day
 * headings (Today / Yesterday / date), newest first. Grouping is purely
 * presentational — ordering and pagination come from the keyset query.
 */
export function ActivityFeed({ events, slug, onSelectActor }: ActivityFeedProps) {
  const groups = useMemo<DayGroup[]>(() => {
    const byDay: DayGroup[] = [];
    for (const event of events) {
      const date = new Date(event.createdAt);
      const key = format(date, 'yyyy-MM-dd');
      const last = byDay[byDay.length - 1];
      if (last && last.key === key) {
        last.events.push(event);
      } else {
        byDay.push({ key, label: dayLabel(date), events: [event] });
      }
    }
    return byDay;
  }, [events]);

  return (
    <div className="rounded border border-steel/20 bg-canvas">
      {groups.map((group) => (
        <section key={group.key} aria-label={group.label}>
          <h3 className="sticky top-0 z-10 border-b border-steel/10 bg-surface px-4 py-1.5 text-[11px] font-medium uppercase tracking-wider text-steel">
            {group.label}
          </h3>
          <ul className="divide-y divide-steel/10 px-4">
            {group.events.map((event) => (
              <ActivityEventRow
                key={event.id}
                event={event}
                slug={slug}
                onSelectActor={onSelectActor}
              />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
