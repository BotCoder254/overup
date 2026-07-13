import { Search } from 'lucide-react';
import { useMe } from '../../features/auth/hooks/useAuth';
import { NotificationBell } from '../../features/notifications/components/NotificationBell';
import { Logo } from '../brand/Logo';
import { SidebarNav } from './SidebarNav';
import { UserFooter } from './UserFooter';
import { WorkspaceSwitcher } from './WorkspaceSwitcher';

interface SidebarProps {
  /** Called after a nav link is activated (closes the mobile drawer). */
  onNavigate?: () => void;
  /** Opens the command palette, optionally seeded with typed text. */
  onSearch: (seed?: string) => void;
}

/**
 * The persistent navigation frame: workspace identity + switcher fixed at the
 * top, an independently scrollable nav list in the middle, and the user
 * footer fixed at the bottom. Rendered both as the desktop rail and inside
 * the mobile drawer.
 */
export function Sidebar({ onNavigate, onSearch }: SidebarProps) {
  const { data: me } = useMe();
  if (!me?.workspace) return null; // guarded by WorkspaceRoute; satisfies types

  return (
    <div className="flex h-full flex-col">
      <div className="shrink-0 px-3 pt-3">
        <div className="flex items-center gap-1.5">
          <Logo size="sm" withWordmark={false} className="shrink-0 px-1 text-charcoal" />
          <WorkspaceSwitcher workspace={me.workspace} />
          {/* Operational inbox: right edge of the shell header, before the
              user's own controls — the "top of the app" position. */}
          <div className="shrink-0">
            <NotificationBell />
          </div>
        </div>
        {/* Real search input, centered at the top of the sidebar. Focusing
            or typing hands off to the floating palette (seeded with the
            typed text) — one search surface, so the input itself stays
            empty and the palette owns the query. */}
        <div className="relative mx-auto mt-3 w-full">
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
            className="w-full rounded border border-steel/20 bg-canvas py-1.5 pl-8 pr-14 text-sm text-charcoal transition-colors placeholder:text-steel hover:border-steel/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          />
          <kbd className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded border border-steel/20 bg-surface px-1.5 font-sans text-[11px] text-steel">
            Ctrl K
          </kbd>
        </div>
      </div>
      <nav aria-label="Workspace" className="mt-4 min-h-0 flex-1 overflow-y-auto px-3 pb-4">
        <SidebarNav slug={me.workspace.slug} onNavigate={onNavigate} />
      </nav>
      <div className="shrink-0">
        <UserFooter me={me} />
      </div>
    </div>
  );
}
