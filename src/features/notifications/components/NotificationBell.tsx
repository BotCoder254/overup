import { Bell } from 'lucide-react';
import { useState } from 'react';
import { Popover } from '../../../components/ui/Popover';
import { cn } from '../../../lib/cn';
import { useIsDesktop } from '../../../lib/useMediaQuery';
import { useNotificationStreamContext } from '../hooks/useNotificationStream';
import { useUnreadCount } from '../hooks/useNotifications';
import { NotificationPanel } from './NotificationPanel';
import { NotificationSheet } from './NotificationSheet';
import { PreferencesDialog } from './PreferencesDialog';

/** Badge copy caps at 99+ so the bell never stretches the layout. */
function badgeLabel(count: number): string {
  return count > 99 ? '99+' : String(count);
}

interface BellButtonProps {
  unreadCount: number;
  isOpen?: boolean;
}

function bellClasses(isOpen?: boolean): string {
  return cn(
    'relative rounded p-2 text-steel transition-colors hover:bg-charcoal/5 hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
    isOpen && 'bg-charcoal/5 text-charcoal',
  );
}

function BellGlyph({ unreadCount }: BellButtonProps) {
  return (
    <>
      <Bell size={18} aria-hidden="true" />
      {unreadCount > 0 && (
        // Count pill: rounded-full is the sanctioned exception (avatars,
        // spinners, count dots) to the strict 6px radius rule.
        <span
          aria-hidden="true"
          className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[10px] font-semibold leading-none text-white"
        >
          {badgeLabel(unreadCount)}
        </span>
      )}
    </>
  );
}

/**
 * The Notification Center entry point, rendered in the sidebar header
 * (desktop) and the mobile top bar. Desktop opens a right-aligned floating
 * popover under the bell; below `lg` it opens a full-height bottom sheet —
 * identical content, touch-optimized container. Never navigates away.
 */
export function NotificationBell() {
  const isDesktop = useIsDesktop();
  const { connected } = useNotificationStreamContext();
  const { data: unreadCount = 0 } = useUnreadCount(connected);
  const [sheetOpen, setSheetOpen] = useState(false);
  // Owned here, OUTSIDE the popover: the dialog portals to document.body,
  // so inside the panel every dialog click would read as an outside click
  // and close the popover (unmounting the dialog with it).
  const [preferencesOpen, setPreferencesOpen] = useState(false);

  const ariaLabel =
    unreadCount > 0
      ? `Notifications (${badgeLabel(unreadCount)} unread)`
      : 'Notifications';

  if (!isDesktop) {
    return (
      <>
        <button
          type="button"
          aria-label={ariaLabel}
          aria-haspopup="dialog"
          aria-expanded={sheetOpen}
          onClick={() => setSheetOpen(true)}
          className={bellClasses(sheetOpen)}
        >
          <BellGlyph unreadCount={unreadCount} />
        </button>
        <NotificationSheet open={sheetOpen} onClose={() => setSheetOpen(false)}>
          <NotificationPanel
            enableSwipe
            onClose={() => setSheetOpen(false)}
            onOpenPreferences={() => {
              setSheetOpen(false);
              setPreferencesOpen(true);
            }}
          />
        </NotificationSheet>
        <PreferencesDialog open={preferencesOpen} onClose={() => setPreferencesOpen(false)} />
      </>
    );
  }

  return (
    <>
      <Popover
        role="dialog"
        align="end"
        ariaLabel="Notifications"
        panelClassName="flex w-96 max-w-[calc(100vw-16px)] flex-col p-0 shadow-lg"
        renderTrigger={(triggerProps, isOpen) => (
          <button
            type="button"
            aria-label={ariaLabel}
            {...triggerProps}
            className={bellClasses(isOpen)}
          >
            <BellGlyph unreadCount={unreadCount} />
          </button>
        )}
      >
        {({ close }) => (
          <div className="flex max-h-[min(70vh,560px)] min-h-0 flex-col">
            <NotificationPanel
              onClose={close}
              onOpenPreferences={() => {
                close();
                setPreferencesOpen(true);
              }}
            />
          </div>
        )}
      </Popover>
      <PreferencesDialog open={preferencesOpen} onClose={() => setPreferencesOpen(false)} />
    </>
  );
}
