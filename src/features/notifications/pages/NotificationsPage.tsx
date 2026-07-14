import { Archive, BellOff, CheckCheck, Download, MailOpen, Settings2 } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { toast } from 'sonner';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import { useDebouncedValue } from '../../../lib/useDebouncedValue';
import type { Notification } from '../../../types/notification';
import { exportNotifications } from '../api/notificationsApi';
import { NotificationCard } from '../components/NotificationCard';
import {
  NotificationFilterBar,
  type HistoryFilters,
} from '../components/NotificationFilterBar';
import { PreferencesDialog } from '../components/PreferencesDialog';
import { useNotificationStreamContext } from '../hooks/useNotificationStream';
import {
  useBulkNotifications,
  useMarkAllRead,
  useMarkRead,
  useNotificationsList,
  useUnreadCount,
  useWorkspaceId,
} from '../hooks/useNotifications';
import { resolveNotificationLink } from '../lib/notificationPresentation';

function filtersFromParams(params: URLSearchParams): HistoryFilters {
  const archived = params.get('archived');
  return {
    q: params.get('q') ?? '',
    category: params.get('category') ?? '',
    severity: params.get('severity') ?? '',
    unread: params.get('unread') === 'true',
    archived: archived === 'include' || archived === 'only' ? archived : 'exclude',
  };
}

/**
 * Notification history: the long-term, searchable view of the operational
 * inbox — URL-synced filters, keyset infinite scroll, and bulk read/unread/
 * archive over checkbox selections. The popover handles the "right now"
 * flow; this page handles "what happened last week".
 */
