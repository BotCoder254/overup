import {
  LogOut,
  MonitorSmartphone,
  SlidersHorizontal,
  User as UserIcon,
} from 'lucide-react';
import { useLogout } from '../../features/auth/hooks/useAuth';
import type { Me } from '../../types/user';
import { MenuItem, Popover } from '../ui/Popover';
import { Spinner } from '../ui/Spinner';
import { cn } from '../../lib/cn';

interface UserFooterProps {
  me: Me;
}

/**
 * Fixed sidebar footer: the profile region opens a contextual menu, while a
 * standalone logout icon keeps sign-out a single click away. The avatar uses
 * the 6px square treatment (sidebar identity), not the circular one.
 */
export function UserFooter({ me }: UserFooterProps) {
  const logout = useLogout();
  const name = me.displayName ?? me.username;

  return (
    <div className="flex items-center gap-1 border-t border-steel/20 p-3">
      <Popover
        ariaLabel="Account menu"
        side="top"
        align="start"
        panelClassName="w-56"
        renderTrigger={(triggerProps, isOpen) => (
          <button
            type="button"
            {...triggerProps}
            className={cn(
              'flex min-w-0 flex-1 items-center gap-2.5 rounded p-1.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
              isOpen ? 'bg-charcoal/5' : 'hover:bg-charcoal/5',
            )}
          >
            {me.avatarUrl ? (
              <img
                src={me.avatarUrl}
                alt=""
                referrerPolicy="no-referrer"
                className="h-8 w-8 shrink-0 rounded object-cover"
              />
            ) : (
              <span
                aria-hidden="true"
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded bg-primary/10 text-sm font-semibold text-primary"
              >
                {name.charAt(0).toUpperCase()}
              </span>
            )}
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-medium text-charcoal">{name}</span>
              <span className="block truncate text-xs text-steel">@{me.username}</span>
            </span>
          </button>
        )}
      >
        {({ close }) => (
          <>
            <div className="mb-1 border-b border-steel/20 px-2.5 py-2">
              <div className="truncate text-sm font-medium text-charcoal">{name}</div>
              {me.email && <div className="truncate text-xs text-steel">{me.email}</div>}
            </div>
            <MenuItem icon={UserIcon} disabled hint="Soon">
              Profile
            </MenuItem>
            <MenuItem icon={SlidersHorizontal} disabled hint="Soon">
              Preferences
            </MenuItem>
            <MenuItem icon={MonitorSmartphone} disabled hint="Soon">
              Sessions
            </MenuItem>
            <div className="my-1 border-t border-steel/20" aria-hidden="true" />
            <MenuItem
              icon={LogOut}
              destructive
              onSelect={() => {
                close();
                logout.mutate();
              }}
            >
              Sign out
            </MenuItem>
          </>
        )}
      </Popover>
      <button
        type="button"
        aria-label="Sign out"
        title="Sign out"
        disabled={logout.isPending}
        onClick={() => logout.mutate()}
        className="shrink-0 rounded p-2 text-steel transition-colors hover:bg-danger/10 hover:text-danger focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:pointer-events-none"
      >
        {logout.isPending ? (
          <Spinner className="h-4 w-4" />
        ) : (
          <LogOut size={16} aria-hidden="true" />
        )}
      </button>
    </div>
  );
}
