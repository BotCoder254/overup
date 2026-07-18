import { Command, Search } from 'lucide-react';
import { NotificationBell } from '../../features/notifications/components/NotificationBell';

interface TopBarProps {
  /** Opens the command palette, optionally seeded with typed text. */
  onSearch: (seed?: string) => void;
}

/**
 * Desktop-only header strip on the outer shell surface, above the floating
 * content canvas: workspace search centered, the notification bell pinned to
 * the far right. Below lg these controls live in MobileTopBar instead.
 */
export function TopBar({ onSearch }: TopBarProps) {
  return (
    <div className="hidden shrink-0 grid-cols-[1fr_minmax(0,21rem)_1fr] items-center gap-2 px-3 pt-3 lg:grid">
      {/* Left spacer keeps the search truly centered despite the bell. */}
      <div aria-hidden="true" />
      {/* Real search input, centered on the shell. Focusing or typing hands
          off to the floating palette (seeded with the typed text) — one
          search surface, so the input itself stays empty and the palette
          owns the query. */}
      <div className="relative w-full">
        <Search
          size={14}
          aria-hidden="true"
          className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
        />
        <input
          type="search"
          aria-label="Search workspace"
          placeholder="Search…"
          value=""
          onFocus={() => onSearch()}
          onChange={(event) => onSearch(event.target.value)}
          className="w-full rounded border border-steel/20 bg-canvas py-2 pl-8 pr-14 text-sm text-charcoal transition-colors placeholder:text-steel hover:border-steel/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        />
        {/* Icon-based shortcut chip (Ctrl/⌘ + K opens the palette). */}
        <kbd
          aria-label="Shortcut: Control or Command plus K"
          className="pointer-events-none absolute right-2 top-1/2 flex -translate-y-1/2 items-center gap-0.5 rounded border border-steel/20 bg-surface px-1.5 py-0.5 font-sans text-[11px] text-steel"
        >
          <Command size={11} aria-hidden="true" />
          K
        </kbd>
      </div>
      {/* Operational inbox at the far right of the outer shell; the bell's
          popover is portaled and end-aligned, so no extra wiring needed. */}
      <div className="flex justify-end">
        <NotificationBell />
      </div>
    </div>
  );
}