export function NotificationsPage() {
  const { slug } = useParams<{ slug: string }>();
  const navigate = useNavigate();
  const { connected } = useNotificationStreamContext();
  const [searchParams, setSearchParams] = useSearchParams();
  const filters = useMemo(() => filtersFromParams(searchParams), [searchParams]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [preferencesOpen, setPreferencesOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const workspaceId = useWorkspaceId();

  const debouncedQ = useDebouncedValue(filters.q, 300);
  const queryFilters = useMemo(
    () => ({
      q: debouncedQ || undefined,
      category: filters.category || undefined,
      severity: filters.severity || undefined,
      unread: filters.unread || undefined,
      archived: filters.archived === 'exclude' ? undefined : filters.archived,
    }),
    [debouncedQ, filters.category, filters.severity, filters.unread, filters.archived],
  );

  const list = useNotificationsList(queryFilters, connected);
  const { data: unreadCount = 0 } = useUnreadCount(connected);
  const markRead = useMarkRead();
  const markAllRead = useMarkAllRead();
  const bulk = useBulkNotifications();

  const notifications = useMemo(
    () => (list.data?.pages ?? []).flatMap((page) => page.notifications),
    [list.data],
  );

  // Filter changes reset the selection — ids may no longer be visible.
  useEffect(() => {
    setSelected(new Set());
  }, [debouncedQ, filters.category, filters.severity, filters.unread, filters.archived]);

  const updateFilters = (next: Partial<HistoryFilters>) => {
    const merged = { ...filters, ...next };
    const params = new URLSearchParams();
    if (merged.q) params.set('q', merged.q);
    if (merged.category) params.set('category', merged.category);
    if (merged.severity) params.set('severity', merged.severity);
    if (merged.unread) params.set('unread', 'true');
    if (merged.archived !== 'exclude') params.set('archived', merged.archived);
    setSearchParams(params, { replace: true });
  };

  // Infinite scroll: fetch the next page as the sentinel approaches.
  const sentinelRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (!sentinel) return undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && list.hasNextPage && !list.isFetchingNextPage) {
          void list.fetchNextPage();
        }
      },
      { rootMargin: '200px' },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [list]);

  const toggleSelected = (id: string) => {
    setSelected((old) => {
      const next = new Set(old);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const allVisibleSelected =
    notifications.length > 0 && notifications.every((n) => selected.has(n.id));

  const toggleSelectAll = () => {
    setSelected(allVisibleSelected ? new Set() : new Set(notifications.map((n) => n.id)));
  };

  const runBulk = (action: 'read' | 'unread' | 'archive') => {
    // The API caps bulk payloads at 100 ids; chunk larger selections.
    const ids = Array.from(selected);
    for (let start = 0; start < ids.length; start += 100) {
      bulk.mutate({ action, ids: ids.slice(start, start + 100) });
    }
    setSelected(new Set());
  };

  const openNotification = (notification: Notification) => {
    if (!notification.readAt) markRead.mutate(notification.id);
    if (slug) navigate(resolveNotificationLink(slug, notification.link));
  };

  // Server re-validates every filter and hardens the CSV (formula-injection
  // neutralization + row cap) — the blob is download-ready.
  const handleExport = async () => {
    if (!workspaceId || exporting) return;
    setExporting(true);
    try {
      const blob = await exportNotifications(workspaceId, queryFilters);
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = 'notifications-export.csv';
      anchor.click();
      URL.revokeObjectURL(url);
    } catch {
      toast.error('Export failed — try again.');
    } finally {
      setExporting(false);
    }
  };

  const hasFilters =
    Boolean(debouncedQ) ||
    Boolean(filters.category) ||
    Boolean(filters.severity) ||
    filters.unread ||
    filters.archived !== 'exclude';

  return (
    <div className="animate-fade-in">
      <PageHeader
        title="Notifications"
        description="Your operational inbox: pipeline failures, runner outages, security changes, and reminders — the Activity Feed keeps the complete audit history."
        actions={
          <>
            <Button variant="ghost" size="sm" isLoading={exporting} onClick={handleExport}>
              <Download size={15} aria-hidden="true" />
              Export CSV
            </Button>
            <Button
              variant="secondary"
              size="sm"
              disabled={unreadCount === 0 || markAllRead.isPending}
              onClick={() => markAllRead.mutate(undefined)}
            >
              <CheckCheck size={15} aria-hidden="true" />
              Mark all read
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setPreferencesOpen(true)}>
              <Settings2 size={15} aria-hidden="true" />
              Settings
            </Button>
          </>
        }
      />

      <div className="space-y-4">
        <NotificationFilterBar filters={filters} onChange={updateFilters} />

        {/* Bulk action bar appears with a selection. */}
        {selected.size > 0 && (
          <div className="flex flex-wrap items-center gap-2 rounded border border-steel/20 bg-surface px-3 py-2">
            <span className="text-sm text-charcoal">
              {selected.size} selected
            </span>
            <div className="ml-auto flex items-center gap-1.5">
              <Button variant="ghost" size="sm" onClick={() => runBulk('read')}>
                <MailOpen size={14} aria-hidden="true" />
                Mark read
              </Button>
              <Button variant="ghost" size="sm" onClick={() => runBulk('unread')}>
                Mark unread
              </Button>
              <Button variant="secondary" size="sm" onClick={() => runBulk('archive')}>
                <Archive size={14} aria-hidden="true" />
                Archive
              </Button>
            </div>
          </div>
        )}

        {list.isLoading ? (
          <div className="flex items-center justify-center py-16">
            <Spinner />
          </div>
        ) : notifications.length === 0 ? (
          <EmptyState
            icon={BellOff}
            title={hasFilters ? 'Nothing matches these filters' : 'No notifications yet'}
            description={
              hasFilters
                ? 'Try widening the severity or category filters, or clearing the search.'
                : 'Operational alerts — failed pipelines, lost runners, security changes — will collect here as your workspace runs.'
            }
          />
        ) : (
          <div className="rounded border border-steel/20 bg-canvas">
            <div className="flex items-center gap-2.5 border-b border-steel/10 px-3 py-2">
              <input
                type="checkbox"
                aria-label={allVisibleSelected ? 'Deselect all' : 'Select all'}
                checked={allVisibleSelected}
                onChange={toggleSelectAll}
                className="h-3.5 w-3.5 accent-primary"
              />
              <span className="text-xs uppercase tracking-wider text-steel">
                {notifications.length} loaded
              </span>
            </div>
            <ul className="divide-y divide-steel/10">
              {notifications.map((notification) => (
                <li key={notification.id} className="flex items-start gap-1 px-1.5 py-0.5">
                  <input
                    type="checkbox"
                    aria-label={`Select "${notification.title}"`}
                    checked={selected.has(notification.id)}
                    onChange={() => toggleSelected(notification.id)}
                    className="mt-3.5 ml-1.5 h-3.5 w-3.5 shrink-0 accent-primary"
                  />
                  <div className="min-w-0 flex-1">
                    <NotificationCard notification={notification} onSelect={openNotification} />
                  </div>
                </li>
              ))}
            </ul>
            <div ref={sentinelRef} aria-hidden="true" />
            {list.hasNextPage && (
              <div className="border-t border-steel/10 p-2">
                <Button
                  variant="ghost"
                  size="sm"
                  className="w-full"
                  isLoading={list.isFetchingNextPage}
                  onClick={() => void list.fetchNextPage()}
                >
                  Load more
                </Button>
              </div>
            )}
          </div>
        )}
      </div>

      <PreferencesDialog open={preferencesOpen} onClose={() => setPreferencesOpen(false)} />
    </div>
  );
}
