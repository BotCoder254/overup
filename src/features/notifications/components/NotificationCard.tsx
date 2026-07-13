import { Archive, MailOpen } from 'lucide-react';
import { useRef, useState, type TouchEvent } from 'react';
import { formatDistanceToNow } from 'date-fns';
import { cn } from '../../../lib/cn';
import type { Notification } from '../../../types/notification';
import {
  categoryIcon,
  severityAccentClass,
  severityTextClass,
} from '../lib/notificationPresentation';

/** Horizontal travel (px) that commits a swipe action on release. */
const SWIPE_COMMIT_PX = 56;
/** Cards never translate past this, so the reveal stays composed. */
const SWIPE_MAX_PX = 96;

interface NotificationCardProps {
  notification: Notification;
  /** Activate (navigate + background mark-read); owned by the parent. */
  onSelect: (notification: Notification) => void;
  /** Denser spacing for the popover/sheet stream. */
  compact?: boolean;
  /**
   * Touch-only swipe actions (the mobile bottom sheet): swipe right marks
   * read, swipe left archives. Purely additive — buttons and keyboard
   * remain the primary, accessible path.
   */
  onSwipeRead?: (notification: Notification) => void;
  onSwipeArchive?: (notification: Notification) => void;
}

/**
 * One notification row: category icon, severity accent, server-rendered
 * title/body, relative timestamp, unread dot, and a ×N badge when dedup
 * merged repeated occurrences into this row.
 */
export function NotificationCard({
  notification,
  onSelect,
  compact,
  onSwipeRead,
  onSwipeArchive,
}: NotificationCardProps) {
  const Icon = categoryIcon(notification.category);
  const unread = !notification.readAt;
  const swipeable = Boolean(onSwipeRead || onSwipeArchive);

  const [dx, setDx] = useState(0);
  const [settling, setSettling] = useState(false);
  const touchStart = useRef<{ x: number; y: number } | null>(null);
  const horizontal = useRef(false);

  const onTouchStart = (event: TouchEvent) => {
    if (!swipeable) return;
    const touch = event.touches[0];
    touchStart.current = { x: touch.clientX, y: touch.clientY };
    horizontal.current = false;
    setSettling(false);
  };

  const onTouchMove = (event: TouchEvent) => {
    if (!swipeable || !touchStart.current) return;
    const touch = event.touches[0];
    const deltaX = touch.clientX - touchStart.current.x;
    const deltaY = touch.clientY - touchStart.current.y;
    // Horizontal intent: clearly sideways, not a scroll.
    if (!horizontal.current) {
      if (Math.abs(deltaX) > 12 && Math.abs(deltaX) > 2 * Math.abs(deltaY)) {
        horizontal.current = true;
      } else if (Math.abs(deltaY) > 12) {
        touchStart.current = null; // it's a scroll — stand down
        return;
      }
    }
    if (!horizontal.current) return;
    // Only offer directions that have an action (and reading a read row
    // is a no-op, so don't tease it).
    let next = Math.max(-SWIPE_MAX_PX, Math.min(SWIPE_MAX_PX, deltaX));
    if (next > 0 && (!onSwipeRead || !unread)) next = 0;
    if (next < 0 && (!onSwipeArchive || notification.archivedAt)) next = 0;
    setDx(next);
  };

  const onTouchEnd = () => {
    if (!swipeable) return;
    const committed = dx;
    touchStart.current = null;
    horizontal.current = false;
    setSettling(true);
    setDx(0);
    if (committed >= SWIPE_COMMIT_PX && onSwipeRead && unread) {
      onSwipeRead(notification);
    } else if (committed <= -SWIPE_COMMIT_PX && onSwipeArchive && !notification.archivedAt) {
      onSwipeArchive(notification);
    }
  };

  const card = (
    <button
      type="button"
      onClick={() => onSelect(notification)}
      onTouchStart={onTouchStart}
      onTouchMove={onTouchMove}
      onTouchEnd={onTouchEnd}
      onTouchCancel={onTouchEnd}
      style={swipeable ? { transform: `translateX(${dx}px)` } : undefined}
      className={cn(
        'relative flex w-full items-start gap-2.5 rounded border-l-2 text-left transition-colors',
        'hover:bg-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
        severityAccentClass(notification.severity),
        compact ? 'px-2.5 py-2' : 'px-3 py-2.5',
        unread ? 'bg-primary/[0.03]' : 'bg-canvas',
        swipeable && settling && 'transition-transform duration-200',
      )}
    >
      <Icon
        size={16}
        aria-hidden="true"
        className={cn('mt-0.5 shrink-0', severityTextClass(notification.severity))}
      />
      <span className="min-w-0 flex-1">
        <span className="flex items-start gap-1.5">
          <span
            className={cn(
              'min-w-0 flex-1 break-words text-sm leading-snug',
              unread ? 'font-medium text-charcoal' : 'text-charcoal/80',
            )}
          >
            {notification.title}
          </span>
          {notification.occurrenceCount > 1 && (
            <span className="shrink-0 rounded bg-surface px-1 text-xs font-medium text-steel">
              ×{notification.occurrenceCount}
            </span>
          )}
          {unread && (
            <span
              className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-primary"
              aria-label="Unread"
            />
          )}
        </span>
        {notification.body && (
          <span className="mt-0.5 block break-words text-xs leading-relaxed text-steel">
            {notification.body}
          </span>
        )}
        <span className="mt-1 block text-xs text-steel/80">
          {formatDistanceToNow(new Date(notification.createdAt), { addSuffix: true })}
          {notification.archivedAt && ' · archived'}
        </span>
      </span>
    </button>
  );

  if (!swipeable) return card;

  // Swipe reveal: static-palette hints behind the translating card.
  return (
    <div className="relative overflow-hidden rounded">
      <div aria-hidden="true" className="absolute inset-0 flex items-center justify-between">
        <span
          className={cn(
            'flex items-center gap-1.5 pl-3 text-xs font-medium text-primary transition-opacity',
            dx > 12 ? 'opacity-100' : 'opacity-0',
          )}
        >
          <MailOpen size={14} />
          Read
        </span>
        <span
          className={cn(
            'flex items-center gap-1.5 pr-3 text-xs font-medium text-danger transition-opacity',
            dx < -12 ? 'opacity-100' : 'opacity-0',
          )}
        >
          Archive
          <Archive size={14} />
        </span>
      </div>
      {card}
    </div>
  );
}
