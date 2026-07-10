import { Menu, Search } from 'lucide-react';
import { forwardRef } from 'react';
import { useMe } from '../../features/auth/hooks/useAuth';
import { Logo } from '../brand/Logo';

interface MobileTopBarProps {
  drawerOpen: boolean;
  onMenu: () => void;
  onSearch: () => void;
}

/**
 * Compact top bar shown below the lg breakpoint, where the sidebar becomes an
 * overlay drawer. Forwards a ref to the hamburger so the shell can restore
 * focus when the drawer closes.
 */
export const MobileTopBar = forwardRef<HTMLButtonElement, MobileTopBarProps>(
  function MobileTopBar({ drawerOpen, onMenu, onSearch }, menuButtonRef) {
    const { data: me } = useMe();

    return (
      <div className="flex items-center gap-2 px-3 py-2.5 lg:hidden">
        <button
          ref={menuButtonRef}
          type="button"
          aria-label="Open navigation"
          aria-expanded={drawerOpen}
          onClick={onMenu}
          className="rounded p-2 text-charcoal transition-colors hover:bg-charcoal/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <Menu size={20} aria-hidden="true" />
        </button>
        <Logo size="sm" withWordmark={false} className="shrink-0 text-charcoal" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {me?.workspace?.name}
        </span>
        <button
          type="button"
          aria-label="Search"
          onClick={onSearch}
          className="rounded p-2 text-steel transition-colors hover:bg-charcoal/5 hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <Search size={18} aria-hidden="true" />
        </button>
      </div>
    );
  },
);
