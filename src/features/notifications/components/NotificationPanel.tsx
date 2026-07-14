import { Bell, CheckCheck, Settings2 } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { Badge } from '../../../components/ui/Badge';
import { Spinner } from '../../../components/ui/Spinner';
import { Tabs } from '../../../components/ui/Tabs';
import type {
  Notification,
  NotificationCategory,
  NotificationSeverity,
} from '../../../types/notification';
import { useNotificationStreamContext } from '../hooks/useNotificationStream';
import {
  useBulkNotifications,
  useMarkAllRead,
  useMarkRead,
  useNotificationsList,
  useUnreadCount,
} from '../hooks/useNotifications';
import {
  CATEGORY_LABELS,
  SEVERITY_LABELS,
  resolveNotificationLink,
} from '../lib/notificationPresentation';
import { NotificationCard } from './NotificationCard';

const selectClasses =
  'min-w-0 flex-1 rounded border border-steel/20 bg-canvas px-2 py-1 text-xs text-charcoal transition-colors hover:border-steel/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary';

interface NotificationPanelProps {
  /** Close the popover/sheet (before navigating away). */
  onClose: () => void;
  /**
   * Open the preferences dialog. Owned by the bell, OUTSIDE the popover:
   * the dialog portals to document.body, so if it lived inside the panel,
   * clicking it would register as an outside click and close the popover
   * out from under it.
   */
  onOpenPreferences: () => void;
  /** Enable touch swipe read/archive on cards (the mobile bottom sheet). */
  enableSwipe?: boolean;
}

/**
 * The Notification Center stream: header (title, unread badge, mark-all,
 * settings, view-all), All/Unread tabs, and a scrollable card list. Shared
 * verbatim between the desktop popover and the mobile bottom sheet so both
 * surfaces stay identical in behavior.
 */
