import { useEffect, useRef, useState } from 'react';
import { Outlet, useLocation } from 'react-router-dom';
import { NotificationStreamProvider } from '../../features/notifications/hooks/useNotificationStream';
import { cn } from '../../lib/cn';
import { AnnouncementBanner } from './AnnouncementBanner';
import { CommandPalette } from './CommandPalette';
import { MobileTopBar } from './MobileTopBar';
import { Sidebar } from './Sidebar';
import { TopBar } from './TopBar';

/**
 * The persistent application shell, mounted once as a layout route for the
 * whole authenticated workspace: a fixed navigation sidebar on the left and a
 * floating, independently scrolling content canvas on the right. Below lg the
 * sidebar becomes an overlay drawer behind a hamburger in the mobile top bar.
 */
export function AppShell() {
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  // Text typed into the sidebar search input before the palette took focus;
  // seeds the palette's query so no keystroke is lost.
  const [paletteSeed, setPaletteSeed] = useState('');
  const drawerRef = useRef<HTMLDivElement>(null);
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const location = useLocation();

  // Route changes swap only the canvas content; make sure the drawer is gone.
  useEffect(() => {
    setDrawerOpen(false);
  }, [location.pathname]);

  const openSearch = (seed?: string) => {
    if (seed !== undefined) setPaletteSeed(seed);
    setPaletteOpen(true);
  };

  // Move focus into the drawer when it opens; back to the hamburger on close.
  useEffect(() => {
    if (drawerOpen) {
      drawerRef.current
        ?.querySelector<HTMLElement>('button, [href], input, [tabindex]:not([tabindex="-1"])')
        ?.focus();
    } else if (drawerRef.current?.contains(document.activeElement)) {
      menuButtonRef.current?.focus();
    }
  }, [drawerOpen]);

  return (
    // The notification socket lives at the shell level so the bell is live
    // on every page; the bell and the history page share its `connected`.
    <NotificationStreamProvider>
    <div className="flex h-dvh flex-col overflow-hidden bg-surface text-charcoal">
      {/* Full-width system announcement, above the shell on the surface band. */}
      <AnnouncementBanner />

      <div className="flex min-h-0 flex-1">
      {/* Desktop sidebar */}
      <div className="hidden w-[272px] shrink-0 lg:block">
        <Sidebar />
      </div>

      {/* Mobile drawer + backdrop */}
      <div
        className={cn('fixed inset-0 z-40 lg:hidden', !drawerOpen && 'pointer-events-none')}
      >
        <div
          aria-hidden="true"
          onClick={() => setDrawerOpen(false)}
          className={cn(
            'absolute inset-0 bg-navy/40 transition-opacity duration-200',
            drawerOpen ? 'opacity-100' : 'opacity-0',
          )}
        />
        <div
          ref={drawerRef}
          role="dialog"
          aria-modal="true"
          aria-label="Navigation"
          inert={!drawerOpen}
          onKeyDown={(event) => {
            if (event.key === 'Escape') setDrawerOpen(false);
          }}
          className={cn(
            'absolute inset-y-0 left-0 w-[272px] bg-surface transition-transform duration-200 ease-out',
            drawerOpen ? 'translate-x-0' : '-translate-x-full',
          )}
        >
          <Sidebar onNavigate={() => setDrawerOpen(false)} />
        </div>
      </div>

      {/* Content column: top bar (desktop) / mobile top bar + floating canvas */}
      <div className="flex min-w-0 flex-1 flex-col">
        <MobileTopBar
          ref={menuButtonRef}
          drawerOpen={drawerOpen}
          onMenu={() => setDrawerOpen(true)}
          onSearch={() => openSearch()}
        />
        {/* Desktop header strip on the exposed outer shell: centered search,
            bell far right. A shrink-0 sibling of the canvas, so the canvas
            gives up exactly its height and stays the only scroll region. */}
        <TopBar onSearch={openSearch} />
        <div className="flex min-h-0 flex-1 flex-col p-2 pt-0 lg:p-3 lg:pl-0 lg:pt-3">
          <main className="min-h-0 flex-1 overflow-y-auto rounded border border-steel/20 bg-canvas">
            <div className="mx-auto max-w-6xl p-4 sm:p-6 lg:p-8">
              <Outlet />
            </div>
          </main>
        </div>
      </div>
      </div>

      <CommandPalette
        open={paletteOpen}
        initialQuery={paletteSeed}
        onOpenChange={(open) => {
          setPaletteOpen(open);
          if (!open) setPaletteSeed('');
        }}
      />
    </div>
    </NotificationStreamProvider>
  );
}
