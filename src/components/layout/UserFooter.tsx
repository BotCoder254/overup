import {
  LogOut,
  MonitorSmartphone,
  SlidersHorizontal,
  User as UserIcon,
} from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { useLogout } from '../../features/auth/hooks/useAuth';
import type { Me } from '../../types/user';
import { MenuItem, Popover } from '../ui/Popover';
import { cn } from '../../lib/cn';

interface UserFooterProps {
  me: Me;
}

/**
 * Fixed sidebar footer: the profile region opens a contextual menu that owns
 * every account action, including sign-out. Deliberately seamless — no
 * separator against the nav above. The avatar uses the 6px square treatment
 * (sidebar identity), not the circular one. Profile / Preferences / Sessions
 * are shortcuts into the Settings sub-tabs.
 */
export function UserFooter({ me }: UserFooterProps) {
  const logout = useLogout();
  const navigate = useNavigate();
  const name = me.displayName ?? me.username;
  const slug = me.workspace?.slug;

  const goToSettings = (close: () => void, tab: 'profile' | 'authentication' | 'workspace') => {
    close();
    if (slug) navigate(`/w/${slug}/settings?tab=${tab}`);
  };

  return (
    <div className="flex items-center p-3">
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
            <MenuItem
              icon={UserIcon}
              disabled={!slug}
              onSelect={() => goToSettings(close, 'profile')}
            >
              Profile
            </MenuItem>
            <MenuItem
              icon={SlidersHorizontal}
              disabled={!slug}
              onSelect={() => goToSettings(close, 'workspace')}
            >
              Preferences
            </MenuItem>
            <MenuItem
              icon={MonitorSmartphone}
              disabled={!slug}
              onSelect={() => goToSettings(close, 'authentication')}
            >
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
    </div>
  );
}