export function NotificationPanel({
  onClose,
  onOpenPreferences,
  enableSwipe,
}: NotificationPanelProps) {
  const { slug } = useParams<{ slug: string }>();
  const navigate = useNavigate();
  const { connected } = useNotificationStreamContext();
  const [tab, setTab] = useState<'all' | 'unread'>('all');
  // Panel-local narrowing (validated server-side); resets with the panel.
  const [category, setCategory] = useState('');
  const [severity, setSeverity] = useState('');

  const { data: unreadCount = 0 } = useUnreadCount(connected);
  const list = useNotificationsList(
    {
      unread: tab === 'unread' ? true : undefined,
      category: category || undefined,
      severity: severity || undefined,
    },
    connected,
  );
  const markRead = useMarkRead();
  const markAllRead = useMarkAllRead();
  const bulk = useBulkNotifications();

  const notifications = useMemo(
    () => (list.data?.pages ?? []).flatMap((page) => page.notifications),
    [list.data],
  );

  const openNotification = (notification: Notification) => {
    // Mark-read is fire-and-forget: navigation must never wait on it.
    if (!notification.readAt) markRead.mutate(notification.id);
    onClose();
    if (slug) navigate(resolveNotificationLink(slug, notification.link));
  };

  const viewAll = () => {
    onClose();
    if (slug) navigate(`/w/${slug}/notifications`);
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-2 px-2.5 pb-1.5 pt-2.5">
        <h2 className="text-sm font-semibold text-charcoal">Notifications</h2>
        {unreadCount > 0 && <Badge variant="primary">{unreadCount} unread</Badge>}
        <div className="ml-auto flex items-center gap-0.5">
          <button
            type="button"
            title="Mark all read"
            aria-label="Mark all notifications as read"
            disabled={unreadCount === 0 || markAllRead.isPending}
            onClick={() => markAllRead.mutate(undefined)}
            className="rounded p-1.5 text-steel transition-colors hover:bg-charcoal/5 hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:cursor-default disabled:opacity-40"
          >
            <CheckCheck size={16} aria-hidden="true" />
          </button>
          <button
            type="button"
            title="Notification settings"
            aria-label="Notification settings"
            onClick={onOpenPreferences}
            className="rounded p-1.5 text-steel transition-colors hover:bg-charcoal/5 hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          >
            <Settings2 size={16} aria-hidden="true" />
          </button>
        </div>
      </div>

      <Tabs
        className="shrink-0 px-2.5"
        ariaLabel="Notification filters"
        active={tab}
        onChange={(id) => setTab(id as 'all' | 'unread')}
        tabs={[
          { id: 'all', label: 'All' },
          {
            id: 'unread',
            label: 'Unread',
            adornment:
              unreadCount > 0 ? (
                <span className="rounded bg-primary/10 px-1 text-xs font-medium text-primary">
                  {unreadCount > 99 ? '99+' : unreadCount}
                </span>
              ) : undefined,
          },
        ]}
      />

      {/* Rapid narrowing without leaving the popover; native selects keep
          it keyboard/screen-reader accessible for free. */}
      <div className="flex shrink-0 items-center gap-1.5 px-2.5 py-1.5">
        <select
          aria-label="Filter by category"
          value={category}
          onChange={(event) => setCategory(event.target.value)}
          className={selectClasses}
        >
          <option value="">All categories</option>
          {(Object.keys(CATEGORY_LABELS) as NotificationCategory[]).map((value) => (
            <option key={value} value={value}>
              {CATEGORY_LABELS[value]}
            </option>
          ))}
        </select>
        <select
          aria-label="Filter by severity"
          value={severity}
          onChange={(event) => setSeverity(event.target.value)}
          className={selectClasses}
        >
          <option value="">All severities</option>
          {(Object.keys(SEVERITY_LABELS) as NotificationSeverity[]).map((value) => (
            <option key={value} value={value}>
              {SEVERITY_LABELS[value]}
            </option>
          ))}
        </select>
      </div>

      <div role="tabpanel" className="min-h-0 flex-1 overflow-y-auto p-1.5">
        {list.isLoading ? (
          <div className="flex items-center justify-center py-10">
            <Spinner />
          </div>
        ) : notifications.length === 0 ? (
          <div className="flex flex-col items-center gap-2 px-4 py-10 text-center">
            <Bell size={32} strokeWidth={1.25} className="text-steel" aria-hidden="true" />
            <p className="text-sm font-medium text-charcoal">
              {tab === 'unread' ? "You're all caught up" : 'No notifications yet'}
            </p>
            <p className="text-xs leading-relaxed text-steel">
              {tab === 'unread'
                ? 'New operational alerts will appear here as they happen.'
                : 'Pipeline failures, runner outages, and security changes will land here.'}
            </p>
          </div>
        ) : (
          <ul className="space-y-0.5">
            {notifications.map((notification) => (
              <li key={notification.id}>
                <NotificationCard
                  compact
                  notification={notification}
                  onSelect={openNotification}
                  onSwipeRead={
                    enableSwipe ? (n) => markRead.mutate(n.id) : undefined
                  }
                  onSwipeArchive={
                    enableSwipe
                      ? (n) => bulk.mutate({ action: 'archive', ids: [n.id] })
                      : undefined
                  }
                />
              </li>
            ))}
            {list.hasNextPage && (
              <li className="px-1.5 pt-1">
                <button
                  type="button"
                  disabled={list.isFetchingNextPage}
                  onClick={() => void list.fetchNextPage()}
                  className="w-full rounded border border-steel/20 px-2 py-1.5 text-xs text-steel transition-colors hover:bg-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                >
                  {list.isFetchingNextPage ? 'Loading…' : 'Load more'}
                </button>
              </li>
            )}
          </ul>
        )}
      </div>

      <div className="shrink-0 border-t border-steel/10 p-1.5">
        <button
          type="button"
          onClick={viewAll}
          className="w-full rounded px-2 py-1.5 text-center text-sm text-link transition-colors hover:bg-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          View all notifications
        </button>
      </div>
    </div>
  );
}
