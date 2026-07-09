import { LogOut } from 'lucide-react';
import type { ReactNode } from 'react';
import { Logo } from '../brand/Logo';
import { Button } from '../ui/Button';
import { useLogout, useMe } from '../../features/auth/hooks/useAuth';

interface AppShellProps {
  children: ReactNode;
}

/** Minimal authenticated chrome: top bar with brand, identity, sign-out. */
export function AppShell({ children }: AppShellProps) {
  const { data: me } = useMe();
  const logout = useLogout();

  return (
    <div className="flex min-h-screen flex-col bg-surface">
      <header className="flex items-center justify-between gap-2 border-b border-steel/20 bg-canvas px-4 py-3 sm:px-6">
        <Logo size="sm" className="text-charcoal" />
        <div className="flex items-center gap-2 sm:gap-4">
          <div className="flex items-center gap-2.5">
            {me?.avatarUrl && (
              <img
                src={me.avatarUrl}
                alt=""
                className="h-8 w-8 rounded-full"
                referrerPolicy="no-referrer"
              />
            )}
            <span className="hidden text-sm font-medium sm:inline">
              {me?.displayName ?? me?.username}
            </span>
          </div>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => logout.mutate()}
            isLoading={logout.isPending}
          >
            {!logout.isPending && <LogOut className="h-4 w-4" aria-hidden="true" />}
            Sign out
          </Button>
        </div>
      </header>
      <main className="flex-1 p-4 sm:p-6">{children}</main>
    </div>
  );
}
